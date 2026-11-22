// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `assets.lock`: round trip, strict parsing with locations, and
//! verification of a package's `assets/` against it.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use livspeil_doc::assets::{FileFacts, verify};
use livspeil_doc::{
    AssetEntry, AssetsError, AssetsLock, IssueKind, Location, Severity, add_asset, check_assets,
    read_assets_lock, sha256_file, write_assets_lock,
};

/// SHA-256 of "hello world\n".
const HELLO_SHA: &str = "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447";

const LOCK: &str = r#"assets-lock format=1 {
  asset "assets/a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447.txt" sha256="a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447" size=12 license="CC0-1.0" source="original"
  asset "assets/b.png" sha256="0000000000000000000000000000000000000000000000000000000000000000" size=3 license="CC-BY-4.0" source="https://example.org/b.png" author="Jane \"J\" Doe"
}
"#;

fn hello_path() -> String {
    format!("assets/{HELLO_SHA}.txt")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("livspeil-doc-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn err_at(text: &str) -> (Location, String) {
    let err = AssetsLock::parse(text).unwrap_err();
    (err.location().expect("location"), err.to_string())
}

#[test]
fn round_trip_is_canonical_and_sorted() {
    let lock = AssetsLock::parse(LOCK).unwrap();
    assert_eq!(lock.assets.len(), 2);
    let b = lock.get("assets/b.png").unwrap();
    assert_eq!(b.author.as_deref(), Some("Jane \"J\" Doe"));
    assert_eq!(b.size, 3);
    assert_eq!(lock.format(), LOCK);
    assert_eq!(AssetsLock::parse(&lock.format()).unwrap(), lock);

    // Entries are written sorted by path whatever the input order.
    let mut reversed = lock.clone();
    reversed.assets.reverse();
    assert_eq!(reversed.format(), LOCK);

    assert_eq!(AssetsLock::default().format(), "assets-lock format=1\n");
    assert_eq!(
        AssetsLock::parse("assets-lock format=1\n").unwrap(),
        AssetsLock::default()
    );
}

#[test]
fn lenient_spacing_and_comments_parse() {
    let text = "// header\nassets-lock format=1 {\n  // a comment\n  asset \"assets/x\" source=\"original\" license=\"MIT\" size=0 sha256=\"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\" author=\"A\"\n}";
    let lock = AssetsLock::parse(text).unwrap();
    assert_eq!(lock.assets[0].path, "assets/x");
}

#[test]
fn parse_errors_have_line_and_column() {
    let cases: [(&str, (usize, usize), &str); 11] = [
        (
            "assets format=1",
            (1, 1),
            "expected the `assets-lock` header",
        ),
        ("assets-lock", (1, 1), "needs a format"),
        ("assets-lock format=2", (1, 13), "format 2 is not supported"),
        (
            "assets-lock format=1 {\n  file \"x\"\n}",
            (2, 3),
            "unknown node `file`",
        ),
        (
            "assets-lock format=1 {\n  asset \"assets/x\" sha256=\"00\" size=1 license=\"MIT\" source=\"original\"\n}",
            (2, 20),
            "64 lowercase hex",
        ),
        (
            "assets-lock format=1 {\n  asset \"x.png\" sha256=\"00\"\n}",
            (2, 9),
            "must be `assets/<name>`",
        ),
        (
            "assets-lock format=1 {\n  asset \"assets/../x\"\n}",
            (2, 9),
            "must be `assets/<name>`",
        ),
        (
            "assets-lock format=1 {\n  asset \"assets/x\" size=1\n}",
            (2, 3),
            "needs `sha256=`",
        ),
        (
            "assets-lock format=1 {\n  asset \"assets/x\" sha256=\"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\" size=-1 license=\"MIT\" source=\"original\"\n}",
            (2, 94),
            "non-negative",
        ),
        (
            "assets-lock format=1 {\n  asset \"assets/x\" sha256=\"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\" size=1 license=\"MIT\" source=\"example.org\"\n}",
            (2, 115),
            "must be \"original\" or a URL",
        ),
        (
            "assets-lock format=1 {\n  asset \"assets/x\" sha256=\"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\" size=1 license=\"MIT\" source=\"original\" colour=\"red\"\n}",
            (2, 133),
            "unknown property `colour`",
        ),
    ];
    for (text, (line, column), needle) in cases {
        let (at, message) = err_at(text);
        assert_eq!(at, Location { line, column }, "{text}\n{message}");
        assert!(message.contains(needle), "{message} lacks {needle}");
    }

    let dup = format!(
        "assets-lock format=1 {{\n{}{}}}",
        LOCK.lines().nth(1).unwrap().to_owned() + "\n",
        LOCK.lines().nth(1).unwrap().to_owned() + "\n"
    );
    let (at, message) = err_at(&dup);
    assert_eq!(at, Location { line: 3, column: 3 });
    assert!(message.contains("duplicate asset"));

    let err = AssetsLock::parse("assets-lock format=1 {").unwrap_err();
    assert!(err.to_string().contains("syntax error"), "{err}");
}

fn facts(sha: &str, size: u64) -> FileFacts {
    FileFacts::File {
        sha256: sha.to_owned(),
        size,
    }
}

#[test]
fn verify_reports_each_problem_with_its_location() {
    let lock = AssetsLock::parse(LOCK).unwrap();
    let mut files = std::collections::BTreeMap::new();
    // Hash and size wrong for the .txt; b.png missing; an extra file.
    files.insert(format!("{HELLO_SHA}.txt"), facts(&"1".repeat(64), 13));
    files.insert("extra.bin".to_owned(), facts(HELLO_SHA, 12));
    files.insert("sub".to_owned(), FileFacts::NotAFile);
    let issues = verify(&lock, &files);
    let summary: Vec<(String, Option<Location>, Severity)> = issues
        .iter()
        .map(|i| {
            (
                format!("{} {}", i.path, kind_name(&i.kind)),
                i.at,
                i.severity(),
            )
        })
        .collect();
    let at = |line, column| Some(Location { line, column });
    assert_eq!(
        summary,
        [
            (format!("{} size", hello_path()), at(2, 3), Severity::Error),
            (format!("{} hash", hello_path()), at(2, 3), Severity::Error),
            ("assets/b.png missing".to_owned(), at(3, 3), Severity::Error),
            (
                "assets/extra.bin unlisted".to_owned(),
                None,
                Severity::Error
            ),
            ("assets/sub not-a-file".to_owned(), None, Severity::Error),
        ]
    );
    let text = issues[1].to_string();
    assert!(text.starts_with("2:3: error: assets/a948"), "{text}");
    assert!(text.contains("sha256 mismatch"), "{text}");
}

fn kind_name(kind: &IssueKind) -> &'static str {
    match kind {
        IssueKind::HashMismatch { .. } => "hash",
        IssueKind::SizeMismatch { .. } => "size",
        IssueKind::MissingFile => "missing",
        IssueKind::UnlistedFile => "unlisted",
        IssueKind::NotAFile => "not-a-file",
        IssueKind::Unreadable(_) => "unreadable",
        IssueKind::InvalidLicense(_) => "bad-license",
        IssueKind::UnlistedLicense(_) => "unlisted-license",
        IssueKind::MissingAttribution => "attribution",
        _ => "other",
    }
}

