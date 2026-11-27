// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Accessibility bounds follow layers (scrolling) without the tree being
//! rebuilt, and the incrementally maintained tree always equals a fresh one.

use std::collections::HashMap;

use accesskit::{Node, NodeId, TreeUpdate};
use livspeil_ui_a11y::{A11yTree, node_id};
use livspeil_ui_core::{App, Element, Render, TestHost, View, ViewContext, div, text};
use livspeil_ui_layout::{Size, Vec2};
use livspeil_ui_widgets::{Env, ScrollView, VirtualList};

struct Root {
    list: View<ScrollView>,
    label: u32,
}

impl Render for Root {
    fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
        div()
            .size(300.0, 300.0)
            .flex_col()
            .child(text(format!("Rows {}", self.label)).name("title"))
            .child(div().size(300.0, 200.0).child(Element::view(&self.list)))
    }
}

fn fixture() -> (TestHost, View<Root>, View<ScrollView>) {
    let mut app = App::new();
    let env = Env::headless(&mut app);
    let list = app.new_view(
        VirtualList::build(&env, 1000, 20.0, |_, i| {
            text(format!("Row {i}")).name(format!("text-{i}"))
        })
        .name("list"),
    );
    let root = app.new_view(Root { list, label: 0 });
    app.set_root(root);
    let mut h = TestHost::new(app, Size::new(300.0, 300.0));
    h.run_until_idle(100);
    (h, root, list)
}

/// What a platform adapter would hold after the updates.
#[derive(Default)]
struct Mirror {
    nodes: HashMap<NodeId, Node>,
    root: Option<NodeId>,
}

impl Mirror {
    fn apply(&mut self, update: TreeUpdate) {
        if let Some(tree) = update.tree {
            self.root = Some(tree.root);
        }
        self.nodes.extend(update.nodes);
        // Drop what is no longer reachable.
        let mut reachable = HashMap::new();
        let mut stack: Vec<NodeId> = self.root.into_iter().collect();
        while let Some(id) = stack.pop() {
            let node = self.nodes[&id].clone();
            stack.extend(node.children().iter().copied());
            reachable.insert(id, node);
        }
        self.nodes = reachable;
    }

    fn assert_fresh(&self, app: &App, scale: f64, at: &str) {
        let mut fresh = Mirror::default();
        let mut tree = A11yTree::new();
        fresh.apply(tree.full_update(app, scale).unwrap());
        assert_eq!(self.nodes.len(), fresh.nodes.len(), "{at}");
        for (id, node) in &fresh.nodes {
            assert_eq!(self.nodes.get(id), Some(node), "{at}");
        }
    }
}

#[test]
fn scrolling_sends_moved_bounds_without_rebuilding() {
    let (mut h, _, list) = fixture();
    let mut tree = A11yTree::new();
    let mut mirror = Mirror::default();
    mirror.apply(tree.full_update(&h.app, 2.0).unwrap());
    assert!(tree.stats().rebuilt);
    let row = node_id(h.app.find("text-3").unwrap());
    let title = node_id(h.app.find("title").unwrap());
    let before = mirror.nodes[&row].bounds().unwrap();

    ScrollView::jump_to(list, &mut h.app, Vec2::new(0.0, 40.0));
    let frame = h.step().unwrap();
    assert_eq!(frame.stats.views_rendered, 0);
    let update = tree.update(&h.app, 2.0).unwrap();
    assert!(!tree.stats().rebuilt);
    assert!(update.nodes.iter().any(|(id, _)| *id == row));
    assert!(update.nodes.iter().all(|(id, _)| *id != title));
    mirror.apply(update);
    // Physical pixels: 40 logical ones at scale 2.
    assert_eq!(mirror.nodes[&row].bounds().unwrap().y0, before.y0 - 80.0);
    mirror.assert_fresh(&h.app, 2.0, "after scrolling");

    // Nothing moved: nothing to send.
    h.app.request_frame();
    h.step().unwrap();
    assert!(tree.update(&h.app, 2.0).is_none());
    assert_eq!(tree.stats().sent, 0);
    // Another scale factor is a full walk.
    mirror.apply(tree.update(&h.app, 1.0).unwrap());
    assert!(tree.stats().rebuilt);
    mirror.assert_fresh(&h.app, 1.0, "after a scale change");
}

#[test]
fn incremental_bounds_equal_a_fresh_tree() {
    let (mut h, root, list) = fixture();
    let mut tree = A11yTree::new();
    let mut mirror = Mirror::default();
    mirror.apply(tree.full_update(&h.app, 1.25).unwrap());
    let mut seed = 7_u64;
    let mut walks = [0, 0];
    for step in 0..300 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let pick = seed >> 33;
        let to = f32::from(u16::try_from(pick % 4000).unwrap());
        match pick % 7 {
            0..=4 => ScrollView::jump_to(list, &mut h.app, Vec2::new(0.0, to / 10.0)),
            5 => ScrollView::scroll_to(list, &mut h.app, Vec2::new(0.0, to)),
            _ => root.update(&mut h.app, |r, cx| {
                r.label += 1;
                cx.notify();
            }),
        }
        // Sometimes several frames of motion between updates.
        for _ in 0..=(pick % 3) {
            h.step();
        }
        if let Some(update) = tree.update(&h.app, 1.25) {
            walks[usize::from(tree.stats().rebuilt)] += 1;
            mirror.apply(update);
        }
        mirror.assert_fresh(&h.app, 1.25, &format!("step {step}"));
    }
    assert!(walks[0] > 50 && walks[1] > 20, "{walks:?}");
}
