// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! motion.toml / theme.toml parsing and hot reload.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use livspeil_testkit::assert_golden;
use livspeil_ui_theme::{
    Clock, Color, ConfigEvent, ConfigFs, ConfigKind, ConfigWatcher, Density, MotionClass,
    MotionConfig, MotionProfile, MotionSpec, PropertyKind, Scheme, SpringSpec, Theme, Tier,
};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

const SPEC_EXAMPLE: &str = r#"
# ~/.config/livspeil/ui/motion.toml
profile = "snappy"          # snappy | fluid | calm | off
speed = 1.0                 # global multiplier; 0 = instant
reduce_motion = false       # replaces movement with fades

[profiles.snappy]
micro    = { response = 0.10, damping = 0.90 }   # hover, press, toggle
standard = { response = 0.16, damping = 0.86 }   # menus, popovers, selection
large    = { response = 0.26, damping = 0.82 }   # panels, window tokens, camera moves

[overrides.windows]
speed = 1.2
"#;

#[test]
fn spec_example_parses_exactly() {
    let config = MotionConfig::parse(SPEC_EXAMPLE).unwrap();
    assert_eq!(config.profile, "snappy");
    assert_eq!(config.speed, 1.0);
    assert!(!config.reduce_motion);
    assert_eq!(config.active_profile(), MotionProfile::SNAPPY);
    assert_eq!(config.speed(Some("windows")), 1.2);
    assert_eq!(config.speed(Some("menus")), 1.0);
    // Defaults equal the spec example.
    let mut defaults = MotionConfig::default();
    defaults.overrides = config.overrides.clone();
    assert_eq!(defaults, config);
}

#[test]
fn empty_file_is_defaults() {
    assert_eq!(MotionConfig::parse("").unwrap(), MotionConfig::default());
    assert_eq!(Theme::parse("").unwrap(), Theme::default());
}

#[test]
fn resolve_applies_speed_scope_and_reduce_motion() {
    let config = MotionConfig::parse(SPEC_EXAMPLE).unwrap();
    let spring = |s: MotionSpec| match s {
        MotionSpec::Spring(s) => s,
        MotionSpec::Instant => panic!("expected spring"),
    };
    let micro = spring(config.resolve(None, MotionClass::Micro, PropertyKind::Movement));
    assert_eq!(micro, SpringSpec::new(0.10, 0.90));
    let windows = spring(config.resolve(Some("windows"), MotionClass::Large, PropertyKind::Movement));
    assert!((windows.response - 0.26 / 1.2).abs() < 1e-6);

    let zero = MotionConfig::parse("speed = 0").unwrap();
    assert_eq!(
        zero.resolve(None, MotionClass::Micro, PropertyKind::Appearance),
        MotionSpec::Instant
    );
    let off = MotionConfig::parse("profile = \"off\"").unwrap();
    assert_eq!(
        off.resolve(None, MotionClass::Large, PropertyKind::Movement),
        MotionSpec::Instant
    );

    let reduce = MotionConfig::parse("reduce_motion = true").unwrap();
    assert_eq!(
        reduce.resolve(None, MotionClass::Standard, PropertyKind::Movement),
        MotionSpec::Instant
    );
    // Appearance takes the Calm path: same spring as the Calm profile.
    let calm = MotionConfig::parse("profile = \"calm\"").unwrap();
    assert_eq!(
        reduce.resolve(None, MotionClass::Standard, PropertyKind::Appearance),
        calm.resolve(None, MotionClass::Standard, PropertyKind::Appearance)
    );
    let s = spring(reduce.resolve(None, MotionClass::Micro, PropertyKind::Appearance));
    assert!(s.damping >= 1.0);

    let scoped = MotionConfig::parse(
        "[overrides.shell]\nreduce_motion = true\nprofile = \"fluid\"\nspeed = 0.5\n",
    )
    .unwrap();
    assert_eq!(
        scoped.resolve(Some("shell"), MotionClass::Micro, PropertyKind::Movement),
        MotionSpec::Instant
    );
    assert_eq!(
        scoped.resolve(None, MotionClass::Micro, PropertyKind::Movement),
        MotionSpec::Spring(MotionProfile::SNAPPY.micro)
    );
}