#[test]
fn license_problems_point_at_the_license_property() {
    let base = LOCK.lines().nth(1).unwrap();
    let col = base.find("license=").unwrap() + 1;
    let files: std::collections::BTreeMap<String, FileFacts> =
        [(format!("{HELLO_SHA}.txt"), facts(HELLO_SHA, 12))].into();
    for (license, expected, severity) in [
        ("MIT or CC0-1.0", "bad-license", Severity::Error),
        ("NOASSERTION", "bad-license", Severity::Error),
        ("", "bad-license", Severity::Error),
        ("LicenseRef-custom", "unlisted-license", Severity::Warning),
        ("CC-BY-SA-4.0", "attribution", Severity::Warning),
    ] {
        let line = base.replace("\"CC0-1.0\"", &format!("{license:?}"));
        let text = format!("assets-lock format=1 {{\n{line}\n}}\n");
        let lock = AssetsLock::parse(&text).unwrap();
        let issues = verify(&lock, &files);
        assert_eq!(issues.len(), 1, "{license}: {issues:?}");
        assert_eq!(kind_name(&issues[0].kind), expected, "{license}");
        assert_eq!(issues[0].severity(), severity);
        let at = issues[0].at.unwrap();
        assert_eq!(at.line, 2);
        if expected != "attribution" {
            assert_eq!(at.column, col, "{license}");
        }
    }
    // A clean entry has no issues.
    let lock = AssetsLock::parse(&format!("assets-lock format=1 {{\n{base}\n}}\n")).unwrap();
    assert!(verify(&lock, &files).is_empty());
}

