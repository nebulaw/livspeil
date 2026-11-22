// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! System bindings: read-only, host-provided views of the outside world
//! (the file system for now) for nodes such as `core.fs.list_dir`.
//!
//! # Design
//!
//! Nodes stay pure functions. A node that reads the system declares it
//! ([`NodeImpl::reads_system`](crate::NodeImpl::reads_system)) and cooks
//! through [`NodeImpl::cook_with_system`](crate::NodeImpl::cook_with_system),
//! which receives a [`SystemContext`] next to the usual
//! [`CookContext`](crate::CookContext). The context borrows a
//! [`SystemHost`] that the embedding application installs on its cooker.
//!
//! **Snapshot semantics.** A host exposes a [`SystemHost::revision`]. While
//! the revision stays the same, the host promises that the same query gives
//! the same answer, so a system-reading node is a pure function of its
//! inputs *and the revision*. The cooker caches such nodes like any other
//! and re-cooks them (and, through early cutoff, only what really changed
//! downstream) when the revision changes. The host decides when to refresh:
//! a file watcher, a timer, or an explicit "rescan" bumps the revision.
//!
//! **Capabilities.** A [`FileSystem`] is rooted: nodes name files with
//! relative [`FsPath`]s inside that root and can never reach outside it.
//! [`FsPath::parse`] rejects absolute paths, backslashes, NUL and `..`
//! components that would climb above the root; providers must also refuse
//! to follow links out of the root (see `livspeil-nodes-core`'s `StdFs`).
//! Everything is read-only.
//!
//! **Why not a field on `CookContext`.** `CookContext` is a small `Copy`
//! struct with public fields that node authors build with struct literals;
//! adding a (non-`Copy`, lifetime-carrying) host field would break them and
//! make every context heavier. The system context is a separate argument of
//! a separate, defaulted trait method instead, so existing nodes compile and
//! behave unchanged.

use std::fmt;

/// A host's read-only view of the system. Installed on a cooker by the
/// embedding application; see the [module documentation](self).
pub trait SystemHost: Send + Sync + fmt::Debug {
    /// The snapshot revision: equal revisions promise equal answers.
    fn revision(&self) -> u64;

    /// The rooted file system, if the host grants one.
    fn fs(&self) -> Option<&dyn FileSystem> {
        None
    }
}

/// What a system-reading node sees while cooking.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct SystemContext<'a> {
    host: Option<&'a dyn SystemHost>,
}

impl<'a> SystemContext<'a> {
    /// A context backed by `host`.
    #[must_use]
    pub fn new(host: &'a dyn SystemHost) -> Self {
        SystemContext { host: Some(host) }
    }

    /// A context without any system access.
    #[must_use]
    pub fn none() -> Self {
        SystemContext { host: None }
    }

    /// The host, if any.
    #[must_use]
    pub fn host(&self) -> Option<&'a dyn SystemHost> {
        self.host
    }

    /// The host's rooted file system, if any.
    #[must_use]
    pub fn fs(&self) -> Option<&'a dyn FileSystem> {
        self.host.and_then(|h| h.fs())
    }

    /// The host's snapshot revision (0 without a host).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.host.map_or(0, |h| h.revision())
    }
}

/// A normalized path relative to a [`FileSystem`]'s root: components
/// joined by `/`, no `.` or `..`, no leading or trailing `/`. The root
/// itself is the empty path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct FsPath(String);

impl FsPath {
    /// The root.
    #[must_use]
    pub fn root() -> Self {
        FsPath(String::new())
    }

    /// Parse and normalize a relative path. `""`, `"."` and `"./"` are the
    /// root; repeated `/` and `.` components are dropped; `..` removes the
    /// previous component.
    ///
    /// # Errors
    /// [`FsError::InvalidPath`] for absolute paths, `\` and NUL, and [`FsError::OutsideRoot`] when
    /// `..` would climb above the root.
    pub fn parse(text: &str) -> Result<Self, FsError> {
        let invalid = |reason: &str| FsError::InvalidPath {
            path: text.to_owned(),
            reason: reason.to_owned(),
        };
        if text.starts_with('/') {
            return Err(invalid(
                "absolute paths are not allowed; paths are relative to the file root",
            ));
        }
        if text.contains('\\') {
            return Err(invalid("use '/' to separate path components"));
        }
        if text.contains('\0') {
            return Err(invalid("paths must not contain NUL"));
        }
        let mut parts: Vec<&str> = Vec::new();
        for part in text.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err(FsError::OutsideRoot(text.to_owned()));
                    }
                }
                part => parts.push(part),
            }
        }
        Ok(FsPath(parts.join("/")))
    }

    /// The path text (`""` for the root).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The components, outermost first.
    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|c| !c.is_empty())
    }

    /// This path with the entry `name` appended. `name` must be a single
    /// component (no `/`, not `.` or `..`).
    ///
    /// # Errors
    /// [`FsError::InvalidPath`] when `name` is not a single component.
    pub fn join(&self, name: &str) -> Result<FsPath, FsError> {
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(FsError::InvalidPath {
                path: name.to_owned(),
                reason: "not a single path component".to_owned(),
            });
        }
        Ok(if self.is_root() {
            FsPath(name.to_owned())
        } else {
            FsPath(format!("{}/{name}", self.0))
        })
    }
}

