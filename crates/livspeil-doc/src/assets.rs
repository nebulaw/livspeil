// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Room assets: the `assets.lock` file and its license policy.
//!
//! A room package may hold files in `assets/` (flat, content-addressed:
//! `assets/<sha256>.<ext>`) and must then record every one of them in
//! `assets.lock`, a KDL v2 file next to `room.kdl`:
//!
//! ```kdl
//! assets-lock format=1 {
//!   asset "assets/2c26b4...7ae.txt" sha256="2c26b4...7ae" size=12 license="CC0-1.0" source="original" author="Jane Doe"
//! }
//! ```
//!
//! - **Header**: `assets-lock format=1`, the only top-level node.
//! - **`asset "<path>"`**: `path` is `assets/<name>`, where `name` uses only
//!   ASCII letters, digits, `.`, `_` and `-` and does not start with `.`.
//!   Properties, in canonical order: `sha256=` (64 lowercase hex digits),
//!   `size=` (bytes), `license=` (an SPDX license expression), `source=`
//!   (`"original"` or a URL such as `https://...`) and the optional
//!   `author=` (name or attribution text). All but `author` are required.
//!
//! [`AssetsLock::parse`] is strict (unknown nodes or properties, missing or
//! malformed values and duplicate paths are errors with line and column).
//! [`AssetsLock::format`] writes the canonical form: entries sorted by
//! path, properties in the order above, two-space indent. Leading `//`
//! comment lines (license headers) are kept by
//! [`crate::write_assets_lock`].
//!
//! # License policy
//!
//! [`check_license`] decides, for one `license=` expression:
//!
//! - **error**: empty, not a valid SPDX expression (operators `AND`, `OR`,
//!   `WITH` in upper case, parentheses, ids of letters, digits, `.` and
//!   `-`), or `NONE` / `NOASSERTION` (no license is no permission);
//! - **warning**: valid, but uses an id outside [`ALLOWED_LICENSES`]
//!   (including `LicenseRef-...`, `+` and `WITH` exceptions). Such a room
//!   is valid locally but will not be accepted by the official store;
//! - **ok**: every id is in [`ALLOWED_LICENSES`].
//!
//! A missing `author=` is a warning for licenses that require attribution
//! (`CC-BY-*`, `OFL-1.1`, `MIT`, `Apache-2.0`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;
use std::path::PathBuf;

use kdl::{KdlDocument, KdlEntry, KdlNode, KdlValue};

use crate::error::{DocError, Location};
use crate::model::SourceLoc;
use crate::value::quote;

/// File name of the lock inside a `Name.room/` package.
pub const ASSETS_LOCK_FILE: &str = "assets.lock";

/// Directory of the asset files inside a `Name.room/` package.
pub const ASSETS_DIR: &str = "assets";

/// The newest `assets.lock` format this build reads and writes.
pub const ASSETS_LOCK_FORMAT: u32 = 1;

/// SPDX license ids accepted without a warning: the content licenses the
/// project recommends (`docs/licensing.md`) plus the code licenses of
/// `deny.toml`'s allow-list that suit assets.
pub const ALLOWED_LICENSES: [&str; 9] = [
    "CC0-1.0",
    "CC-BY-4.0",
    "CC-BY-SA-4.0",
    "OFL-1.1",
    "MIT",
    "Apache-2.0",
    "MPL-2.0",
    "GPL-3.0-or-later",
    "LGPL-3.0-or-later",
];

/// One file recorded in `assets.lock`.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetEntry {
    /// `assets/<name>`, relative to the package.
    pub path: String,
    /// SHA-256 of the contents, 64 lowercase hex digits.
    pub sha256: String,
    /// Size in bytes.
    pub size: u64,
    /// SPDX license expression.
    pub license: String,
    /// `"original"` or the URL the file came from.
    pub source: String,
    /// Author or attribution text.
    pub author: Option<String>,
    /// Location of the `asset` node.
    pub at: SourceLoc,
    /// Location of the `license=` property.
    pub license_at: SourceLoc,
}

