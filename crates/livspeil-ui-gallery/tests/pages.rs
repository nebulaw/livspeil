// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Every gallery page in a `TestHost`: it builds and settles, every
//! control is reachable with Tab, and its accessibility tree is valid with
//! a name on every control. Plus page shortcuts and the live toolbar.

use std::collections::{HashMap, HashSet};

use livspeil_ui_a11y::A11yTree;
use livspeil_ui_a11y::accesskit::{Node, NodeId, Role as AkRole};
use livspeil_ui_core::{App, ElementId, TestHost};
use livspeil_ui_gallery::{Mounted, Page, WINDOW_SIZE, mount};
use livspeil_ui_layout::Size;
use livspeil_ui_paint::PaintBridge;
use livspeil_ui_text::FontSystem;
use livspeil_ui_theme::{Density, Scheme, Theme, Tier};

fn host_with(page: Page, theme: Theme) -> (TestHost, Mounted) {
    let mut app = App::new();
    // Real text measurement, like the app and the screenshots.
    let bridge = PaintBridge::new(livspeil_ui_paint::FontSystem::test());
    app.set_text_measurer(bridge.text.measurer());
    app.set_theme(theme);
    let m = mount(&mut app, FontSystem::test(), page);
    let mut h = TestHost::new(app, Size::new(WINDOW_SIZE.0, WINDOW_SIZE.1));
    h.run_until_idle(300);
    (h, m)
}

fn host(page: Page) -> (TestHost, Mounted) {
    host_with(page, Theme::default())
}

fn current(h: &TestHost, m: &Mounted) -> Page {
    m.gallery.read(&h.app).page(&h.app)
}

const CONTROLS: [AkRole; 11] = [
    AkRole::Button,
    AkRole::CheckBox,
    AkRole::Switch,
    AkRole::RadioButton,
    AkRole::Slider,
    AkRole::SpinButton,
    AkRole::TextInput,
    AkRole::ComboBox,
    AkRole::Tab,
    AkRole::RadioGroup,
    AkRole::TabList,
];

/// Checks the tree's structure and names; returns the nodes.
fn check_tree(app: &App, page: Page) -> HashMap<NodeId, Node> {
    let mut tree = A11yTree::new();
    let update = tree.full_update(app, 1.0).expect("tree");
    let root = update.tree.as_ref().expect("full tree").root;
    let nodes: HashMap<NodeId, Node> = update.nodes.into_iter().collect();
    assert!(nodes.contains_key(&root), "{page:?}: root missing");
    assert!(nodes.contains_key(&update.focus), "{page:?}: focus missing");
    let mut parents: HashMap<NodeId, NodeId> = HashMap::new();
    for (id, n) in &nodes {
        for c in n.children() {
            assert!(nodes.contains_key(c), "{page:?}: dangling child {c:?}");
            assert!(
                parents.insert(*c, *id).is_none(),
                "{page:?}: {c:?} has two parents"
            );
        }
        if CONTROLS.contains(&n.role()) {
            assert!(
                n.label().is_some_and(|l| !l.trim().is_empty()),
                "{page:?}: unnamed {:?} {id:?}",
                n.role()
            );
        }
    }
    // Every node but the root hangs off the root.
    for id in nodes.keys() {
        let mut cur = *id;
        let mut steps = 0;
        while cur != root {
            cur = *parents
                .get(&cur)
                .unwrap_or_else(|| panic!("{page:?}: {id:?} is detached"));
            steps += 1;
            assert!(steps < 1000, "{page:?}: cycle");
        }
    }
    nodes
}