impl fmt::Display for FsPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_root() {
            f.write_str(".")
        } else {
            f.write_str(&self.0)
        }
    }
}

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirEntry {
    /// File name (a single path component, valid UTF-8).
    pub name: String,
    /// Whether the entry is a directory (links are followed).
    pub is_dir: bool,
    /// Size in bytes; 0 for directories.
    pub size: u64,
    /// Last modification, whole seconds since the Unix epoch (negative
    /// before it, 0 if unknown).
    pub modified: i64,
}

/// Why a file-system query failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FsError {
    /// The path is not acceptable (see [`FsPath::parse`]).
    InvalidPath { path: String, reason: String },
    /// The path, or a link it goes through, leads outside the root.
    OutsideRoot(String),
    /// Nothing exists at the path.
    NotFound(String),
    /// The path is not a directory.
    NotADirectory(String),
    /// The host refused access.
    PermissionDenied(String),
    /// Any other I/O failure.
    Io { path: String, message: String },
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsError::InvalidPath { path, reason } => write!(f, "invalid path '{path}': {reason}"),
            FsError::OutsideRoot(path) => {
                write!(f, "path '{path}' leads outside the allowed file root")
            }
            FsError::NotFound(path) => write!(f, "'{path}' does not exist"),
            FsError::NotADirectory(path) => write!(f, "'{path}' is not a directory"),
            FsError::PermissionDenied(path) => write!(f, "permission denied for '{path}'"),
            FsError::Io { path, message } => write!(f, "cannot read '{path}': {message}"),
        }
    }
}

impl std::error::Error for FsError {}

/// A rooted, read-only file system. See the [module documentation](self).
pub trait FileSystem: Send + Sync + fmt::Debug {
    /// The entries directly inside directory `dir`, in any order (nodes
    /// sort them). Must be deterministic for a given host revision.
    ///
    /// # Errors
    /// An [`FsError`] when `dir` cannot be listed.
    fn list_dir(&self, dir: &FsPath) -> Result<Vec<DirEntry>, FsError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_normalizes() {
        for (text, expected) in [
            ("", ""),
            (".", ""),
            ("./", ""),
            ("books", "books"),
            ("books/", "books"),
            ("./books//sci-fi/.", "books/sci-fi"),
            ("a/b/../c", "a/c"),
            ("a/..", ""),
            ("héllo wörld", "héllo wörld"),
        ] {
            assert_eq!(FsPath::parse(text).unwrap().as_str(), expected, "{text:?}");
        }
    }

    #[test]
    fn parse_rejects_escapes() {
        assert!(matches!(
            FsPath::parse("/etc"),
            Err(FsError::InvalidPath { .. })
        ));
        assert!(matches!(
            FsPath::parse("a\\b"),
            Err(FsError::InvalidPath { .. })
        ));
        assert!(matches!(
            FsPath::parse("a\0"),
            Err(FsError::InvalidPath { .. })
        ));
        assert_eq!(FsPath::parse(".."), Err(FsError::OutsideRoot("..".into())));
        assert_eq!(
            FsPath::parse("a/../../b"),
            Err(FsError::OutsideRoot("a/../../b".into()))
        );
    }

    #[test]
    fn join_and_components() {
        let root = FsPath::root();
        let a = root.join("a").unwrap();
        let ab = a.join("b c").unwrap();
        assert_eq!(ab.as_str(), "a/b c");
        assert_eq!(ab.components().collect::<Vec<_>>(), ["a", "b c"]);
        assert_eq!(root.components().count(), 0);
        assert!(a.join("..").is_err());
        assert!(a.join("x/y").is_err());
        assert_eq!(root.to_string(), ".");
    }

    #[derive(Debug)]
    struct Host;
    impl SystemHost for Host {
        fn revision(&self) -> u64 {
            7
        }
    }

    #[test]
    fn context_without_fs() {
        let none = SystemContext::none();
        assert_eq!(none.revision(), 0);
        assert!(none.fs().is_none());
        let host = Host;
        let ctx = SystemContext::new(&host);
        assert_eq!(ctx.revision(), 7);
        assert!(ctx.fs().is_none());
        assert!(ctx.host().is_some());
    }
}