#[test]
fn custom_and_partial_profiles() {
    let config = MotionConfig::parse(
        r#"
profile = "mine"
[profiles.mine]
micro = { response = 0.05, damping = 1.0 }
standard = { response = 0.1, damping = 1.0 }
large = { response = 0.2, damping = 1 }
[profiles.fluid]
micro = { response = 0.2, damping = 0.7 }
"#,
    )
    .unwrap();
    assert_eq!(config.active_profile().large, SpringSpec::new(0.2, 1.0));
    let fluid = config.profiles["fluid"];
    assert_eq!(fluid.micro, SpringSpec::new(0.2, 0.7));
    assert_eq!(fluid.standard, MotionProfile::FLUID.standard);
}

#[test]
fn motion_errors_have_locations() {
    let cases = [
        ("unknown-key", "profile = \"snappy\"\nsped = 2\n"),
        ("wrong-type", "speed = \"fast\"\n"),
        ("unknown-profile", "speed = 1.0\nprofile = \"bouncy\"\n"),
        ("negative-speed", "speed = -1\n"),
        (
            "bad-damping",
            "[profiles.snappy]\nmicro = { response = 0.1, damping = 0 }\n",
        ),
        (
            "custom-incomplete",
            "[profiles.mine]\nmicro = { response = 0.1, damping = 1 }\n",
        ),
        (
            "spring-unknown-field",
            "[profiles.calm]\nmicro = { response = 0.1, damping = 1, mass = 2 }\n",
        ),
        ("override-profile", "[overrides.windows]\nprofile = \"nope\"\n"),
        ("syntax", "profile = \n"),
    ];
    let mut out = String::new();
    for (name, src) in cases {
        let err = MotionConfig::parse(src).unwrap_err();
        assert!(err.line > 0, "{name}: no location: {err}");
        out.push_str(&format!("{name}: {err}\n"));
    }
    assert_golden(&golden_dir(), "motion-errors", &out);
}

#[test]
fn theme_overrides_tokens() {
    let theme = Theme::parse(
        r##"
scheme = "light"
density = "pro"
tier = "T1"
reduce_transparency = true
spacing = [1, 2, 4, 6, 8, 12, 16]
icon_sizes = [12, 14, 18, 22]
[radii]
md = 9
[metrics.pro]
row_height = 22
[elevation]
e2 = ["0 6 16 / 0.3", "0 1 3 1 / 0.2"]
e1 = ["0 1 1"]
[colors.light]
accent = "oklch(0.6 0.2 300)"
"surface.0" = "#ffffff"
"##,
    )
    .unwrap();
    assert_eq!(theme.scheme, Scheme::Light);
    assert_eq!(theme.density, Density::Pro);
    assert_eq!(theme.tier, Tier::T1);
    assert!(theme.reduce_transparency);
    assert_eq!(theme.tokens.spacing.step(6), 16.0);
    assert_eq!(theme.tokens.radii.md, 9.0);
    assert_eq!(theme.tokens.radii.lg, 14.0);
    assert_eq!(theme.metrics().row_height, 22.0);
    assert_eq!(theme.metrics().body, 12.0);
    assert_eq!(theme.tokens.elevations[1].key.y, 6.0);
    assert_eq!(theme.tokens.elevations[1].contact.unwrap().spread, 1.0);
    assert!(theme.tokens.elevations[0].contact.is_none());
    assert_eq!(theme.colors().accent, Color::oklch(0.6, 0.2, 300.0, 1.0));
    assert!((theme.colors().surface[0].l - 1.0).abs() < 1e-3);
}

#[test]
fn theme_errors_have_locations() {
    let cases = [
        ("unknown-key", "colour = 1\n"),
        ("scheme", "scheme = \"sepia\"\n"),
        ("spacing-length", "spacing = [1, 2, 3]\n"),
        ("negative-radius", "[radii]\nsm = -2\n"),
        ("color-role", "[colors.dark]\nprimary = \"#fff\"\n"),
        ("color-value", "[colors.dark]\naccent = \"oklch(2 0 0)\"\n"),
        ("shadow", "[elevation]\ne3 = [\"0 x 4\"]\n"),
        ("elevation-name", "[elevation]\ne9 = [\"0 1 2\"]\n"),
        ("tier", "tier = \"t3\"\n"),
    ];
    let mut out = String::new();
    for (name, src) in cases {
        let err = Theme::parse(src).unwrap_err();
        assert!(err.line > 0, "{name}: no location: {err}");
        out.push_str(&format!("{name}: {err}\n"));
    }
    assert_golden(&golden_dir(), "theme-errors", &out);
}