#[test]
fn every_page_builds_settles_and_has_a_valid_tree() {
    for page in Page::ALL {
        let (h, m) = host(page);
        assert!(!h.app.needs_frame(), "{page:?}: settles");
        assert_eq!(current(&h, &m), page);
        let nodes = check_tree(&h.app, page);
        // The sidebar lists every page as a tab; the current one is selected.
        let titles: Vec<&str> = Page::ALL.iter().map(|p| p.title()).collect();
        let tabs: Vec<&Node> = nodes
            .values()
            .filter(|n| n.role() == AkRole::Tab && n.label().is_some_and(|l| titles.contains(&l)))
            .collect();
        assert_eq!(tabs.len(), Page::ALL.len(), "{page:?}");
        let selected: Vec<_> = tabs
            .iter()
            .filter(|n| n.is_selected() == Some(true))
            .map(|n| n.label().map(str::to_owned))
            .collect();
        assert_eq!(selected, [Some(page.title().to_owned())]);
        assert!(
            nodes
                .values()
                .any(|n| n.role() == AkRole::Heading && n.label() == Some(page.title())),
            "{page:?}: heading"
        );
    }
}

#[test]
fn light_pro_theme_page_is_valid() {
    let theme = Theme {
        scheme: Scheme::Light,
        density: Density::Pro,
        ..Theme::default()
    };
    let (h, _) = host_with(Page::Theme, theme);
    assert!(!h.app.needs_frame());
    check_tree(&h.app, Page::Theme);
}

#[test]
fn every_control_is_reachable_with_tab() {
    for page in Page::ALL {
        let (mut h, _) = host(page);
        let order = h.app.tab_order();
        assert!(order.len() > Page::ALL.len(), "{page:?}: {}", order.len());
        let mut seen: HashSet<ElementId> = HashSet::new();
        for _ in 0..order.len() + 2 {
            h.key("tab");
            h.run_until_idle(100);
            if let Some(f) = h.app.focused() {
                seen.insert(f);
            }
        }
        h.run_until_idle(300);
        // Tab order is recomputed per frame; compare against the final one.
        let order: HashSet<ElementId> = h.app.tab_order().into_iter().collect();
        let missing: Vec<String> = order
            .difference(&seen)
            .filter_map(|id| h.app.label(*id))
            .collect();
        assert!(
            missing.is_empty(),
            "{page:?}: not reached by Tab: {missing:?}"
        );
        // Every tab stop is exposed to accessibility with a name.
        let nodes = check_tree(&h.app, page);
        for id in &order {
            let n = livspeil_ui_a11y::node_id(*id);
            let node = nodes
                .get(&n)
                .unwrap_or_else(|| panic!("{page:?}: tab stop {:?} not exposed", h.app.label(*id)));
            assert!(
                node.label().is_some_and(|l| !l.is_empty()),
                "{page:?}: tab stop {:?} unnamed",
                h.app.label(*id)
            );
        }
    }
}

#[test]
fn shortcuts_switch_pages() {
    let (mut h, m) = host(Page::Buttons);
    for page in Page::ALL {
        h.key(&format!("ctrl-{}", page.shortcut()));
        h.run_until_idle(300);
        assert_eq!(current(&h, &m), page, "ctrl-{}", page.shortcut());
    }
    h.key("cmd-3");
    h.run_until_idle(300);
    assert_eq!(current(&h, &m), Page::Radios);
    // Clicking a sidebar row works too.
    let id = h.app.find("page:Motion").unwrap();
    let c = h.app.bounds(id).unwrap().center();
    h.click(c.x, c.y);
    h.run_until_idle(300);
    assert_eq!(current(&h, &m), Page::Motion);
}

fn click_named(h: &mut TestHost, name: &str) {
    let id = h.app.find(name).unwrap_or_else(|| panic!("no {name}"));
    let c = h.app.bounds(id).unwrap().center();
    h.click(c.x, c.y);
    h.run_until_idle(300);
}

/// The toolbar control labelled `label` (segmented controls and toggles
/// are found through their accessibility names).
fn toolbar_control(h: &TestHost, label: &str) -> ElementId {
    let toolbar = h.app.find("toolbar").unwrap();
    let mut stack = h.app.children_of(toolbar);
    while let Some(id) = stack.pop() {
        if let Some(n) = h.app.semantic_node(id)
            && n.label() == Some(label)
        {
            return id;
        }
        stack.extend(h.app.children_of(id));
    }
    panic!("no toolbar control {label}");
}

