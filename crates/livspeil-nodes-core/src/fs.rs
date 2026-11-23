// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! File-system nodes and providers (milestone M0.8).
//!
//! - [`ListDir`] (`core.fs.list_dir@1`) lists a folder into a table; it is
//!   the source of procedural scenes such as a bookshelf generated from the
//!   files in a folder.
//! - [`StdFs`] is the real, read-only provider over `std::fs`, rooted at a
//!   directory (the CLI roots it at the room package).
//! - [`MemFs`] is an in-memory provider for tests and previews.
//!
//! Both providers are [`SystemHost`]s with a revision counter: the host
//! decides when the outside world is looked at again by calling `bump`,
//! and the cooker re-cooks `list_dir` nodes when the revision changes (see
//! `livspeil_node_api::system`). [`StdFs`] memoizes listings per revision,
//! so within one revision every query has one answer even if the disk
//! changes meanwhile (true snapshot semantics).
//!
//! # Refresh
//!
//! By default a [`StdFs`] is a snapshot: it looks at the disk again only
//! when the host calls [`StdFs::bump`]. With a [`RefreshPolicy::Watch`]
//! policy ([`StdFs::with_refresh`]) it also watches the folders it has
//! actually listed in the current revision: [`StdFs::refresh`], which the
//! host calls once per tick before cooking (at most once per
//! `min_interval`), compares a cheap [`DirStamp`] of each of them (the
//! folder's modification time and number of entries) with the one taken
//! when it was listed, and bumps the revision when any differs (also when
//! a folder that was missing appears, or one disappears). Only
//! system-reading nodes are then re-cooked, and their dependents only if
//! the listing really changed. Calling `refresh` once per tick keeps one
//! answer per tick. Not detected: a file changed in place without the
//! folder's modification time changing (its size or time in the listing
//! stays stale until the next detected change or `bump`). The clock and
//! the stamp function can be replaced ([`StdFs::with_clock`],
//! [`StdFs::with_stat`]) for deterministic tests.
//!
//! # Capability rules
//!
//! Nodes name folders with relative paths inside the provider's root
//! ([`FsPath`]): absolute paths and `..` above the root are rejected
//! before any I/O. [`StdFs`] also resolves links: a folder whose real
//! location is outside the root cannot be listed, and entries that are
//! links leading outside the root are left out of listings. Names that are
//! not valid UTF-8 are left out too. Nothing is ever written.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use livspeil_node_api::{
    CookContext, DirEntry, FileSystem, FsError, FsPath, NodeError, NodeImpl, NodeSpec, PortType,
    SystemContext, SystemHost, Table, Value,
};

use crate::string;

/// Whether `name` matches the glob `pattern`: `*` matches any run of
/// characters (also none), `?` exactly one character, everything else
/// itself (case-sensitive, by Unicode scalar value). Several patterns may
/// be given separated by `;` (`"*.pdf;*.epub"`); empty parts are ignored,
/// and a pattern with no non-empty part matches everything.
#[must_use]
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split(';').filter(|p| !p.is_empty()).peekable();
    if parts.peek().is_none() {
        return true;
    }
    let name: Vec<char> = name.chars().collect();
    parts.any(|p| glob_one(&p.chars().collect::<Vec<_>>(), &name))
}

