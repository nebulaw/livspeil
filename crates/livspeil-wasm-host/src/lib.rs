// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Sandboxed WebAssembly node packages (milestone M0.15, decision D13).
//!
//! Third-party nodes run in process only as WebAssembly. A *package* is a
//! core wasm module that follows the Livspeil node ABI (version 1; logical
//! interface in `wit/livspeil-node.wit`, byte encoding in [`abi`], authoring
//! guide in `docs/wasm-nodes.md`). It exports `memory`,
//! `livspeil_describe`, `livspeil_alloc` and `livspeil_cook`, and may not
//! import anything: no WASI, no clock, no file system, no network.
//!
//! [`WasmHost`] compiles a package once (cached by SHA-256 of its bytes),
//! calls `describe` and validates the node types against the Livspeil
//! rules (namespaced ids outside `core.*`, known port types, typed
//! defaults). [`WasmPackage::nodes`] then exposes every node type as an
//! `Arc<dyn NodeImpl>` for a [`NodeRegistry`].
//!
//! # Sandbox
//!
//! Every call (`describe` and each cook) runs in a **fresh instance** with
//! its own store, so a guest cannot keep state between cooks: cooking is a
//! pure function of the inputs (and of the context, for time-dependent
//! nodes; others see a zero context). Each call gets a deterministic
//! **fuel** budget (about one unit per wasm instruction), a **memory**
//! limit and a wasm stack limit ([`WasmLimits`]). Float NaNs are
//! canonicalized, and threads, relaxed SIMD and the other
//! non-deterministic or unneeded proposals are off. Traps, fuel exhaustion, the memory limit and
//! ABI violations become [`NodeError::Failed`]; they never panic the host.

pub mod abi;
pub mod validate;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use livspeil_node_api::{
    CookContext, NodeError, NodeImpl, NodeRegistry, NodeSpec, PortSpec, RegistryError, Value,
};
use sha2::{Digest, Sha256};
use wasmtime::{
    Config, Engine, ExternType, Instance, InstancePre, Linker, Memory, Module, ResourceLimiter,
    Store, Trap,
};

use crate::abi::CookResult;
use crate::validate::{DescribeError, DescribedNode, DescribedPort};

/// Export names of the ABI.
pub const EXPORT_MEMORY: &str = "memory";
pub const EXPORT_DESCRIBE: &str = "livspeil_describe";
pub const EXPORT_ALLOC: &str = "livspeil_alloc";
pub const EXPORT_COOK: &str = "livspeil_cook";

/// Longest string a guest may return in a cook result, in bytes.
pub const MAX_RESULT_STRING: usize = 1 << 20;

/// Resource limits applied to every call into a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmLimits {
    /// Fuel per call (instantiation included); roughly wasm instructions.
    pub fuel: u64,
    /// Most bytes of linear memory an instance may have.
    pub memory_bytes: usize,
    /// Most bytes of native stack wasm code may use.
    pub max_stack_bytes: usize,
}

impl Default for WasmLimits {
    /// 10 million fuel (a few milliseconds), 64 MiB memory, 512 KiB stack.
    fn default() -> Self {
        WasmLimits {
            fuel: 10_000_000,
            memory_bytes: 64 << 20,
            max_stack_bytes: 512 << 10,
        }
    }
}

/// Why a package could not be loaded.
#[derive(Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// The file could not be read.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The host engine could not be configured.
    Engine(String),
    /// The bytes are not a valid wasm module (or WAT text).
    Compile(String),
    /// The module imports something; packages may not import anything.
    Import { module: String, name: String },
    /// A required export is missing or has the wrong type.
    Export { name: &'static str, reason: String },
    /// Instantiating or calling `describe` failed.
    Describe(String),
    /// `describe` returned a malformed buffer.
    Abi(String),
    /// A node type breaks a Livspeil rule.
    Invalid { node: String, reason: String },
    /// Registering a node type failed (for example a duplicate id).
    Registry(RegistryError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            LoadError::Engine(msg) => write!(f, "cannot configure the wasm engine: {msg}"),
            LoadError::Compile(msg) => write!(f, "not a valid wasm module: {msg}"),
            LoadError::Import { module, name } => write!(
                f,
                "imports are not allowed (found '{module}' '{name}'); packages must be self-contained"
            ),
            LoadError::Export { name, reason } => write!(f, "export '{name}': {reason}"),
            LoadError::Describe(msg) => write!(f, "describe failed: {msg}"),
            LoadError::Abi(msg) => write!(f, "describe broke the ABI: {msg}"),
            LoadError::Invalid { node, reason } => write!(f, "node type '{node}': {reason}"),
            LoadError::Registry(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io { source, .. } => Some(source),
            LoadError::Registry(err) => Some(err),
            _ => None,
        }
    }
}