// ---- hot reload -------------------------------------------------------

#[derive(Clone, Default)]
struct MemFs {
    files: Rc<RefCell<HashMap<PathBuf, (u64, String)>>>,
    reads: Rc<Cell<usize>>,
    stats: Rc<Cell<usize>>,
}

impl MemFs {
    fn write(&self, path: &str, mtime: u64, text: &str) {
        self.files
            .borrow_mut()
            .insert(PathBuf::from(path), (mtime, text.to_owned()));
    }
    fn remove(&self, path: &str) {
        self.files.borrow_mut().remove(Path::new(path));
    }
}

impl ConfigFs for MemFs {
    fn modified(&self, path: &Path) -> io::Result<Option<SystemTime>> {
        self.stats.set(self.stats.get() + 1);
        Ok(self
            .files
            .borrow()
            .get(path)
            .map(|(m, _)| SystemTime::UNIX_EPOCH + Duration::from_secs(*m)))
    }
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        self.reads.set(self.reads.get() + 1);
        self.files
            .borrow()
            .get(path)
            .map(|(_, t)| t.clone())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

#[derive(Clone, Default)]
struct ManualClock(Rc<Cell<Duration>>);

impl ManualClock {
    fn advance(&self, ms: u64) {
        self.0.set(self.0.get() + Duration::from_millis(ms));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

#[test]
fn hot_reload_with_injected_clock() {
    let fs = MemFs::default();
    let clock = ManualClock::default();
    let mut w = ConfigWatcher::with(fs.clone(), clock.clone(), Duration::from_millis(500));
    w.watch_dir(Path::new("/cfg"));
    w.watch("/cfg/keymap.toml", ConfigKind::Raw("keymap"));

    // Nothing exists: defaults, no events.
    assert!(w.poll().is_empty());

    // A file appears, but the interval has not passed: not even stat'ed.
    fs.write("/cfg/motion.toml", 1, "speed = 2.0");
    let stats = fs.stats.get();
    clock.advance(100);
    assert!(w.poll().is_empty());
    assert_eq!(fs.stats.get(), stats);

    clock.advance(400);
    let events = w.poll();
    assert_eq!(events.len(), 1);
    match &events[0] {
        ConfigEvent::Motion(m) => assert_eq!(m.speed, 2.0),
        other => panic!("unexpected {other:?}"),
    }

    // Touch without content change: no event.
    fs.write("/cfg/motion.toml", 2, "speed = 2.0");
    clock.advance(500);
    assert!(w.poll().is_empty());

    // Broken edit: error event with location; nothing else.
    fs.write("/cfg/motion.toml", 3, "speed = 2.0\nprofile = \"zippy\"\n");
    clock.advance(500);
    let events = w.poll();
    match events.as_slice() {
        [ConfigEvent::Error { kind, error }] => {
            assert_eq!(*kind, ConfigKind::Motion);
            assert_eq!((error.line, error.column), (2, 11));
            assert_eq!(error.file, "motion.toml");
        }
        other => panic!("unexpected {other:?}"),
    }

    // Theme and raw file change in the same poll.
    fs.write("/cfg/theme.toml", 4, "density = \"pro\"");
    fs.write("/cfg/keymap.toml", 4, "[bindings]\n");
    clock.advance(500);
    let events = w.poll();
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], ConfigEvent::Theme(t) if t.density == Density::Pro));
    assert!(matches!(&events[1], ConfigEvent::Raw { tag: "keymap", contents: Some(_) }));

    // Removal reverts to defaults.
    fs.remove("/cfg/motion.toml");
    fs.remove("/cfg/keymap.toml");
    clock.advance(500);
    let events = w.poll();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0], ConfigEvent::Motion(MotionConfig::default()));
    assert_eq!(
        events[1],
        ConfigEvent::Raw {
            tag: "keymap",
            contents: None
        }
    );
    clock.advance(500);
    assert!(w.poll().is_empty());
}