impl AssetEntry {
    /// An entry with no location.
    #[must_use]
    pub fn new(
        path: impl Into<String>,
        sha256: impl Into<String>,
        size: u64,
        license: impl Into<String>,
        source: impl Into<String>,
        author: Option<String>,
    ) -> Self {
        AssetEntry {
            path: path.into(),
            sha256: sha256.into(),
            size,
            license: license.into(),
            source: source.into(),
            author,
            at: SourceLoc::default(),
            license_at: SourceLoc::default(),
        }
    }
}

/// The contents of `assets.lock`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssetsLock {
    pub assets: Vec<AssetEntry>,
}

/// How bad an [`AssetIssue`] is. Errors make `validate` fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

/// What is wrong with an asset.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IssueKind {
    /// The file's SHA-256 is not the recorded one.
    HashMismatch { expected: String, found: String },
    /// The file's size is not the recorded one.
    SizeMismatch { expected: u64, found: u64 },
    /// The lock lists a file that does not exist.
    MissingFile,
    /// A file in `assets/` is not listed in the lock.
    UnlistedFile,
    /// Something in `assets/` is not a regular file (a directory or link).
    NotAFile,
    /// The file could not be read.
    Unreadable(String),
    /// The license expression is not acceptable at all.
    InvalidLicense(String),
    /// The license expression uses ids outside [`ALLOWED_LICENSES`].
    UnlistedLicense(Vec<String>),
    /// The license requires attribution but `author=` is missing.
    MissingAttribution,
}

impl IssueKind {
    #[must_use]
    pub fn severity(&self) -> Severity {
        match self {
            IssueKind::UnlistedLicense(_) | IssueKind::MissingAttribution => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for IssueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IssueKind::HashMismatch { expected, found } => {
                write!(
                    f,
                    "sha256 mismatch: assets.lock has {expected}, file has {found}"
                )
            }
            IssueKind::SizeMismatch { expected, found } => write!(
                f,
                "size mismatch: assets.lock has {expected} bytes, file has {found}"
            ),
            IssueKind::MissingFile => f.write_str("listed in assets.lock but the file is missing"),
            IssueKind::UnlistedFile => f.write_str(
                "not listed in assets.lock (add it with `livspeil assets add` or remove it)",
            ),
            IssueKind::NotAFile => {
                f.write_str("not a regular file; assets/ may only hold plain files")
            }
            IssueKind::Unreadable(msg) => write!(f, "cannot read: {msg}"),
            IssueKind::InvalidLicense(msg) => write!(f, "invalid license: {msg}"),
            IssueKind::UnlistedLicense(ids) => write!(
                f,
                "license {} not in the allow-list ({}); the official store will not accept it",
                ids.join(", "),
                ALLOWED_LICENSES.join(", ")
            ),
            IssueKind::MissingAttribution => {
                f.write_str("the license requires attribution; add author=\"...\"")
            }
        }
    }
}

/// One problem found by [`verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetIssue {
    pub kind: IssueKind,
    /// The asset path (`assets/<name>`).
    pub path: String,
    /// Location in `assets.lock`, when the asset is listed there.
    pub at: Option<Location>,
}

impl AssetIssue {
    #[must_use]
    pub fn severity(&self) -> Severity {
        self.kind.severity()
    }
}

/// `line:col: severity: path: message`.
impl fmt::Display for AssetIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(at) = self.at {
            write!(f, "{at}: ")?;
        }
        write!(f, "{}: {}: {}", self.severity(), self.path, self.kind)
    }
}

/// Errors reading, writing or updating assets.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AssetsError {
    /// A file could not be read or written.
    Io { path: PathBuf, message: String },
    /// `assets.lock` is not valid (with line and column when known).
    Lock(DocError),
    /// A license expression was rejected.
    License { license: String, message: String },
    /// The asset is already listed with different data.
    Conflict { path: String, message: String },
}