/// Compiles, caches and validates packages. Share one per process.
pub struct WasmHost {
    engine: Engine,
    limits: WasmLimits,
    cache: Mutex<HashMap<[u8; 32], Arc<PackageInner>>>,
}

impl fmt::Debug for WasmHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasmHost")
            .field("limits", &self.limits)
            .field("cached", &self.cached())
            .finish_non_exhaustive()
    }
}

impl WasmHost {
    /// A host with the default limits.
    ///
    /// # Errors
    /// Returns [`LoadError::Engine`] if the engine cannot be created.
    pub fn new() -> Result<Self, LoadError> {
        Self::with_limits(WasmLimits::default())
    }

    /// A host with the given limits.
    ///
    /// # Errors
    /// Returns [`LoadError::Engine`] if the engine cannot be created.
    pub fn with_limits(limits: WasmLimits) -> Result<Self, LoadError> {
        let mut config = Config::new();
        config
            .consume_fuel(true)
            .max_wasm_stack(limits.max_stack_bytes)
            // Determinism: one NaN bit pattern on every platform, and no
            // proposals whose results may differ between machines
            // (threads and GC are compiled out).
            .cranelift_nan_canonicalization(true)
            .wasm_relaxed_simd(false)
            .wasm_memory64(false)
            .wasm_multi_memory(false)
            // Exceptions need the `gc` feature, which is compiled out.
            .wasm_wide_arithmetic(false)
            .wasm_custom_page_sizes(false);
        let engine = Engine::new(&config).map_err(|e| LoadError::Engine(format!("{e:#}")))?;
        Ok(WasmHost {
            engine,
            limits,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// The limits applied to every call.
    #[must_use]
    pub fn limits(&self) -> WasmLimits {
        self.limits
    }

    /// Number of distinct packages compiled so far.
    #[must_use]
    pub fn cached(&self) -> usize {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Load a package from a file (binary wasm or WAT text).
    ///
    /// # Errors
    /// See [`LoadError`].
    pub fn load_file(&self, path: &Path) -> Result<WasmPackage, LoadError> {
        let bytes = std::fs::read(path).map_err(|source| LoadError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        self.load_bytes(&bytes)
    }

    /// Load a package from bytes (binary wasm or WAT text). The same bytes
    /// are compiled and described only once per host.
    ///
    /// # Errors
    /// See [`LoadError`].
    pub fn load_bytes(&self, bytes: &[u8]) -> Result<WasmPackage, LoadError> {
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        if let Some(inner) = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&hash)
        {
            return Ok(WasmPackage {
                inner: Arc::clone(inner),
            });
        }
        let inner = Arc::new(self.compile(bytes, hash)?);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = Arc::clone(cache.entry(hash).or_insert(inner));
        Ok(WasmPackage { inner })
    }

    fn compile(&self, bytes: &[u8], hash: [u8; 32]) -> Result<PackageInner, LoadError> {
        let module =
            Module::new(&self.engine, bytes).map_err(|e| LoadError::Compile(format!("{e:#}")))?;
        if let Some(import) = module.imports().next() {
            return Err(LoadError::Import {
                module: import.module().to_owned(),
                name: import.name().to_owned(),
            });
        }
        check_exports(&module)?;
        let pre = Linker::new(&self.engine)
            .instantiate_pre(&module)
            .map_err(|e| LoadError::Describe(format!("{e:#}")))?;
        let runner = Runner {
            pre,
            limits: self.limits,
        };
        let buffer = runner
            .call(|store, instance| {
                let describe = instance
                    .get_typed_func::<(), i64>(&mut *store, EXPORT_DESCRIBE)
                    .map_err(|e| CallError::Abi(format!("{e:#}")))?;
                let packed = describe.call(&mut *store, ()).map_err(CallError::Wasm)?;
                read_packed(store, instance, packed)
            })
            .map_err(|err| match err {
                CallError::Abi(msg) => LoadError::Abi(msg),
                other => LoadError::Describe(other.describe(&self.limits)),
            })?;
        let described = validate::describe(&buffer).map_err(|err| match err {
            DescribeError::Decode(msg) => LoadError::Abi(msg),
            DescribeError::Invalid { node, reason } => LoadError::Invalid { node, reason },
        })?;
        let specs = described.iter().map(leak_spec).collect();
        Ok(PackageInner {
            runner,
            specs,
            hash,
        })
    }
}

/// Check that the ABI exports exist with the right kinds and signatures.
fn check_exports(module: &Module) -> Result<(), LoadError> {
    use wasmtime::ValType::{I32, I64};
    let func = |name: &'static str, params: &[wasmtime::ValType], results: &[wasmtime::ValType]| {
        let missing = || LoadError::Export {
            name,
            reason: "missing".to_owned(),
        };
        let ty = module.get_export(name).ok_or_else(missing)?;
        let ExternType::Func(ty) = ty else {
            return Err(LoadError::Export {
                name,
                reason: "not a function".to_owned(),
            });
        };
        let same = |a: &mut dyn ExactSizeIterator<Item = wasmtime::ValType>,
                    b: &[wasmtime::ValType]| {
            a.len() == b.len() && a.zip(b).all(|(x, y)| wasmtime::ValType::eq(&x, y))
        };
        if same(&mut ty.params(), params) && same(&mut ty.results(), results) {
            Ok(())
        } else {
            Err(LoadError::Export {
                name,
                reason: format!("wrong signature {ty}"),
            })
        }
    };
    match module.get_export(EXPORT_MEMORY) {
        Some(ExternType::Memory(ty)) if !ty.is_64() && !ty.is_shared() => {}
        Some(_) => {
            return Err(LoadError::Export {
                name: EXPORT_MEMORY,
                reason: "must be an unshared 32-bit memory".to_owned(),
            });
        }
        None => {
            return Err(LoadError::Export {
                name: EXPORT_MEMORY,
                reason: "missing".to_owned(),
            });
        }
    }
    func(EXPORT_DESCRIBE, &[], &[I64])?;
    func(EXPORT_ALLOC, &[I32], &[I32])?;
    func(EXPORT_COOK, &[I32, I32, I32], &[I64])?;
    Ok(())
}

/// Turn a described node into a [`NodeSpec`].
///
/// `NodeSpec` holds `&'static str`s, so the strings are leaked. That
/// happens once per distinct package per host (see the cache), and the
/// sizes are bounded by [`validate`]'s limits.
fn leak_spec(node: &DescribedNode) -> NodeSpec {
    fn leak(s: &str) -> &'static str {
        Box::leak(s.to_owned().into_boxed_str())
    }
    let ports = |ports: &[DescribedPort]| {
        ports
            .iter()
            .map(|p| PortSpec {
                name: leak(&p.name),
                ty: p.ty,
                default: p.default.clone(),
            })
            .collect()
    };
    let mut spec = NodeSpec::new(leak(&node.id), node.version, leak(&node.summary));
    spec.time_dependent = node.time_dependent;
    spec.inputs = ports(&node.inputs);
    spec.outputs = ports(&node.outputs);
    spec
}

/// A loaded, validated package. Cheap to clone.
#[derive(Clone)]
pub struct WasmPackage {
    inner: Arc<PackageInner>,
}

impl fmt::Debug for WasmPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasmPackage")
            .field("sha256", &self.sha256_hex())
            .field(
                "nodes",
                &self
                    .inner
                    .specs
                    .iter()
                    .map(NodeSpec::qualified_id)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl WasmPackage {
    /// The node types the package describes, in its order.
    #[must_use]
    pub fn specs(&self) -> &[NodeSpec] {
        &self.inner.specs
    }

    /// SHA-256 of the package bytes, lowercase hex.
    #[must_use]
    pub fn sha256_hex(&self) -> String {
        self.inner.hash.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Whether two handles share one compiled module.
    #[must_use]
    pub fn same_module(&self, other: &WasmPackage) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// One node implementation per described node type.
    #[must_use]
    pub fn nodes(&self) -> Vec<Arc<dyn NodeImpl>> {
        (0..self.inner.specs.len())
            .map(|index| {
                Arc::new(WasmNode {
                    package: Arc::clone(&self.inner),
                    index,
                }) as Arc<dyn NodeImpl>
            })
            .collect()
    }

    /// Register every node type. Returns how many were registered.
    ///
    /// # Errors
    /// Returns the first [`RegistryError`]; node types registered before it
    /// stay registered.
    pub fn register(&self, registry: &mut NodeRegistry) -> Result<usize, RegistryError> {
        let nodes = self.nodes();
        let count = nodes.len();
        for node in nodes {
            registry.register(node)?;
        }
        Ok(count)
    }
}

/// Load every package in `paths` (in order) into `registry`.
///
/// # Errors
/// Returns the path and error of the first package that fails.
pub fn register_files(
    host: &WasmHost,
    registry: &mut NodeRegistry,
    paths: &[PathBuf],
) -> Result<(), (PathBuf, LoadError)> {
    for path in paths {
        host.load_file(path)
            .and_then(|pkg| pkg.register(registry).map_err(LoadError::Registry))
            .map_err(|err| (path.clone(), err))?;
    }
    Ok(())
}

struct PackageInner {
    runner: Runner,
    specs: Vec<NodeSpec>,
    hash: [u8; 32],
}

/// Per-store state: the memory limiter.
struct StoreState {
    limiter: Limiter,
}

struct Limiter {
    memory_bytes: usize,
    memory_hit: bool,
}

/// Instances, tables and their elements a package may have.
const MAX_INSTANCES: usize = 1;
const MAX_TABLES: usize = 4;
const MAX_TABLE_ELEMENTS: usize = 100_000;

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.memory_bytes {
            self.memory_hit = true;
            return Err(wasmtime::Error::msg("memory limit exceeded"));
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= MAX_TABLE_ELEMENTS)
    }

    fn instances(&self) -> usize {
        MAX_INSTANCES
    }

    fn tables(&self) -> usize {
        MAX_TABLES
    }

    fn memories(&self) -> usize {
        1
    }
}

/// Why a call into a package failed.
#[derive(Debug)]
enum CallError {
    /// A trap or other wasmtime error (classified by [`CallError::describe`]).
    Wasm(wasmtime::Error),
    /// The memory limit was hit.
    Memory,
    /// The guest broke the ABI.
    Abi(String),
}

impl CallError {
    fn describe(&self, limits: &WasmLimits) -> String {
        match self {
            CallError::Memory => {
                format!("exceeded the memory limit of {} bytes", limits.memory_bytes)
            }
            CallError::Abi(msg) => format!("broke the ABI: {msg}"),
            CallError::Wasm(err) => match err.downcast_ref::<Trap>() {
                Some(Trap::OutOfFuel) => format!("ran out of fuel (limit {})", limits.fuel),
                Some(Trap::StackOverflow) => "overflowed the wasm stack".to_owned(),
                Some(trap) => format!("trapped: {trap}"),
                None => format!("failed: {err:#}"),
            },
        }
    }
}

/// Runs calls in fresh, limited instances of one compiled module.
struct Runner {
    pre: InstancePre<StoreState>,
    limits: WasmLimits,
}

impl Runner {
    fn call<R>(
        &self,
        f: impl FnOnce(&mut Store<StoreState>, &Instance) -> Result<R, CallError>,
    ) -> Result<R, CallError> {
        let mut store = Store::new(
            self.pre.module().engine(),
            StoreState {
                limiter: Limiter {
                    memory_bytes: self.limits.memory_bytes,
                    memory_hit: false,
                },
            },
        );
        store.limiter(|s| &mut s.limiter);
        let result = store
            .set_fuel(self.limits.fuel)
            .map_err(CallError::Wasm)
            .and_then(|()| self.pre.instantiate(&mut store).map_err(CallError::Wasm))
            .and_then(|instance| f(&mut store, &instance));
        match result {
            Err(_) if store.data().limiter.memory_hit => Err(CallError::Memory),
            other => other,
        }
    }

    fn cook(&self, index: usize, input: &[u8]) -> Result<Vec<u8>, CallError> {
        let abi = |e: wasmtime::Error| CallError::Abi(format!("{e:#}"));
        self.call(|store, instance| {
            let alloc = instance
                .get_typed_func::<i32, i32>(&mut *store, EXPORT_ALLOC)
                .map_err(abi)?;
            let cook = instance
                .get_typed_func::<(i32, i32, i32), i64>(&mut *store, EXPORT_COOK)
                .map_err(abi)?;
            let len = i32::try_from(input.len())
                .map_err(|_| CallError::Abi("cook input too large".to_owned()))?;
            let ptr = alloc.call(&mut *store, len).map_err(CallError::Wasm)?;
            let memory = memory(store, instance)?;
            let offset = usize::try_from(ptr as u32)
                .map_err(|_| CallError::Abi("bad pointer".to_owned()))?;
            memory.write(&mut *store, offset, input).map_err(|_| {
                CallError::Abi(format!(
                    "{EXPORT_ALLOC}({len}) returned {offset:#x}, outside linear memory"
                ))
            })?;
            let node = i32::try_from(index)
                .map_err(|_| CallError::Abi("node index too large".to_owned()))?;
            let packed = cook
                .call(&mut *store, (node, ptr, len))
                .map_err(CallError::Wasm)?;
            read_packed(store, instance, packed)
        })
    }
}

fn memory(store: &mut Store<StoreState>, instance: &Instance) -> Result<Memory, CallError> {
    instance
        .get_memory(&mut *store, EXPORT_MEMORY)
        .ok_or_else(|| CallError::Abi(format!("no '{EXPORT_MEMORY}' export")))
}

/// Copy out the buffer named by a packed `(ptr << 32) | len` result.
fn read_packed(
    store: &mut Store<StoreState>,
    instance: &Instance,
    packed: i64,
) -> Result<Vec<u8>, CallError> {
    let packed = packed as u64;
    let ptr = (packed >> 32) as usize;
    let len = (packed & 0xffff_ffff) as usize;
    let memory = memory(store, instance)?;
    let data = memory.data(&*store);
    ptr.checked_add(len)
        .and_then(|end| data.get(ptr..end))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| {
            CallError::Abi(format!(
                "result buffer {ptr:#x}+{len} is outside linear memory ({} bytes)",
                data.len()
            ))
        })
}

/// One node type of a package.
struct WasmNode {
    package: Arc<PackageInner>,
    index: usize,
}

impl fmt::Debug for WasmNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WasmNode")
            .field("id", &self.spec().qualified_id())
            .finish_non_exhaustive()
    }
}