fn segment(h: &TestHost, control: ElementId, index: usize) -> (f32, f32) {
    let b = h.app.bounds(control).unwrap();
    let kids = h.app.children_of(control);
    let n = kids.len().max(1);
    #[allow(clippy::cast_precision_loss)]
    let x = b.x + b.width * (index as f32 + 0.5) / n as f32;
    (x, b.y + b.height / 2.0)
}

#[test]
fn toolbar_switches_are_live() {
    let (mut h, _) = host(Page::Theme);
    let theme = toolbar_control(&h, "Theme");
    let (x, y) = segment(&h, theme, 1);
    h.click(x, y);
    h.run_until_idle(300);
    assert_eq!(h.app.theme().scheme, Scheme::Light);
    let density = toolbar_control(&h, "Density");
    let (x, y) = segment(&h, density, 1);
    h.click(x, y);
    h.run_until_idle(300);
    assert_eq!(h.app.theme().density, Density::Pro);
    let tier = toolbar_control(&h, "Material tier");
    let (x, y) = segment(&h, tier, 0);
    h.click(x, y);
    h.run_until_idle(300);
    assert_eq!(h.app.theme().tier, Tier::T0);
    let rm = toolbar_control(&h, "Reduce motion");
    let c = h.app.bounds(rm).unwrap().center();
    h.click(c.x, c.y);
    h.run_until_idle(300);
    assert!(h.app.motion().reduce_motion);
    let rt = toolbar_control(&h, "Reduce transparency");
    let c = h.app.bounds(rt).unwrap().center();
    h.click(c.x, c.y);
    h.run_until_idle(300);
    assert!(h.app.theme().reduce_transparency);
    assert!(!h.app.needs_frame());
    check_tree(&h.app, Page::Theme);
}

#[test]
fn spring_playground_animates_and_settles() {
    let (mut h, m) = host(Page::Motion);
    let knob = h.app.find("knob").unwrap();
    let x0 = h.app.bounds(knob).unwrap().x;
    click_named(&mut h, "button:Animate");
    let knob = h.app.find("knob").unwrap();
    let x1 = h.app.bounds(knob).unwrap().x;
    assert!(x1 > x0 + 100.0, "{x0} -> {x1}");
    assert!(!h.app.needs_frame(), "settled");
    let ball = m.gallery.read(&h.app).state().ball;
    assert!((ball.read(&h.app).value() - 1.0).abs() < 1e-3);
    // Reduce motion: the next move is instant.
    let mut motion = h.app.motion().clone();
    motion.reduce_motion = true;
    h.app.set_motion(motion);
    click_named(&mut h, "button:Animate");
    assert!(ball.read(&h.app).value().abs() < 1e-6);
}

#[test]
fn speed_slider_drives_the_motion_config() {
    let (mut h, m) = host(Page::Motion);
    let speed = m.gallery.read(&h.app).state().speed;
    speed.update(&mut h.app, |s, cx| {
        *s = 0.5;
        cx.notify();
    });
    h.run_until_idle(300);
    assert!((h.app.motion().speed - 0.5).abs() < 1e-6);
    // A hot-reloaded speed shows on the slider.
    let mut motion = h.app.motion().clone();
    motion.speed = 1.5;
    h.app.set_motion(motion);
    m.gallery.update(&mut h.app, |_, cx| cx.notify());
    h.run_until_idle(300);
    assert!((*speed.read(&h.app) - 1.5).abs() < 1e-6);
    assert!((h.app.motion().speed - 1.5).abs() < 1e-6);
}

#[test]
fn check_box_parent_is_mixed() {
    let (mut h, m) = host(Page::Toggles);
    let files = m.gallery.read(&h.app).state().files;
    click_named(&mut h, "checkbox");
    // The first checkbox in the tree is "All files": mixed -> all checked.
    assert!(files.iter().all(|f| *f.read(&h.app)));
}

#[test]
fn the_shell_starts_focused() {
    let (h, _) = host(Page::Buttons);
    assert_eq!(h.app.focused(), h.app.find("gallery-shell"));
    assert!(!h.app.focus_visible());
}