/// One glob, iterative with single-star backtracking: linear for patterns
/// with one `*`, `O(pattern * name)` at worst.
fn glob_one(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    p = sp + 1;
                    n = sn + 1;
                    star = Some((sp, sn + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

/// Seconds since the Unix epoch, negative before it, 0 if unknown.
fn unix_seconds(time: io::Result<SystemTime>) -> i64 {
    match time {
        Ok(t) => match t.duration_since(UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            Err(e) => {
                let d = e.duration();
                // Floor: 0.5 s before the epoch is second -1.
                let secs = i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
                if d.subsec_nanos() > 0 {
                    -secs - 1
                } else {
                    -secs
                }
            }
        },
        Err(_) => 0,
    }
}

fn io_error(path: &FsPath, err: &io::Error) -> FsError {
    let shown = path.to_string();
    match err.kind() {
        io::ErrorKind::NotFound => FsError::NotFound(shown),
        io::ErrorKind::PermissionDenied => FsError::PermissionDenied(shown),
        _ => FsError::Io {
            path: shown,
            message: err.to_string(),
        },
    }
}

type Listing = Result<Vec<DirEntry>, FsError>;

/// When a [`StdFs`] looks at the disk again (see "Refresh" in the
/// [module documentation](self)).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RefreshPolicy {
    /// Only on [`StdFs::bump`] (the default).
    #[default]
    Snapshot,
    /// Also when [`StdFs::refresh`] finds that a listed folder changed;
    /// it checks at most once per `min_interval` (zero: every call).
    Watch { min_interval: Duration },
}

/// A cheap fingerprint of a folder, compared by [`StdFs::refresh`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirStamp {
    /// The folder's modification time, if the platform has one.
    pub modified: Option<SystemTime>,
    /// Number of entries.
    pub entries: u64,
}

impl DirStamp {
    /// The stamp of the folder at `path` (links followed), or `None` when
    /// it is missing or not a folder.
    #[must_use]
    pub fn of(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        if !meta.is_dir() {
            return None;
        }
        let entries = fs::read_dir(path).ok()?.count();
        Some(DirStamp {
            modified: meta.modified().ok(),
            entries: u64::try_from(entries).unwrap_or(u64::MAX),
        })
    }
}

/// Monotonic time since an arbitrary origin.
type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;
/// The stamp of a folder (see [`DirStamp::of`]).
type StatFn = Arc<dyn Fn(&Path) -> Option<DirStamp> + Send + Sync>;

/// What [`StdFs::refresh`] watches.
#[derive(Debug, Default)]
struct Watched {
    /// Clock reading of the last check.
    last_check: Option<Duration>,
    /// Folders listed in the current revision and their stamps then.
    dirs: HashMap<PathBuf, Option<DirStamp>>,
}

/// The real file system below a root directory, read-only. See the
/// [module documentation](self) for the capability rules and refresh.
pub struct StdFs {
    root: PathBuf,
    revision: AtomicU64,
    /// Listings of the revision in the key.
    cache: Mutex<(u64, HashMap<FsPath, Listing>)>,
    policy: RefreshPolicy,
    clock: Clock,
    stat: StatFn,
    watched: Mutex<Watched>,
}

impl fmt::Debug for StdFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StdFs")
            .field("root", &self.root)
            .field("revision", &self.revision)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl StdFs {
    /// A provider rooted at `root`, which must be an existing directory.
    ///
    /// # Errors
    /// The I/O error of resolving `root`, or `NotADirectory`-kind
    /// [`io::Error`] if it is not a directory.
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a directory", root.display()),
            ));
        }
        let start = Instant::now();
        Ok(StdFs {
            root,
            revision: AtomicU64::new(1),
            cache: Mutex::new((0, HashMap::new())),
            policy: RefreshPolicy::Snapshot,
            clock: Arc::new(move || start.elapsed()),
            stat: Arc::new(DirStamp::of),
            watched: Mutex::new(Watched::default()),
        })
    }

    /// Builder: use `policy` (see "Refresh" in the module docs).
    #[must_use]
    pub fn with_refresh(mut self, policy: RefreshPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Builder: read time from `clock` (monotonic, since any origin)
    /// instead of the system's monotonic clock.
    #[must_use]
    pub fn with_clock(mut self, clock: impl Fn() -> Duration + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// Builder: fingerprint folders with `stat` instead of
    /// [`DirStamp::of`].
    #[must_use]
    pub fn with_stat(
        mut self,
        stat: impl Fn(&Path) -> Option<DirStamp> + Send + Sync + 'static,
    ) -> Self {
        self.stat = Arc::new(stat);
        self
    }

    /// The refresh policy.
    #[must_use]
    pub fn refresh_policy(&self) -> RefreshPolicy {
        self.policy
    }

    /// With [`RefreshPolicy::Watch`], check the folders listed in the
    /// current revision (unless the last check is less than
    /// `min_interval` ago) and [`bump`](StdFs::bump) the revision if any
    /// changed. Returns whether it bumped. Does nothing with
    /// [`RefreshPolicy::Snapshot`]. Call it once per tick, before cooking.
    pub fn refresh(&self) -> bool {
        let RefreshPolicy::Watch { min_interval } = self.policy else {
            return false;
        };
        let now = (self.clock)();
        let mut watched = self.watched.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(last) = watched.last_check
            && now.saturating_sub(last) < min_interval
        {
            return false;
        }
        watched.last_check = Some(now);
        let changed = watched
            .dirs
            .iter()
            .any(|(path, stamp)| (self.stat)(path) != *stamp);
        if changed {
            // The next listings record new stamps.
            watched.dirs.clear();
            self.bump();
        }
        changed
    }

    /// Remember the stamp of a folder about to be listed (when watching).
    fn watch(&self, real: PathBuf) {
        if self.policy == RefreshPolicy::Snapshot {
            return;
        }
        let stamp = (self.stat)(&real);
        self.watched
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dirs
            .insert(real, stamp);
    }

    /// The canonical root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Start a new snapshot: the next cook looks at the disk again.
    pub fn bump(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    fn inside(&self, real: &Path) -> bool {
        real.starts_with(&self.root)
    }

    fn read(&self, dir: &FsPath) -> Listing {
        let mut joined = self.root.clone();
        joined.extend(dir.components());
        let real = match fs::canonicalize(&joined) {
            Ok(real) => real,
            Err(e) => {
                // Watch for a missing folder to appear.
                self.watch(joined);
                return Err(io_error(dir, &e));
            }
        };
        if !self.inside(&real) {
            return Err(FsError::OutsideRoot(dir.to_string()));
        }
        self.watch(real.clone());
        let meta = fs::metadata(&real).map_err(|e| io_error(dir, &e))?;
        if !meta.is_dir() {
            return Err(FsError::NotADirectory(dir.to_string()));
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&real).map_err(|e| io_error(dir, &e))? {
            let entry = entry.map_err(|e| io_error(dir, &e))?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let path = entry.path();
            let is_link = entry.file_type().is_ok_and(|t| t.is_symlink());
            if is_link && !fs::canonicalize(&path).is_ok_and(|target| self.inside(&target)) {
                // Links leading outside the root (or nowhere) are left out.
                continue;
            }
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            let is_dir = meta.is_dir();
            out.push(DirEntry {
                name,
                is_dir,
                size: if is_dir { 0 } else { meta.len() },
                modified: unix_seconds(meta.modified()),
            });
        }
        Ok(out)
    }
}

impl FileSystem for StdFs {
    fn list_dir(&self, dir: &FsPath) -> Result<Vec<DirEntry>, FsError> {
        let revision = self.revision.load(Ordering::Relaxed);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.0 != revision {
            *cache = (revision, HashMap::new());
        }
        if let Some(hit) = cache.1.get(dir) {
            return hit.clone();
        }
        let listing = self.read(dir);
        cache.1.insert(dir.clone(), listing.clone());
        listing
    }
}

impl SystemHost for StdFs {
    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    fn fs(&self) -> Option<&dyn FileSystem> {
        Some(self)
    }
}

/// A file or folder of a [`MemFs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MemEntry {
    is_dir: bool,
    size: u64,
    modified: i64,
}

/// An in-memory file system for tests and previews. Paths are
/// [`FsPath`] text; adding an entry creates its missing parent folders
/// (modified 0). Changes are visible immediately; call [`MemFs::bump`]
/// to tell cookers to look again.
#[derive(Debug)]
pub struct MemFs {
    entries: Mutex<BTreeMap<FsPath, MemEntry>>,
    revision: AtomicU64,
}

impl Default for MemFs {
    fn default() -> Self {
        Self::new()
    }
}

impl MemFs {
    /// An empty file system (just the root folder).
    #[must_use]
    pub fn new() -> Self {
        MemFs {
            entries: Mutex::new(BTreeMap::new()),
            revision: AtomicU64::new(1),
        }
    }

    /// Builder form of [`MemFs::add_file`].
    ///
    /// # Errors
    /// As [`MemFs::add_file`].
    pub fn with_file(self, path: &str, size: u64, modified: i64) -> Result<Self, FsError> {
        self.add_file(path, size, modified)?;
        Ok(self)
    }

    /// Builder form of [`MemFs::add_dir`].
    ///
    /// # Errors
    /// As [`MemFs::add_dir`].
    pub fn with_dir(self, path: &str, modified: i64) -> Result<Self, FsError> {
        self.add_dir(path, modified)?;
        Ok(self)
    }

    /// Add or replace a file.
    ///
    /// # Errors
    /// [`FsError`] for an invalid path, the root, or a parent that is a
    /// file.
    pub fn add_file(&self, path: &str, size: u64, modified: i64) -> Result<(), FsError> {
        self.add(
            path,
            MemEntry {
                is_dir: false,
                size,
                modified,
            },
        )
    }

    /// Add or replace a folder.
    ///
    /// # Errors
    /// As [`MemFs::add_file`].
    pub fn add_dir(&self, path: &str, modified: i64) -> Result<(), FsError> {
        self.add(
            path,
            MemEntry {
                is_dir: true,
                size: 0,
                modified,
            },
        )
    }

    /// Remove an entry and everything below it. Returns whether it existed.
    ///
    /// # Errors
    /// [`FsError`] for an invalid path.
    pub fn remove(&self, path: &str) -> Result<bool, FsError> {
        let path = FsPath::parse(path)?;
        let mut entries = self.lock();
        let existed = entries.remove(&path).is_some();
        let prefix = format!("{}/", path.as_str());
        entries.retain(|p, _| !p.as_str().starts_with(&prefix));
        Ok(existed)
    }

    /// Start a new snapshot revision.
    pub fn bump(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<FsPath, MemEntry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn add(&self, path: &str, entry: MemEntry) -> Result<(), FsError> {
        let path = FsPath::parse(path)?;
        if path.is_root() {
            return Err(FsError::InvalidPath {
                path: ".".to_owned(),
                reason: "the root always exists".to_owned(),
            });
        }
        let mut entries = self.lock();
        let mut parent = FsPath::root();
        let components: Vec<&str> = path.components().collect();
        for component in &components[..components.len() - 1] {
            parent = parent.join(component)?;
            match entries.get(&parent) {
                Some(e) if !e.is_dir => return Err(FsError::NotADirectory(parent.to_string())),
                Some(_) => {}
                None => {
                    entries.insert(
                        parent.clone(),
                        MemEntry {
                            is_dir: true,
                            size: 0,
                            modified: 0,
                        },
                    );
                }
            }
        }
        entries.insert(path, entry);
        Ok(())
    }
}

impl FileSystem for MemFs {
    fn list_dir(&self, dir: &FsPath) -> Result<Vec<DirEntry>, FsError> {
        let entries = self.lock();
        if !dir.is_root() {
            match entries.get(dir) {
                None => return Err(FsError::NotFound(dir.to_string())),
                Some(e) if !e.is_dir => return Err(FsError::NotADirectory(dir.to_string())),
                Some(_) => {}
            }
        }
        let depth = dir.components().count();
        Ok(entries
            .iter()
            .filter(|(path, _)| {
                path.components().count() == depth + 1
                    && path.components().zip(dir.components()).all(|(a, b)| a == b)
            })
            .filter_map(|(path, e)| {
                Some(DirEntry {
                    name: path.components().last()?.to_owned(),
                    is_dir: e.is_dir,
                    size: e.size,
                    modified: e.modified,
                })
            })
            .collect())
    }
}

impl SystemHost for MemFs {
    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    fn fs(&self) -> Option<&dyn FileSystem> {
        Some(self)
    }
}

/// Sort orders of `core.fs.list_dir`.
const SORTS: [&str; 3] = ["name", "size", "modified"];

/// `core.fs.list_dir@1`: the entries of a folder as a table.
///
/// Inputs: `path` (String, default `"."`: the file root), `pattern`
/// (String glob, default `"*"`; see [`glob_match`]), `include_hidden`
/// (Bool, default false: names starting with `.` are left out), `sort`
/// (String: `"name"` (default), `"size"` or `"modified"`). Outputs:
/// `entries` (Table with columns `name` String, `path` String (relative to
/// the file root, `/`-separated), `is_dir` Bool, `size` Int (bytes, 0 for
/// folders), `modified` Int (Unix seconds)) and `count` (Int, rows).
///
/// The pattern applies to folders too. Sorting is ascending; `name`
/// compares names by Unicode scalar values, `size` and `modified` break
/// ties by name, so the order never depends on the provider.
///
/// Reads the system ([`NodeImpl::reads_system`]): the result is a pure
/// function of the inputs and the host's snapshot revision. Without a file
/// system (no host, or a host that grants none) the node fails. Path
/// problems (absolute, outside the root, missing, not a folder) are
/// [`NodeError::InvalidInput`] on `path`; other I/O errors are
/// [`NodeError::Failed`].
#[derive(Debug)]
pub struct ListDir {
    spec: NodeSpec,
}

impl ListDir {
    #[must_use]
    pub fn new() -> Self {
        ListDir {
            spec: NodeSpec::new(
                "core.fs.list_dir",
                1,
                "List the files of a folder as a table (name, path, is_dir, size, modified)",
            )
            .input(
                "path",
                PortType::String,
                Some(Value::String(".".to_owned())),
            )
            .input(
                "pattern",
                PortType::String,
                Some(Value::String("*".to_owned())),
            )
            .input("include_hidden", PortType::Bool, Some(Value::Bool(false)))
            .input(
                "sort",
                PortType::String,
                Some(Value::String("name".to_owned())),
            )
            .output("entries", PortType::Table)
            .output("count", PortType::Int),
        }
    }

    /// The schema of the `entries` table.
    pub const COLUMNS: [(&'static str, PortType); 5] = [
        ("name", PortType::String),
        ("path", PortType::String),
        ("is_dir", PortType::Bool),
        ("size", PortType::Int),
        ("modified", PortType::Int),
    ];
}

impl Default for ListDir {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for ListDir {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn reads_system(&self) -> bool {
        true
    }

    fn cook(&self, ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        self.cook_with_system(ctx, &SystemContext::none(), inputs)
    }

    fn cook_with_system(
        &self,
        _ctx: &CookContext,
        system: &SystemContext<'_>,
        inputs: &[Value],
    ) -> Result<Vec<Value>, NodeError> {
        let path = string(inputs, 0, "path")?;
        let pattern = string(inputs, 1, "pattern")?;
        let include_hidden = match inputs.get(2) {
            Some(Value::Bool(b)) => *b,
            _ => {
                return Err(NodeError::InvalidInput {
                    port: "include_hidden",
                    reason: "expected a Bool".to_owned(),
                });
            }
        };
        let sort = string(inputs, 3, "sort")?;
        if !SORTS.contains(&sort) {
            return Err(NodeError::InvalidInput {
                port: "sort",
                reason: format!(
                    "unknown sort \"{sort}\"; expected \"name\", \"size\" or \"modified\""
                ),
            });
        }
        let path_error = |e: FsError| match e {
            FsError::Io { .. } => NodeError::Failed(e.to_string()),
            e => NodeError::InvalidInput {
                port: "path",
                reason: e.to_string(),
            },
        };
        let dir = FsPath::parse(path).map_err(path_error)?;
        let fs = system.fs().ok_or_else(|| {
            NodeError::Failed("no file system is available here (the host granted none)".to_owned())
        })?;
        let mut listing: Vec<DirEntry> = fs
            .list_dir(&dir)
            .map_err(path_error)?
            .into_iter()
            .filter(|e| include_hidden || !e.name.starts_with('.'))
            .filter(|e| glob_match(pattern, &e.name))
            .collect();
        match sort {
            "size" => listing.sort_by(|a, b| a.size.cmp(&b.size).then_with(|| a.name.cmp(&b.name))),
            "modified" => listing.sort_by(|a, b| {
                a.modified
                    .cmp(&b.modified)
                    .then_with(|| a.name.cmp(&b.name))
            }),
            _ => listing.sort_by(|a, b| a.name.cmp(&b.name)),
        }
        let mut rows = Vec::with_capacity(listing.len());
        for e in listing {
            let Ok(full) = dir.join(&e.name) else {
                continue;
            };
            rows.push(vec![
                Value::String(e.name),
                Value::String(full.as_str().to_owned()),
                Value::Bool(e.is_dir),
                Value::Int(i64::try_from(e.size).unwrap_or(i64::MAX)),
                Value::Int(e.modified),
            ]);
        }
        let table = Table::from_rows(&Self::COLUMNS, rows)
            .map_err(|e| NodeError::Failed(format!("building the listing failed: {e}")))?;
        let count = i64::try_from(table.row_count()).unwrap_or(i64::MAX);
        Ok(vec![Value::Table(table), Value::Int(count)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches() {
        for (pattern, name, expected) in [
            ("*", "anything", true),
            ("*", "", true),
            ("", "x", true),
            (";", "x", true),
            ("*.pdf", "book.pdf", true),
            ("*.pdf", "book.PDF", false),
            ("*.pdf", "book.pdf.txt", false),
            ("b?ok*", "book.pdf", true),
            ("b?ok", "bk", false),
            ("*a*b*c", "xxaxxbxxbxc", true),
            ("*a*b*c", "xxaxxbxxbx", false),
            ("a**", "a", true),
            ("?", "é", true),
            ("*.pdf;*.epub", "x.epub", true),
            ("*.pdf;*.epub", "x.mobi", false),
            ("exact", "exact", true),
            ("exact", "exactly", false),
        ] {
            assert_eq!(glob_match(pattern, name), expected, "{pattern:?} {name:?}");
        }
    }

    #[test]
    fn unix_seconds_floor() {
        use std::time::Duration;
        assert_eq!(
            unix_seconds(Ok(UNIX_EPOCH + Duration::from_millis(1500))),
            1
        );
        assert_eq!(
            unix_seconds(Ok(UNIX_EPOCH - Duration::from_millis(500))),
            -1
        );
        assert_eq!(unix_seconds(Ok(UNIX_EPOCH - Duration::from_secs(2))), -2);
        assert_eq!(unix_seconds(Err(io::Error::other("x"))), 0);
    }

    #[test]
    fn mem_fs_lists_and_rejects() {
        let fs = MemFs::new()
            .with_file("books/a.pdf", 10, 5)
            .unwrap()
            .with_dir("books/sub", 7)
            .unwrap()
            .with_file("books/sub/deep.txt", 1, 1)
            .unwrap();
        let mut names: Vec<String> = fs
            .list_dir(&FsPath::parse("books").unwrap())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, ["a.pdf", "sub"]);
        let root = fs.list_dir(&FsPath::root()).unwrap();
        assert_eq!(root.len(), 1);
        assert!(root[0].is_dir);
        assert_eq!(
            fs.list_dir(&FsPath::parse("nope").unwrap()),
            Err(FsError::NotFound("nope".into()))
        );
        assert_eq!(
            fs.list_dir(&FsPath::parse("books/a.pdf").unwrap()),
            Err(FsError::NotADirectory("books/a.pdf".into()))
        );
        assert!(fs.add_file("books/a.pdf/x", 1, 1).is_err());
        assert!(fs.add_file(".", 1, 1).is_err());
        assert_eq!(fs.remove("books/sub"), Ok(true));
        assert_eq!(fs.remove("books/sub"), Ok(false));
        assert_eq!(
            fs.list_dir(&FsPath::parse("books").unwrap()).unwrap().len(),
            1
        );
        let r = fs.revision();
        fs.bump();
        assert_eq!(fs.revision(), r + 1);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("livspeil-fs-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn std_fs_lists_snapshots_and_stays_inside_the_root() {
        let base = temp_dir("std");
        let root = base.join("root");
        fs::create_dir_all(root.join("books/sub")).unwrap();
        fs::write(root.join("books/a.pdf"), b"0123456789").unwrap();
        fs::write(base.join("secret.txt"), b"x").unwrap();
        let provider = StdFs::new(&root).unwrap();
        let books = FsPath::parse("books").unwrap();
        let mut listing = provider.list_dir(&books).unwrap();
        listing.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(listing.len(), 2);
        assert_eq!((listing[0].name.as_str(), listing[0].size), ("a.pdf", 10));
        assert!(!listing[0].is_dir);
        assert!(listing[0].modified > 0);
        assert_eq!((listing[1].name.as_str(), listing[1].size), ("sub", 0));
        assert!(listing[1].is_dir);

        // Same revision: the snapshot does not see the new file.
        fs::write(root.join("books/b.pdf"), b"").unwrap();
        assert_eq!(provider.list_dir(&books).unwrap().len(), 2);
        provider.bump();
        assert_eq!(provider.list_dir(&books).unwrap().len(), 3);

        assert!(matches!(
            provider.list_dir(&FsPath::parse("missing").unwrap()),
            Err(FsError::NotFound(_))
        ));
        assert!(matches!(
            provider.list_dir(&FsPath::parse("books/a.pdf").unwrap()),
            Err(FsError::NotADirectory(_))
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(&base, root.join("escape")).unwrap();
            symlink(base.join("secret.txt"), root.join("books/leak.txt")).unwrap();
            symlink(root.join("books/a.pdf"), root.join("books/alias.pdf")).unwrap();
            provider.bump();
            assert!(matches!(
                provider.list_dir(&FsPath::parse("escape").unwrap()),
                Err(FsError::OutsideRoot(_))
            ));
            let names: Vec<String> = provider
                .list_dir(&FsPath::root())
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert!(!names.contains(&"escape".to_owned()), "{names:?}");
            let mut names: Vec<String> = provider
                .list_dir(&books)
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            names.sort();
            assert_eq!(names, ["a.pdf", "alias.pdf", "b.pdf", "sub"]);
        }
        assert!(StdFs::new(root.join("books/a.pdf")).is_err());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn snapshot_policy_never_refreshes() {
        let base = temp_dir("snapshot");
        let provider = StdFs::new(&base).unwrap();
        assert_eq!(provider.refresh_policy(), RefreshPolicy::Snapshot);
        assert!(provider.list_dir(&FsPath::root()).unwrap().is_empty());
        fs::write(base.join("new.txt"), b"").unwrap();
        assert!(!provider.refresh());
        assert!(provider.list_dir(&FsPath::root()).unwrap().is_empty());
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn watch_bumps_when_listed_folders_change() {
        let base = temp_dir("watch");
        fs::create_dir_all(base.join("books")).unwrap();
        fs::create_dir_all(base.join("other")).unwrap();
        let provider = StdFs::new(&base)
            .unwrap()
            .with_refresh(RefreshPolicy::Watch {
                min_interval: Duration::ZERO,
            });
        let books = FsPath::parse("books").unwrap();
        let names = |p: &StdFs| -> Vec<String> {
            let mut names: Vec<String> = p
                .list_dir(&books)
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            names.sort();
            names
        };
        // Nothing listed yet: nothing to watch.
        assert!(!provider.refresh());
        assert!(names(&provider).is_empty());
        let revision = provider.revision();
        assert!(!provider.refresh());
        // A folder that was not listed does not count.
        fs::write(base.join("other/x.txt"), b"").unwrap();
        assert!(!provider.refresh());
        assert_eq!(provider.revision(), revision);
        // Created and deleted files are noticed (the entry count changes).
        fs::write(base.join("books/a.pdf"), b"a").unwrap();
        assert!(provider.refresh());
        assert_eq!(provider.revision(), revision + 1);
        assert_eq!(names(&provider), ["a.pdf"]);
        assert!(!provider.refresh());
        fs::remove_file(base.join("books/a.pdf")).unwrap();
        assert!(provider.refresh());
        assert!(names(&provider).is_empty());
        // A missing folder is watched until it appears.
        let later = FsPath::parse("later").unwrap();
        assert!(matches!(
            provider.list_dir(&later),
            Err(FsError::NotFound(_))
        ));
        assert!(!provider.refresh());
        fs::create_dir_all(base.join("later")).unwrap();
        assert!(provider.refresh());
        assert_eq!(provider.list_dir(&later).unwrap().len(), 0);
        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn watch_is_rate_limited_and_deterministic_with_injected_clock_and_stat() {
        let base = temp_dir("inject");
        let millis = Arc::new(AtomicU64::new(0));
        let stamp = Arc::new(AtomicU64::new(0));
        let (m, st) = (Arc::clone(&millis), Arc::clone(&stamp));
        let stats = Arc::new(AtomicU64::new(0));
        let calls = Arc::clone(&stats);
        let provider = StdFs::new(&base)
            .unwrap()
            .with_refresh(RefreshPolicy::Watch {
                min_interval: Duration::from_millis(100),
            })
            .with_clock(move || Duration::from_millis(m.load(Ordering::Relaxed)))
            .with_stat(move |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                Some(DirStamp {
                    modified: None,
                    entries: st.load(Ordering::Relaxed),
                })
            });
        provider.list_dir(&FsPath::root()).unwrap();
        let listed = stats.load(Ordering::Relaxed);
        assert_eq!(listed, 1, "one stamp when listed");
        // First check at t=0: unchanged.
        assert!(!provider.refresh());
        stamp.store(1, Ordering::Relaxed);
        // t=50 ms: too soon, the folder is not even looked at.
        millis.store(50, Ordering::Relaxed);
        let before = stats.load(Ordering::Relaxed);
        assert!(!provider.refresh());
        assert_eq!(stats.load(Ordering::Relaxed), before);
        // t=100 ms: checked, changed.
        millis.store(100, Ordering::Relaxed);
        assert!(provider.refresh());
        // Nothing is watched until the next listing.
        millis.store(300, Ordering::Relaxed);
        stamp.store(2, Ordering::Relaxed);
        assert!(!provider.refresh());
        fs::remove_dir_all(&base).ok();
    }
}