impl fmt::Display for AssetsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetsError::Io { path, message } => write!(f, "{}: {message}", path.display()),
            AssetsError::Lock(err) => write!(f, "{err}"),
            AssetsError::License { license, message } => {
                write!(f, "license '{license}': {message}")
            }
            AssetsError::Conflict { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for AssetsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AssetsError::Lock(err) => Some(err),
            _ => None,
        }
    }
}

impl From<DocError> for AssetsError {
    fn from(err: DocError) -> Self {
        AssetsError::Lock(err)
    }
}

/// Facts about one file in `assets/`, gathered by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileFacts {
    /// A regular file with its SHA-256 (lowercase hex) and size.
    File { sha256: String, size: u64 },
    /// A directory, link or other non-regular entry.
    NotAFile,
    /// It could not be read.
    Unreadable(String),
}

impl AssetsLock {
    /// Parse the text of `assets.lock`.
    ///
    /// # Errors
    /// [`DocError::Syntax`] or [`DocError::Invalid`] with line and column.
    pub fn parse(text: &str) -> Result<AssetsLock, DocError> {
        let reader = Reader { text };
        let kdl = KdlDocument::parse_v2(text).map_err(|err| {
            let first = err.diagnostics.first();
            DocError::Syntax {
                at: reader.locate(first.map_or(0, |d| d.span.offset())),
                message: first.map_or_else(|| "invalid KDL".to_owned(), ToString::to_string),
            }
        })?;
        reader.lock(&kdl)
    }

    /// The canonical text (no comments). Entries are written sorted by path.
    #[must_use]
    pub fn format(&self) -> String {
        let mut assets: Vec<&AssetEntry> = self.assets.iter().collect();
        assets.sort_by(|a, b| a.path.cmp(&b.path));
        let mut out = format!("assets-lock format={ASSETS_LOCK_FORMAT}");
        if assets.is_empty() {
            out.push('\n');
            return out;
        }
        out.push_str(" {\n");
        for a in assets {
            let _ = write!(
                out,
                "  asset {} sha256={} size={} license={} source={}",
                quote(&a.path),
                quote(&a.sha256),
                a.size,
                quote(&a.license),
                quote(&a.source)
            );
            if let Some(author) = &a.author {
                let _ = write!(out, " author={}", quote(author));
            }
            out.push('\n');
        }
        out.push_str("}\n");
        out
    }

    /// The entry for `path`, if listed.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&AssetEntry> {
        self.assets.iter().find(|a| a.path == path)
    }
}

/// Whether `name` is a valid file name inside `assets/`.
#[must_use]
pub fn valid_asset_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn valid_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `source` is `"original"` or looks like a URL (`scheme://...`).
#[must_use]
pub fn valid_source(source: &str) -> bool {
    if source == "original" {
        return true;
    }
    match source.split_once("://") {
        Some((scheme, rest)) => {
            !scheme.is_empty()
                && scheme
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
                && !rest.is_empty()
                && !source.chars().any(char::is_whitespace)
        }
        None => false,
    }
}

const REQUIRED: [&str; 4] = ["sha256", "size", "license", "source"];
const ALLOWED_PROPS: [&str; 5] = ["sha256", "size", "license", "source", "author"];

struct Reader<'a> {
    text: &'a str,
}