#[test]
fn package_check_add_and_write_preserve_prolog() {
    let tmp = TempDir::new("assets");
    let room = tmp.path().join("R.room");
    fs::create_dir_all(&room).unwrap();
    // Nothing yet: no assets at all.
    assert_eq!(check_assets(&room).unwrap(), None);

    let src = tmp.path().join("Hello.TXT");
    fs::write(&src, "hello world\n").unwrap();
    assert_eq!(sha256_file(&src).unwrap(), (HELLO_SHA.to_owned(), 12));

    fs::write(
        room.join("assets.lock"),
        "// License header   \n\n\nassets-lock format=1\n",
    )
    .unwrap();
    let added = add_asset(&room, &src, "CC0-1.0", "original", None).unwrap();
    assert!(added.added);
    assert_eq!(added.entry.path, hello_path());
    assert!(room.join(hello_path()).is_file());
    let text = fs::read_to_string(room.join("assets.lock")).unwrap();
    assert!(text.starts_with("// License header\n\nassets-lock format=1 {\n"));
    let (lock, issues) = check_assets(&room).unwrap().unwrap();
    assert_eq!(lock.assets.len(), 1);
    assert!(issues.is_empty(), "{issues:?}");

    // Adding again is a no-op; with other data it is a conflict.
    let again = add_asset(&room, &src, "CC0-1.0", "original", None).unwrap();
    assert!(!again.added);
    assert!(matches!(
        add_asset(&room, &src, "MIT", "original", None),
        Err(AssetsError::Conflict { .. })
    ));
    assert!(matches!(
        add_asset(&room, &src, "MIT AND", "original", None),
        Err(AssetsError::License { .. })
    ));
    let warn = tmp.path().join("w.bin");
    fs::write(&warn, "x").unwrap();
    let added = add_asset(&room, &warn, "CC-BY-3.0", "https://e.org/w", Some("A")).unwrap();
    assert_eq!(added.unlisted_licenses, ["CC-BY-3.0"]);

    // Tamper with the file: hash and size mismatch.
    fs::write(room.join(hello_path()), "hello world!\n").unwrap();
    // An unlisted file.
    fs::write(room.join("assets/stray.txt"), "?").unwrap();
    let (_, issues) = check_assets(&room).unwrap().unwrap();
    let kinds: Vec<&str> = issues.iter().map(|i| kind_name(&i.kind)).collect();
    assert_eq!(kinds, ["unlisted-license", "size", "hash", "unlisted"]);

    // Removing the file: missing.
    fs::remove_file(room.join(hello_path())).unwrap();
    let (_, issues) = check_assets(&room).unwrap().unwrap();
    let missing = issues
        .iter()
        .find(|i| kind_name(&i.kind) == "missing")
        .unwrap();
    // Line 5: prolog, blank line, header, then the entries sorted by path.
    assert_eq!(missing.at, Some(Location { line: 5, column: 3 }));

    // write_assets_lock round trip.
    let (lock, _) = read_assets_lock(&room).unwrap().unwrap();
    write_assets_lock(&room, &lock).unwrap();
    let rewritten = fs::read_to_string(room.join("assets.lock")).unwrap();
    assert_eq!(AssetsLock::parse(&rewritten).unwrap(), lock);
    assert!(rewritten.starts_with("// License header\n"));

    // A broken lock is a typed error with its location.
    fs::write(room.join("assets.lock"), "assets-lock format=1 {\n  x\n}\n").unwrap();
    match check_assets(&room) {
        Err(AssetsError::Lock(err)) => {
            assert_eq!(err.location(), Some(Location { line: 2, column: 3 }));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn assets_without_lock_are_unlisted() {
    let tmp = TempDir::new("nolock");
    fs::create_dir_all(tmp.path().join("assets")).unwrap();
    fs::write(tmp.path().join("assets/a.txt"), "a").unwrap();
    let (lock, issues) = check_assets(tmp.path()).unwrap().unwrap();
    assert!(lock.assets.is_empty());
    assert_eq!(issues.len(), 1);
    assert!(matches!(issues[0].kind, IssueKind::UnlistedFile));
}

#[test]
fn entry_constructor_formats() {
    let lock = AssetsLock {
        assets: vec![AssetEntry::new(
            "assets/x",
            HELLO_SHA,
            12,
            "MIT",
            "original",
            Some("Me".into()),
        )],
    };
    assert!(lock.format().contains(
        "asset \"assets/x\" sha256=\"a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447\" size=12 license=\"MIT\" source=\"original\" author=\"Me\"\n"
    ));
}