impl WasmNode {
    fn fail(&self, what: &str) -> NodeError {
        NodeError::Failed(format!("wasm node {} {what}", self.spec().qualified_id()))
    }
}

impl NodeImpl for WasmNode {
    fn spec(&self) -> &NodeSpec {
        // `index` is always in range: nodes are only made by `nodes()`.
        &self.package.specs[self.index]
    }

    fn cook(&self, ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let spec = self.spec();
        // Only time-dependent nodes see the clock; others must not depend
        // on it (they are cached across frames).
        let ctx = if spec.time_dependent {
            *ctx
        } else {
            CookContext::default()
        };
        let input = abi::encode_cook_input(&ctx, inputs)?;
        let limits = &self.package.runner.limits;
        let bytes = self
            .package
            .runner
            .cook(self.index, &input)
            .map_err(|err| self.fail(&err.describe(limits)))?;
        let result = abi::decode_cook_result(&bytes, spec.outputs.len(), MAX_RESULT_STRING)
            .map_err(|err| self.fail(&format!("broke the ABI: {err}")))?;
        let values = match result {
            CookResult::Ok(values) => values,
            CookResult::Failed(reason) => return Err(NodeError::Failed(reason)),
            CookResult::InvalidInput { port, reason } => {
                return Err(match spec.inputs.iter().find(|p| p.name == port) {
                    Some(p) => NodeError::InvalidInput {
                        port: p.name,
                        reason,
                    },
                    None => NodeError::Failed(format!("invalid input '{port}': {reason}")),
                });
            }
        };
        if values.len() != spec.outputs.len() {
            return Err(self.fail(&format!(
                "returned {} values for {} outputs",
                values.len(),
                spec.outputs.len()
            )));
        }
        values
            .into_iter()
            .zip(&spec.outputs)
            .map(|(value, port)| {
                let got = value.port_type();
                value.coerce(port.ty).ok_or_else(|| {
                    self.fail(&format!(
                        "returned {got} for output '{}' of type {}",
                        port.name, port.ty
                    ))
                })
            })
            .collect()
    }
}