impl Reader<'_> {
    fn locate(&self, offset: usize) -> Location {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let before = &self.text[..offset];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |i| i + 1);
        let column = before[line_start..].chars().count() + 1;
        Location { line, column }
    }

    fn at_node(&self, node: &KdlNode) -> Option<Location> {
        Some(self.locate(node.span().offset()))
    }

    fn err_node(&self, node: &KdlNode, message: impl Into<String>) -> DocError {
        DocError::invalid(self.at_node(node), message)
    }

    fn err_entry(&self, entry: &KdlEntry, message: impl Into<String>) -> DocError {
        DocError::invalid(Some(self.locate(entry.span().offset())), message)
    }

    fn no_annotation(&self, node: &KdlNode) -> Result<(), DocError> {
        match node.ty() {
            None => Ok(()),
            Some(ty) => Err(self.err_node(
                node,
                format!(
                    "unexpected type annotation ({}) on `{}`",
                    ty.value(),
                    node.name().value()
                ),
            )),
        }
    }

    /// Positional arguments and properties, duplicates and annotations
    /// rejected.
    #[allow(clippy::type_complexity)]
    fn entries<'n>(
        &self,
        node: &'n KdlNode,
    ) -> Result<(Vec<&'n KdlEntry>, BTreeMap<&'n str, &'n KdlEntry>), DocError> {
        let mut args = Vec::new();
        let mut props = BTreeMap::new();
        for entry in node.entries() {
            if let Some(ty) = entry.ty() {
                return Err(self.err_entry(
                    entry,
                    format!("unexpected type annotation ({})", ty.value()),
                ));
            }
            match entry.name() {
                None => args.push(entry),
                Some(key) => {
                    if props.insert(key.value(), entry).is_some() {
                        return Err(self.err_entry(
                            entry,
                            format!("property `{}` is given more than once", key.value()),
                        ));
                    }
                }
            }
        }
        Ok((args, props))
    }

    fn string(&self, entry: &KdlEntry, what: &str) -> Result<String, DocError> {
        match entry.value() {
            KdlValue::String(s) => Ok(s.clone()),
            _ => Err(self.err_entry(entry, format!("{what} must be a string"))),
        }
    }

    fn lock(&self, kdl: &KdlDocument) -> Result<AssetsLock, DocError> {
        let mut nodes = kdl.nodes().iter();
        let Some(head) = nodes.next() else {
            return Err(DocError::invalid(
                None,
                "empty assets.lock; expected `assets-lock format=1 { ... }`",
            ));
        };
        if head.name().value() != "assets-lock" {
            return Err(self.err_node(
                head,
                format!(
                    "expected the `assets-lock` header node, found `{}`",
                    head.name().value()
                ),
            ));
        }
        if let Some(extra) = nodes.next() {
            return Err(self.err_node(extra, "only one top-level node (`assets-lock`) is allowed"));
        }
        self.no_annotation(head)?;
        let (args, props) = self.entries(head)?;
        if let Some(arg) = args.first() {
            return Err(self.err_entry(arg, "`assets-lock` takes no arguments"));
        }
        if let Some((key, entry)) = props.iter().find(|(k, _)| **k != "format") {
            return Err(self.err_entry(entry, format!("unknown property `{key}`; expected format")));
        }
        let format = props
            .get("format")
            .ok_or_else(|| self.err_node(head, "assets-lock needs a format, e.g. `format=1`"))?;
        match format.value() {
            KdlValue::Integer(v) if *v == i128::from(ASSETS_LOCK_FORMAT) => {}
            KdlValue::Integer(v) => {
                return Err(self.err_entry(
                    format,
                    format!(
                        "assets.lock format {v} is not supported (this build reads format {ASSETS_LOCK_FORMAT})"
                    ),
                ));
            }
            _ => return Err(self.err_entry(format, "format must be an integer, e.g. `format=1`")),
        }

        let mut lock = AssetsLock::default();
        let mut paths = BTreeSet::new();
        for child in head.children().into_iter().flat_map(KdlDocument::nodes) {
            self.no_annotation(child)?;
            if child.name().value() != "asset" {
                return Err(self.err_node(
                    child,
                    format!(
                        "unknown node `{}` in assets-lock; expected `asset`",
                        child.name().value()
                    ),
                ));
            }
            let entry = self.asset(child)?;
            if !paths.insert(entry.path.clone()) {
                return Err(self.err_node(child, format!("duplicate asset '{}'", entry.path)));
            }
            lock.assets.push(entry);
        }
        Ok(lock)
    }

    fn asset(&self, node: &KdlNode) -> Result<AssetEntry, DocError> {
        if node.children().is_some() {
            return Err(self.err_node(node, "`asset` takes no children block"));
        }
        let (args, props) = self.entries(node)?;
        let path = match args.as_slice() {
            [arg] => self.string(arg, "the asset path")?,
            [] => {
                return Err(self.err_node(
                    node,
                    "`asset` needs a path, e.g. `asset \"assets/<sha256>.png\"`",
                ));
            }
            [_, extra, ..] => return Err(self.err_entry(extra, "`asset` takes exactly one path")),
        };
        let path_entry = args[0];
        match path.strip_prefix("assets/") {
            Some(name) if valid_asset_name(name) => {}
            _ => {
                return Err(self.err_entry(
                    path_entry,
                    format!(
                        "asset path '{path}' must be `assets/<name>` with a name of ASCII letters, digits, '.', '_' and '-' not starting with '.'"
                    ),
                ));
            }
        }
        if let Some((key, entry)) = props.iter().find(|(k, _)| !ALLOWED_PROPS.contains(k)) {
            return Err(self.err_entry(
                entry,
                format!(
                    "unknown property `{key}`; expected one of: {}",
                    ALLOWED_PROPS.join(", ")
                ),
            ));
        }
        if let Some(missing) = REQUIRED.iter().find(|k| !props.contains_key(*k)) {
            return Err(self.err_node(node, format!("asset '{path}' needs `{missing}=`")));
        }
        // REQUIRED keys are present (checked above); `get` keeps this panic-free.
        let get = |key: &str| {
            props
                .get(key)
                .copied()
                .ok_or_else(|| self.err_node(node, format!("asset '{path}' needs `{key}=`")))
        };

        let sha_entry = get("sha256")?;
        let sha256 = self.string(sha_entry, "sha256")?;
        if !valid_sha256(&sha256) {
            return Err(self.err_entry(sha_entry, "sha256 must be 64 lowercase hex digits"));
        }
        let size_entry = get("size")?;
        let size = match size_entry.value() {
            KdlValue::Integer(v) => u64::try_from(*v)
                .map_err(|_| self.err_entry(size_entry, "size must be a non-negative integer"))?,
            _ => return Err(self.err_entry(size_entry, "size must be an integer (bytes)")),
        };
        let license_entry = get("license")?;
        let license = self.string(license_entry, "license")?;
        let source_entry = get("source")?;
        let source = self.string(source_entry, "source")?;
        if !valid_source(&source) {
            return Err(self.err_entry(
                source_entry,
                format!("source '{source}' must be \"original\" or a URL such as \"https://...\""),
            ));
        }
        let author = match props.get("author") {
            None => None,
            Some(entry) => {
                let author = self.string(entry, "author")?;
                if author.trim().is_empty() {
                    return Err(
                        self.err_entry(entry, "author must not be empty; leave it out instead")
                    );
                }
                Some(author)
            }
        };
        Ok(AssetEntry {
            path,
            sha256,
            size,
            license,
            source,
            author,
            at: SourceLoc(self.at_node(node)),
            license_at: SourceLoc(Some(self.locate(license_entry.span().offset()))),
        })
    }
}

