// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The window, headless: selecting, the transport and golden screenshots.
//! `UPDATE_GOLDEN=1` accepts new screenshots; look at them.

use std::path::PathBuf;

use livspeil_dream::app::{Mounted, mount};
use livspeil_dream::session::Session;
use livspeil_node_api::Value;
use livspeil_ui_core::{App, ElementId, Keystroke, TestHost, WindowRequest};
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
    mounted(session)
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

fn mounted(session: Session) -> Fx {
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
        let at = self.session().find(node).unwrap();
        let id = self.m.editor.read(&self.h.app).canvas_id(at).unwrap();
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
    assert_eq!(moved.map(f64::fract), [0.0; 2], "{moved:?}");
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

/// A directory for one test, removed afterwards.
struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let dir = std::env::temp_dir().join(format!("dream-ui-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        // A leftover in the temporary directory is harmless.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn untitled() -> Fx {
    mounted(Session::untitled().unwrap())
}

impl Fx {
    /// As a window delivers Shift+A: the key, its text, the release; then
    /// type into the list and take the first match.
    fn add(&mut self, query: &str) {
        let key = Keystroke::parse("shift-a").unwrap();
        self.h.key_event(key.key, key.modifiers, true);
        self.h.type_text("A");
        self.h.key_event(key.key, key.modifiers, false);
        self.h.run_until_idle(200);
        self.h.type_text(query);
        self.h.run_until_idle(200);
        self.shortcut("enter");
    }

    fn card(&self, node: &str) -> Point {
        let b = self
            .h
            .app
            .visual_bounds(self.find(&format!("node:{node}")))
            .unwrap();
        Point::new(b.center().x, b.y + 6.0)
    }

    /// Drag a node by its header until its header is at `to`.
    fn place(&mut self, node: &str, to: Point) {
        let from = self.card(node);
        self.drag(from, to);
    }

    /// Put `text` into the field named `field`, replacing what is there.
    fn type_into(&mut self, field: &str, typed: &str) {
        self.click(field);
        self.h.key("ctrl-a");
        self.h.type_text(typed);
        self.h.run_until_idle(200);
    }

    fn connect(&mut self, from: &str, to: &str) {
        let (from_node, from_port) = from.split_once('.').unwrap();
        let (to_node, to_port) = to.split_once('.').unwrap();
        let (out, input) = {
            let s = self.session();
            let source = &s.nodes()[s.find(from_node).unwrap()];
            let target = &s.nodes()[s.find(to_node).unwrap()];
            (
                source
                    .outputs
                    .iter()
                    .position(|p| p.name == from_port)
                    .unwrap(),
                target
                    .inputs
                    .iter()
                    .position(|p| p.name == to_port)
                    .unwrap(),
            )
        };
        let a = self.port(from_node, "out", out);
        let b = self.port(to_node, "in", input);
        self.drag(a, b);
        assert_eq!(
            self.linked(to_node, to_port).as_deref(),
            Some(from),
            "{from} -> {to}"
        );
    }
}

#[test]
fn adding_deleting_and_undoing_nodes() {
    let mut f = untitled();
    assert!(f.text().contains("Press Shift+A"), "{}", f.text());
    f.add("time.clock");
    assert!(f.session().find("clock").is_some());
    let selected = |f: &Fx| f.session().inspection().map(|i| i.name.clone());
    assert_eq!(selected(&f).as_deref(), Some("clock"));

    // Another of the same type gets a name of its own, beside the first.
    f.add("time.clock");
    assert_eq!(selected(&f).as_deref(), Some("clock2"));
    assert_ne!(f.card("clock"), f.card("clock2"));

    // Delete is for the selection; one undo brings it back.
    f.shortcut("delete");
    assert!(f.session().find("clock2").is_none());
    assert_eq!(selected(&f), None);
    f.shortcut("ctrl-z");
    assert!(f.session().find("clock2").is_some());
    // Undoing the add removes the node again; the keyboard still works.
    f.click("node:clock2");
    f.shortcut("ctrl-z");
    assert!(f.session().find("clock2").is_none());
    f.shortcut("ctrl-z");
    assert!(f.session().find("clock").is_none());
    f.shortcut("ctrl-shift-z");
    assert!(f.session().find("clock").is_some());

    // The button opens the same list; Escape adds nothing.
    f.click("button:Add");
    f.find("add-query");
    f.h.type_text("mix");
    f.h.run_until_idle(200);
    f.golden("add-list");
    f.shortcut("escape");
    assert!(f.h.app.find("add-query").is_none());
    assert_eq!(f.session().nodes().len(), 1);
}

#[test]
fn a_node_read_by_an_expression_is_not_deleted_alone() {
    let mut f = untitled();
    f.add("math.add");
    f.add("math.mul");
    f.place("mul", Point::new(600.0, 500.0));
    f.click("node:mul");
    f.enter("field:a", "=ch(\"add/a\")");
    f.click("node:add");
    f.shortcut("delete");
    assert!(f.session().find("add").is_some());
    assert!(f.text().contains("`add` cannot be deleted"), "{}", f.text());
    // Together with what reads it, it goes.
    f.shortcut("ctrl-a");
    f.shortcut("delete");
    assert!(f.session().nodes().is_empty());
}

/// Name, type, values and links of every node: what a room does, apart
/// from ids and layout.
fn meaning(session: &Session) -> Vec<String> {
    let graph = session.graph();
    let mut out = Vec::new();
    for node in session.nodes() {
        let type_id = graph.node_type(node.id).unwrap();
        out.push(format!("{} is {type_id}", node.name));
        for port in &node.inputs {
            if let Some(v) = graph.input_value(port.node, &port.port).unwrap() {
                out.push(format!("{}.{} = {v:?}", node.name, port.name));
            }
            if let Some((from, from_port)) = graph.link_source(port.node, &port.port).unwrap() {
                let from = graph.node_name(from).unwrap();
                out.push(format!("{}.{} <- {from}.{from_port}", node.name, port.name));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn sun_and_lamp_can_be_rebuilt_from_an_empty_room() {
    let dir = Dir::new("rebuild");
    let mut f = untitled();
    // (type searched for, name, where its header goes)
    let nodes = [
        ("time.clock", "clock", (110.0, 300.0)),
        ("math.mul", "day_seconds", (340.0, 300.0)),
        ("time.timeofday", "time_of_day", (570.0, 300.0)),
        ("sky.sun_direction", "sun", (800.0, 90.0)),
        ("math.smoothstep", "sunrise", (800.0, 270.0)),
        ("math.smoothstep", "sunset", (800.0, 470.0)),
        ("math.sub", "daylight", (110.0, 480.0)),
        ("color.gradient", "lamp_color", (340.0, 480.0)),
        ("math.remap", "lamp_intensity", (570.0, 480.0)),
    ];
    for (query, name, (x, y)) in nodes {
        f.add(query);
        f.enter("field:name", name);
        assert!(f.session().find(name).is_some(), "{query} as {name}");
        // Naming left the keyboard in the field: move, which does not.
        f.place(name, Point::new(x, y));
    }
    let values: &[(&str, &[(&str, &str)])] = &[
        ("day_seconds", &[("b", "8640")]),
        ("time_of_day", &[("offset", "6")]),
        ("sun", &[("day_of_year", "172"), ("latitude", "41.7")]),
        ("sunrise", &[("edge0", "5"), ("edge1", "8")]),
        ("sunset", &[("edge0", "17"), ("edge1", "20")]),
        (
            "lamp_color",
            &[
                ("c0", "0.35 0.08 260 1"),
                ("c1", "0.78 0.14 60 1"),
                ("c2", "0.95 0.03 95 1"),
                ("p1", "0.4"),
            ],
        ),
        ("lamp_intensity", &[("out_max", "0.2"), ("out_min", "1")]),
    ];
    for (node, inputs) in values {
        f.click(&format!("node:{node}"));
        for (input, typed) in *inputs {
            f.enter(&format!("field:{input}"), typed);
        }
    }
    f.click("node:lamp_intensity");
    f.click("toggle");
    for (from, to) in [
        ("clock.time", "day_seconds.a"),
        ("day_seconds.out", "time_of_day.seconds"),
        ("time_of_day.hour", "sun.solar_hour"),
        ("time_of_day.hour", "sunrise.x"),
        ("time_of_day.hour", "sunset.x"),
        ("sunrise.out", "daylight.a"),
        ("sunset.out", "daylight.b"),
        ("daylight.out", "lamp_color.t"),
        ("daylight.out", "lamp_intensity.x"),
    ] {
        f.connect(from, to);
    }

    // The room has no file yet, so saving asks where.
    f.shortcut("ctrl-s");
    let room = dir.0.join("rebuilt.room");
    f.type_into("save-as-path", room.to_str().unwrap());
    f.shortcut("enter");
    assert!(!f.session().dirty(), "{}", f.text());
    assert_eq!(f.session().name(), "rebuilt");

    let original =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/sun-and-lamp.room");
    let original = Session::open(&original).unwrap();
    let saved = Session::open(&room).unwrap();
    assert_eq!(meaning(&saved), meaning(&original));
}

#[test]
fn new_and_save_as_keep_what_there_is() {
    let dir = Dir::new("save-as");
    let room = Copy::of("save-as");
    let original = room.text();
    std::fs::create_dir_all(room.0.join("assets")).unwrap();
    std::fs::write(room.0.join("assets/picture"), "pixels").unwrap();
    let mut f = open(room.0.to_str().unwrap());
    f.click("node:time_of_day");
    f.enter("field:offset", "9");

    // New asks about the unsaved change first.
    f.shortcut("ctrl-n");
    f.find("dialog");
    f.click("button:Cancel");
    assert!(f.session().dirty() && !f.session().nodes().is_empty());

    // Save as refuses what is there, says why and keeps asking.
    f.shortcut("ctrl-shift-s");
    f.type_into("save-as-path", room.0.parent().unwrap().to_str().unwrap());
    let taken = dir.0.join("taken.room");
    std::fs::create_dir_all(&taken).unwrap();
    f.type_into("save-as-path", taken.to_str().unwrap());
    f.shortcut("enter");
    assert!(f.text().contains("already exists"), "{}", f.text());
    assert!(f.session().dirty());

    // `.room` is added; assets come along; the original is untouched.
    f.type_into("save-as-path", dir.0.join("copy").to_str().unwrap());
    f.shortcut("enter");
    assert!(!f.session().dirty(), "{}", f.text());
    let copy = dir.0.join("copy.room");
    assert_eq!(f.session().path(), Some(copy.join("room.kdl").as_path()));
    assert_eq!(f.session().name(), "Sun and Lamp");
    let saved = std::fs::read_to_string(copy.join("room.kdl")).unwrap();
    assert!(saved.contains("input \"offset\" 9.0"), "{saved}");
    assert!(saved.contains("// The Phase 0 acceptance scene"));
    assert_eq!(
        std::fs::read_to_string(copy.join("assets/picture")).unwrap(),
        "pixels"
    );
    assert_eq!(room.text(), original);

    // With nothing unsaved, New just starts over.
    f.click("button:New");
    assert!(f.session().nodes().is_empty());
    assert_eq!(f.session().path(), None);
    assert_eq!(f.session().name(), "Untitled");
    f.add("math.add");
    assert_eq!(f.session().nodes().len(), 1);
}

#[test]
fn a_new_room_shows_nothing_of_the_old_one() {
    let dir = Dir::new("old-cards");
    let mut f = untitled();
    f.add("time.clock");
    f.shortcut("ctrl-s");
    f.type_into("save-as-path", dir.0.join("one").to_str().unwrap());
    f.shortcut("enter");
    f.shortcut("ctrl-n");
    assert!(f.h.app.find("node:clock").is_none(), "{}", f.text());
    assert!(f.session().nodes().is_empty());
}

#[test]
fn closing_a_room_without_a_file_asks_where_to_save_it() {
    let dir = Dir::new("close-untitled");
    let mut f = untitled();
    f.add("math.add");
    assert!(!f.m.may_close(&mut f.h.app));
    f.h.run_until_idle(200);
    f.shortcut("enter");
    // Not closed yet: it needs a place first.
    assert!(f.h.app.take_window_requests().is_empty());
    f.find("save-as-path");
    // Giving up on that keeps the window.
    f.shortcut("escape");
    assert!(f.h.app.take_window_requests().is_empty());
    assert!(f.session().dirty());

    assert!(!f.m.may_close(&mut f.h.app));
    f.h.run_until_idle(200);
    // Enter is the dialog's Save; the toolbar's is behind it.
    f.shortcut("enter");
    f.type_into("save-as-path", dir.0.join("kept").to_str().unwrap());
    f.shortcut("enter");
    assert_eq!(f.h.app.take_window_requests(), [WindowRequest::Close]);
    assert!(dir.0.join("kept.room/room.kdl").is_file());
}
