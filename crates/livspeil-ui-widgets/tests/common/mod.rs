// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Test harness: a root view built by a closure inside a `WidgetRoot`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use livspeil_ui_core::{
    App, Element, ElementId, InputEvent, Modifiers, MouseButton, PointerEvent, PointerKind,
    Render, TestHost, ViewContext,
};
use livspeil_ui_layout::{Point, Size};
use livspeil_ui_widgets::Env;

pub type Build = Box<dyn Fn(&mut ViewContext<'_, Harness>) -> Element>;

pub struct Harness {
    build: Build,
}

impl Render for Harness {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        (self.build)(cx)
    }
}

pub fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// A host with a `WidgetRoot`; `setup` creates models/views and returns
/// the content builder.
pub fn host(setup: impl FnOnce(&mut App, &Env) -> Build) -> (TestHost, Env) {
    host_sized(Size::new(480.0, 360.0), setup)
}

pub fn host_sized(size: Size, setup: impl FnOnce(&mut App, &Env) -> Build) -> (TestHost, Env) {
    let mut app = App::new();
    let env = Env::headless(&mut app);
    let build = setup(&mut app, &env);
    let content = app.new_view(Harness { build });
    let root = env.root(&mut app, content);
    app.set_root(root);
    let mut host = TestHost::new(app, size);
    host.run_until_idle(100);
    (host, env)
}

pub fn find(h: &TestHost, name: &str) -> ElementId {
    h.app.find(name).unwrap_or_else(|| panic!("no element {name}\n{}", h.app.dump_tree()))
}

pub fn center(h: &TestHost, name: &str) -> Point {
    h.app.bounds(find(h, name)).unwrap().center()
}

pub fn click(h: &mut TestHost, name: &str) {
    let p = center(h, name);
    h.click(p.x, p.y);
    h.run_until_idle(200);
}

/// A pointer event with modifiers at the host's current time.
pub fn pointer(h: &mut TestHost, kind: PointerKind, p: Point, modifiers: Modifiers) {
    let time = h.now();
    h.app.dispatch(InputEvent::Pointer(PointerEvent {
        kind,
        position: p,
        modifiers,
        time,
    }));
}

pub fn down(h: &mut TestHost, p: Point, m: Modifiers) {
    pointer(h, PointerKind::Down(MouseButton::Left), p, m);
}

pub fn moved(h: &mut TestHost, p: Point, m: Modifiers) {
    pointer(h, PointerKind::Move, p, m);
}

pub fn up(h: &mut TestHost, p: Point, m: Modifiers) {
    pointer(h, PointerKind::Up(MouseButton::Left), p, m);
}

pub const ALT: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: true,
    cmd: false,
};

/// Advance virtual time without input.
pub fn wait(h: &mut TestHost, ms: u64) {
    h.advance(Duration::from_millis(ms));
}

/// The scene of the last frame with labels.
pub fn scene(h: &TestHost) -> String {
    let s = h.scene().expect("a frame").clone();
    h.app.dump_scene(&s)
}