/// Licenses (by id prefix) that oblige users to credit the author.
fn needs_attribution(id: &str) -> bool {
    id.starts_with("CC-BY") || matches!(id, "OFL-1.1" | "MIT" | "Apache-2.0")
}

/// Check one SPDX license expression against the policy in the module
/// docs. `Ok` holds the ids outside [`ALLOWED_LICENSES`] (empty if the
/// expression is fully allowed).
///
/// # Errors
/// A reason if the expression is empty, malformed, `NONE` or
/// `NOASSERTION`.
pub fn check_license(expr: &str) -> Result<Vec<String>, String> {
    let ids = license_ids(expr)?;
    let mut outside = Vec::new();
    for id in ids {
        if matches!(id.as_str(), "NONE" | "NOASSERTION") {
            return Err(format!("{id} is not a license; every asset needs one"));
        }
        if !ALLOWED_LICENSES.contains(&id.as_str()) && !outside.contains(&id) {
            outside.push(id);
        }
    }
    Ok(outside)
}

/// The license and exception ids of an SPDX expression (`+` and `WITH
/// <exception>` are kept on the id), after checking the syntax.
fn license_ids(expr: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    for c in expr.chars() {
        if c == '(' || c == ')' || c.is_whitespace() {
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
            if !c.is_whitespace() {
                tokens.push(c.to_string());
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    if tokens.is_empty() {
        return Err("empty license expression".into());
    }
    let mut parser = SpdxParser {
        tokens: &tokens,
        pos: 0,
        ids: Vec::new(),
    };
    parser.expr()?;
    if let Some(extra) = tokens.get(parser.pos) {
        let upper = extra.to_ascii_uppercase();
        if matches!(upper.as_str(), "AND" | "OR" | "WITH") {
            return Err(format!("operators are written in upper case: '{upper}'"));
        }
        return Err(format!(
            "unexpected '{extra}'; join licenses with AND, OR or WITH"
        ));
    }
    Ok(parser.ids)
}

struct SpdxParser<'t> {
    tokens: &'t [String],
    pos: usize,
    ids: Vec<String>,
}

impl SpdxParser<'_> {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.pos).map(String::as_str)
    }

    fn expr(&mut self) -> Result<(), String> {
        self.and()?;
        while self.peek() == Some("OR") {
            self.pos += 1;
            self.and()?;
        }
        Ok(())
    }

    fn and(&mut self) -> Result<(), String> {
        self.atom()?;
        while self.peek() == Some("AND") {
            self.pos += 1;
            self.atom()?;
        }
        Ok(())
    }

    fn atom(&mut self) -> Result<(), String> {
        match self.peek() {
            None => Err("expression ends early; expected a license id".into()),
            Some("(") => {
                self.pos += 1;
                self.expr()?;
                if self.peek() != Some(")") {
                    return Err("missing ')'".into());
                }
                self.pos += 1;
                Ok(())
            }
            Some(token) => {
                let token = token.to_owned();
                let id = Self::id(&token)?;
                self.pos += 1;
                if self.peek() == Some("WITH") {
                    self.pos += 1;
                    let exception = match self.peek() {
                        Some(t) => Self::id(t)?,
                        None => return Err("WITH needs an exception id".into()),
                    };
                    self.pos += 1;
                    self.ids.push(format!("{id} WITH {exception}"));
                } else {
                    self.ids.push(id);
                }
                Ok(())
            }
        }
    }

    fn id(token: &str) -> Result<String, String> {
        if matches!(token, "AND" | "OR" | "WITH" | ")") {
            return Err(format!("expected a license id, found '{token}'"));
        }
        if matches!(token.to_ascii_uppercase().as_str(), "AND" | "OR" | "WITH") {
            return Err(format!(
                "operators are written in upper case: '{}'",
                token.to_ascii_uppercase()
            ));
        }
        let base = token.strip_suffix('+').unwrap_or(token);
        let ok = !base.is_empty()
            && base
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'));
        if ok {
            Ok(token.to_owned())
        } else {
            Err(format!("'{token}' is not an SPDX license id"))
        }
    }
}

