// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The window, headless: selecting, the transport and golden screenshots.
//! `UPDATE_GOLDEN=1` accepts new screenshots; look at them.

use std::path::PathBuf;

use livspeil_dream::app::{Mounted, mount};
use livspeil_dream::session::Session;
use livspeil_node_api::Value;
use livspeil_ui_core::{App, ElementId, TestHost, WindowRequest};
use livspeil_ui_layout::{Point, Size};
use livspeil_ui_paint::{FontSystem as PaintFonts, PaintBridge};
use livspeil_ui_render::{AtlasStore, Color, DisplayList, FrameOptions, encode, render_cpu_with};
use livspeil_ui_shot::{Tolerance, compare, read_png, shots_dir, write_png};
use livspeil_ui_text::FontSystem;
use livspeil_ui_widgets::Env;

const SIZE: Size = Size::new(1280.0, 720.0);

struct Fx {
    h: TestHost,
    bridge: PaintBridge,
    /// The bridge sends each glyph once: keep what it sent.
    atlases: AtlasStore,
    m: Mounted,
}

fn open(room: &str) -> Fx {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(room);
    // `join` keeps an absolute `room` as it is.
    let session = Session::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let bridge = PaintBridge::new(PaintFonts::test());
    let mut app = App::new();
    app.set_text_measurer(bridge.text.measurer());
    let env = Env::new(&mut app, FontSystem::test());
    let m = mount(&mut app, &env, session);
    let mut h = TestHost::new(app, SIZE);
    h.run_until_idle(200);
    Fx {
        h,
        bridge,
        atlases: AtlasStore::new(),
        m,
    }
}

impl Fx {
    fn find(&self, name: &str) -> ElementId {
        self.h
            .app
            .find(name)
            .unwrap_or_else(|| panic!("no element {name}\n{}", self.h.app.dump_tree()))
    }

    fn click(&mut self, name: &str) {
        let b = self.h.app.visual_bounds(self.find(name)).unwrap();
        // A card is clicked on its header.
        let y = if name.starts_with("node:") {
            b.y + 8.0
        } else {
            b.center().y
        };
        self.h.click(b.center().x, y);
        self.h.run_until_idle(200);
    }

    fn text(&self) -> String {
        let scene = self.h.scene().expect("a frame");
        self.h.app.dump_scene(scene)
    }

    fn frame(&self) -> u64 {
        self.m.editor.read(&self.h.app).session().frame()
    }

    /// Compare the window with `goldens/<name>.png`.
    fn golden(&mut self, name: &str) {
        self.h.app.request_frame();
        self.h.step();
        let scene = self.h.scene().expect("a frame").clone();
        let mut list = DisplayList::new();
        let atlas = self.bridge.paint_scene(&scene, 1.0, &mut list).unwrap();
        let options = FrameOptions {
            width: SIZE.width as u32,
            height: SIZE.height as u32,
            scale: 1.0,
            clear: Color::BLACK,
        };
        self.atlases.apply_all(&atlas).unwrap();
        let image = render_cpu_with(&encode(&list, options).unwrap(), &self.atlases);
        let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("goldens")
            .join(format!("{name}.png"));
        let actual = shots_dir().join(format!("{name}.actual.png"));
        write_png(&actual, &image).unwrap();
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            write_png(&golden, &image).unwrap();
            return;
        }
        let expected = read_png(&golden).unwrap_or_else(|e| panic!("{e}"));
        let result = compare(&image, &expected, Tolerance::CPU);
        assert!(
            result.passed,
            "{name} differs from its golden; see {}",
            actual.display()
        );
    }
}

#[test]
fn clicking_a_node_inspects_it() {
    let mut f = open("sun-and-lamp.room");
    assert!(f.text().contains("Select a node"));
    f.golden("sun-and-lamp");
    f.click("node:lamp_color");
    assert_eq!(
        f.m.editor
            .read(&f.h.app)
            .session()
            .inspection()
            .unwrap()
            .name,
        "lamp_color"
    );
    let shown = f.text();
    assert!(shown.contains("core.color.gradient@1"), "{shown}");
    assert!(shown.contains("from daylight.out"), "{shown}");
    f.find("swatch");
    f.golden("sun-and-lamp-inspected");
    // Clicking empty canvas clears it.
    f.h.click(20.0, 690.0);
    f.h.run_until_idle(200);
    assert!(f.m.editor.read(&f.h.app).session().inspection().is_none());
    assert!(f.text().contains("Select a node"));
}

