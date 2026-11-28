// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The window, headless: selecting, the transport and golden screenshots.
//! `UPDATE_GOLDEN=1` accepts new screenshots; look at them.

use std::path::PathBuf;

use livspeil_dream::app::{Mounted, mount};
use livspeil_dream::session::Session;
use livspeil_ui_core::{App, ElementId, TestHost};
use livspeil_ui_layout::Size;
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
        self.m.session.read(&self.h.app).frame()
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
        f.m.session.read(&f.h.app).inspection().unwrap().name,
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
    assert!(f.m.session.read(&f.h.app).inspection().is_none());
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