/// Check `lock` against the files actually in `assets/` (by name, without
/// the `assets/` prefix). Issues come in a deterministic order: lock
/// entries by path (license, then file checks), then unlisted files.
#[must_use]
pub fn verify(lock: &AssetsLock, files: &BTreeMap<String, FileFacts>) -> Vec<AssetIssue> {
    let mut issues = Vec::new();
    let mut entries: Vec<&AssetEntry> = lock.assets.iter().collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    for entry in &entries {
        let issue = |kind, at: SourceLoc| AssetIssue {
            kind,
            path: entry.path.clone(),
            at: at.0,
        };
        match check_license(&entry.license) {
            Err(msg) => issues.push(issue(IssueKind::InvalidLicense(msg), entry.license_at)),
            Ok(outside) => {
                if !outside.is_empty() {
                    issues.push(issue(IssueKind::UnlistedLicense(outside), entry.license_at));
                }
                let ids = license_ids(&entry.license).unwrap_or_default();
                if entry.author.is_none() && ids.iter().any(|id| needs_attribution(id)) {
                    issues.push(issue(IssueKind::MissingAttribution, entry.at));
                }
            }
        }
        let name = entry.path.strip_prefix("assets/").unwrap_or(&entry.path);
        match files.get(name) {
            None => issues.push(issue(IssueKind::MissingFile, entry.at)),
            Some(FileFacts::NotAFile) => issues.push(issue(IssueKind::NotAFile, entry.at)),
            Some(FileFacts::Unreadable(msg)) => {
                issues.push(issue(IssueKind::Unreadable(msg.clone()), entry.at));
            }
            Some(FileFacts::File { sha256, size }) => {
                if *size != entry.size {
                    issues.push(issue(
                        IssueKind::SizeMismatch {
                            expected: entry.size,
                            found: *size,
                        },
                        entry.at,
                    ));
                }
                if *sha256 != entry.sha256 {
                    issues.push(issue(
                        IssueKind::HashMismatch {
                            expected: entry.sha256.clone(),
                            found: sha256.clone(),
                        },
                        entry.at,
                    ));
                }
            }
        }
    }
    let listed: BTreeSet<&str> = entries
        .iter()
        .map(|e| e.path.strip_prefix("assets/").unwrap_or(&e.path))
        .collect();
    for (name, facts) in files {
        if listed.contains(name.as_str()) {
            continue;
        }
        let kind = match facts {
            FileFacts::NotAFile => IssueKind::NotAFile,
            _ => IssueKind::UnlistedFile,
        };
        issues.push(AssetIssue {
            kind,
            path: format!("{ASSETS_DIR}/{name}"),
            at: None,
        });
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_policy() {
        assert_eq!(check_license("CC0-1.0"), Ok(vec![]));
        assert_eq!(check_license("MIT OR Apache-2.0"), Ok(vec![]));
        assert_eq!(check_license("(MIT AND CC-BY-4.0) OR CC0-1.0"), Ok(vec![]));
        assert_eq!(check_license("CC-BY-3.0"), Ok(vec!["CC-BY-3.0".into()]));
        assert_eq!(
            check_license("GPL-2.0+ WITH Classpath-exception-2.0"),
            Ok(vec!["GPL-2.0+ WITH Classpath-exception-2.0".into()])
        );
        assert_eq!(
            check_license("LicenseRef-mine"),
            Ok(vec!["LicenseRef-mine".into()])
        );
        for bad in [
            "",
            "  ",
            "MIT or CC0-1.0",
            "MIT AND",
            "(MIT",
            "MIT)",
            "AND MIT",
            "M!T",
            "NONE",
            "MIT OR NOASSERTION",
            "MIT CC0-1.0",
        ] {
            assert!(check_license(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn sources() {
        assert!(valid_source("original"));
        assert!(valid_source("https://example.org/a.png"));
        assert!(!valid_source("Original"));
        assert!(!valid_source("example.org"));
        assert!(!valid_source("https://"));
        assert!(!valid_source("https://a b"));
    }

    #[test]
    fn names() {
        assert!(valid_asset_name("ab12.png"));
        assert!(!valid_asset_name(".hidden"));
        assert!(!valid_asset_name("a/b"));
        assert!(!valid_asset_name(""));
    }
}