#[test]
fn the_transport_plays_pauses_steps_and_rewinds() {
    let mut f = open("sun-and-lamp.room");
    f.click("node:lamp_intensity");
    let before = f.text();
    f.click("button:Step");
    assert_eq!(f.frame(), 1);
    assert!(f.text().contains("frame 1 "));

    let at =
        f.h.app
            .visual_bounds(f.find("button:Play"))
            .unwrap()
            .center();
    f.h.click(at.x, at.y);
    for _ in 0..180 {
        f.h.step();
    }
    let playing = f.frame();
    assert!(playing > 60, "{playing}");
    assert_ne!(f.text(), before);
    assert!(f.h.app.find("button:Pause").is_some());
    f.click("button:Pause");
    let paused = f.frame();
    f.h.run_until_idle(10);
    assert_eq!(f.frame(), paused, "a paused room is still");
    assert!(!f.h.app.needs_frame(), "and draws no frames");

    f.click("button:Rewind");
    assert_eq!(f.frame(), 0);
}

#[test]
fn a_room_with_a_charm_and_events_opens_and_plays() {
    for room in ["charm-demo.room", "blink.room"] {
        let mut f = open(room);
        f.click("button:Step");
        assert_eq!(f.frame(), 1, "{room}");
    }
    let mut f = open("charm-demo.room");
    f.golden("charm-demo");
}

/// A copy of an example room, removed afterwards.
struct Copy(PathBuf);

impl Copy {
    fn of(tag: &str) -> Copy {
        let from = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/sun-and-lamp.room/room.kdl");
        let dir = std::env::temp_dir().join(format!("dream-ui-{}-{tag}.room", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(from, dir.join("room.kdl")).unwrap();
        Copy(dir)
    }

    fn text(&self) -> String {
        std::fs::read_to_string(self.0.join("room.kdl")).unwrap()
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        // A leftover in the temporary directory is harmless.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Fx {
    fn session(&self) -> &Session {
        self.m.editor.read(&self.h.app).session()
    }

    fn value(&self, node: &str, input: &str) -> Option<Value> {
        let graph = self.session().graph();
        let id = graph.find(node)?;
        graph.input_value(id, input).ok()?.cloned()
    }

    /// Replace the text of a field and press Enter.
    fn enter(&mut self, field: &str, typed: &str) {
        self.click(field);
        self.h.key("ctrl-a");
        if typed.is_empty() {
            self.h.key("backspace");
        } else {
            self.h.type_text(typed);
        }
        self.h.key("enter");
        self.h.run_until_idle(200);
    }

    fn shortcut(&mut self, keys: &str) {
        self.h.key(keys);
        self.h.run_until_idle(200);
    }

    fn center(&self, name: &str) -> Point {
        self.h.app.visual_bounds(self.find(name)).unwrap().center()
    }

    fn drag(&mut self, from: Point, to: Point) {
        self.h.drag(from, to, 6);
        self.h.run_until_idle(200);
    }

    fn port(&self, node: &str, side: &str, index: usize) -> Point {
        let id = self.session().find(node).unwrap() + 1;
        self.center(&format!("port:{id}:{side}:{index}"))
    }

    fn linked(&self, node: &str, input: &str) -> Option<String> {
        let graph = self.session().graph();
        let (from, port) = graph.link_source(graph.find(node)?, input).ok()??;
        Some(format!("{}.{port}", graph.node_name(from)?))
    }
}

#[test]
fn fields_edit_undo_and_save() {
    let room = Copy::of("fields");
    let mut f = open(room.0.to_str().unwrap());
    let original = room.text();
    f.click("node:time_of_day");
    f.enter("field:offset", "12");
    assert_eq!(f.value("time_of_day", "offset"), Some(Value::Float(12.0)));
    assert!(f.text().contains("unsaved"));
    f.golden("editing");

    // A refused entry says why and changes nothing.
    f.enter("field:offset", "noon");
    assert_eq!(f.value("time_of_day", "offset"), Some(Value::Float(12.0)));
    assert!(f.text().contains("Float takes a number"), "{}", f.text());

    // Empty goes back to the default; Ctrl+Z from the canvas undoes.
    f.enter("field:offset", "");
    assert_eq!(f.value("time_of_day", "offset"), None);
    assert!(!f.text().contains("takes a number"));
    f.click("node:time_of_day");
    f.shortcut("ctrl-z");
    assert_eq!(f.value("time_of_day", "offset"), Some(Value::Float(12.0)));
    f.shortcut("ctrl-shift-z");
    assert_eq!(f.value("time_of_day", "offset"), None);
    f.click("button:Undo");
    assert_eq!(f.value("time_of_day", "offset"), Some(Value::Float(12.0)));

    // An expression, then a rename through the title.
    f.enter("field:offset", "=1 + 2");
    f.enter("field:name", "hours");
    assert!(f.session().find("hours").is_some());

    // A boolean input is a toggle.
    f.click("node:lamp_intensity");
    assert_eq!(f.value("lamp_intensity", "clamp"), Some(Value::Bool(true)));
    f.click("toggle");
    assert_eq!(f.value("lamp_intensity", "clamp"), Some(Value::Bool(false)));

    assert_eq!(room.text(), original, "nothing is written before saving");
    f.click("node:day_seconds");
    f.shortcut("ctrl-s");
    assert!(!f.session().dirty());
    assert!(!f.text().contains("unsaved"));
    let saved = room.text();
    assert!(
        saved.contains("expr=\"1 + 2\"") && saved.contains("\"hours\""),
        "{saved}"
    );
    assert!(saved.contains("// The Phase 0 acceptance scene"));
}

#[test]
fn the_canvas_moves_connects_and_disconnects() {
    let room = Copy::of("canvas");
    let mut f = open(room.0.to_str().unwrap());
    let sun = f.session().graph().find("sun").unwrap();
    let before = f.session().graph().position(sun).unwrap();
    // Drag the node by its header.
    let b = f.h.app.visual_bounds(f.find("node:sun")).unwrap();
    let grab = Point::new(b.center().x, b.y + 6.0);
    f.drag(grab, Point::new(grab.x, grab.y - 60.0));
    let moved = f.session().graph().position(sun).unwrap();
    assert!(moved[1] < before[1] - 20.0, "{before:?} {moved:?}");
    f.shortcut("ctrl-z");
    assert_eq!(f.session().graph().position(sun), Some(before), "one step");

    // `daylight.a` comes from sunrise: drag its link off onto nothing.
    assert_eq!(f.linked("daylight", "a").as_deref(), Some("sunrise.out"));
    let a = f.port("daylight", "in", 0);
    f.drag(a, Point::new(a.x, 600.0));
    assert_eq!(f.linked("daylight", "a"), None);
    f.shortcut("ctrl-z");
    assert_eq!(f.linked("daylight", "a").as_deref(), Some("sunrise.out"));

    // Connect sunset to `a` instead, from the output.
    let (out, a) = (f.port("sunset", "out", 0), f.port("daylight", "in", 0));
    f.drag(out, a);
    assert_eq!(f.linked("daylight", "a").as_deref(), Some("sunset.out"));
    f.shortcut("ctrl-z");
    assert_eq!(f.linked("daylight", "a").as_deref(), Some("sunrise.out"));
    assert!(!f.session().dirty(), "everything undone");
}

#[test]
fn closing_with_unsaved_changes_asks() {
    let room = Copy::of("close");
    let mut f = open(room.0.to_str().unwrap());
    let original = room.text();
    assert!(f.m.may_close(&mut f.h.app), "nothing to lose");
    f.click("node:time_of_day");
    f.enter("field:offset", "9");
    assert!(!f.m.may_close(&mut f.h.app));
    f.h.run_until_idle(200);
    f.find("dialog");
    f.click("button:Cancel");
    assert!(f.h.app.take_window_requests().is_empty());
    assert!(f.session().dirty());

    assert!(!f.m.may_close(&mut f.h.app));
    f.h.run_until_idle(200);
    f.click("button:Discard");
    assert_eq!(f.h.app.take_window_requests(), [WindowRequest::Close]);
    assert_eq!(room.text(), original);

    // The Save of the dialog, not of the toolbar.
    assert!(!f.m.may_close(&mut f.h.app));
    f.h.run_until_idle(200);
    f.shortcut("enter");
    assert_eq!(f.h.app.take_window_requests(), [WindowRequest::Close]);
    assert!(
        room.text().contains("input \"offset\" 9.0"),
        "{}",
        room.text()
    );
}
