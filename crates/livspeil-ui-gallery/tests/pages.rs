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
    // The first ten pages have Ctrl/Cmd+1..9, 0; the rest only the sidebar.
    let with_digit: Vec<Page> = Page::ALL
        .into_iter()
        .filter(|p| p.shortcut().is_some())
        .collect();
    assert_eq!(with_digit, Page::ALL[..10]);
    for page in with_digit {
        let digit = page.shortcut().unwrap();
        h.key(&format!("ctrl-{digit}"));
        h.run_until_idle(300);
        assert_eq!(current(&h, &m), page, "ctrl-{digit}");
    }
    assert_eq!(Page::NodeCanvas.shortcut(), Some('0'));
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

/// The switch, segmented control or button on the current page with
/// accessible name `label`.
fn page_control(h: &TestHost, label: &str) -> ElementId {
    use livspeil_ui_core::Role;
    let mut stack = vec![h.app.find("page").unwrap()];
    while let Some(id) = stack.pop() {
        if h.app.semantic_node(id).is_some_and(|n| {
            n.label() == Some(label)
                && matches!(n.role(), Some(Role::Switch | Role::TabList | Role::Button))
        }) {
            return id;
        }
        stack.extend(h.app.children_of(id));
    }
    panic!("no control {label} on the page");
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

#[test]
fn sidebar_groups_pages_and_scrolls() {
    use livspeil_ui_gallery::Group;
    let groups: Vec<Group> = Page::ALL.iter().map(|p| p.group()).collect();
    // Groups are contiguous, in heading order.
    let mut seen: Vec<Group> = Vec::new();
    for g in groups {
        if seen.last() != Some(&g) {
            assert!(!seen.contains(&g), "{g:?} split");
            seen.push(g);
        }
    }
    assert_eq!(seen, Group::ALL);
    // In a short window the sidebar scrolls; Tab reveals the last page.
    let mut app = App::new();
    let bridge = PaintBridge::new(livspeil_ui_paint::FontSystem::test());
    app.set_text_measurer(bridge.text.measurer());
    let m = mount(&mut app, FontSystem::test(), Page::Buttons);
    let mut h = TestHost::new(app, Size::new(WINDOW_SIZE.0, 400.0));
    h.run_until_idle(300);
    let theme_row = h.app.find("page:Theme").unwrap();
    let sb = h.app.find("sidebar-scroll").unwrap();
    let view = h.app.bounds(sb).unwrap();
    assert!(h.app.bounds(theme_row).unwrap().bottom() > view.bottom());
    let typo = h.app.find("page:Typography").unwrap();
    m.env.focus(&mut h.app, typo);
    for _ in 0..2 {
        h.key("tab");
        h.run_until_idle(300);
    }
    assert_eq!(h.app.focused(), Some(theme_row));
    // Layout bounds do not include the scroll translation.
    let offset = m.gallery.read(&h.app).sidebar().read(&h.app).offset();
    let r = h.app.bounds(theme_row).unwrap();
    let shown = r.bottom() - offset.y;
    assert!(
        offset.y > 0.0 && shown <= view.bottom() + 0.5,
        "{r:?} - {offset:?} in {view:?}"
    );
    // A click on it (where it is shown) still switches pages.
    h.click(r.center().x, r.center().y - offset.y);
    h.run_until_idle(300);
    assert_eq!(current(&h, &m), Page::Theme);
}

#[test]
fn virtual_list_builds_only_visible_rows() {
    let (mut h, m) = host(Page::Scroll);
    let list = m.gallery.read(&h.app).state().containers.list;
    let rows = |h: &TestHost| {
        let spacer = h.app.find("virtual-list").unwrap();
        h.app.children_of(spacer).len()
    };
    let n = rows(&h);
    assert!((5..40).contains(&n), "{n} rows mounted");
    // End jumps to the last of the 100 000 rows.
    let sv = h.app.find("virtual-list-scroll").unwrap();
    m.env.focus(&mut h.app, sv);
    h.key("end");
    h.run_until_idle(600);
    assert!(rows(&h) < 40);
    let last = h.app.find("virtual-list").unwrap().child("row-99999");
    assert!(h.app.bounds(last).is_some(), "last row mounted");
    assert!(list.read(&h.app).offset().y > 2_700_000.0);
    assert_eq!(livspeil_ui_gallery::row_title(0), "Aurora ember");
}

#[test]
fn scroll_view_scrolls_with_the_wheel() {
    let (mut h, m) = host(Page::Scroll);
    let sv = h.app.find("long-scroll").unwrap();
    let c = h.app.bounds(sv).unwrap().center();
    h.pointer_move(c.x, c.y);
    h.app.dispatch(livspeil_ui_core::InputEvent::Pointer(
        livspeil_ui_core::PointerEvent {
            kind: livspeil_ui_core::PointerKind::Scroll(livspeil_ui_core::ScrollDelta::Lines(
                livspeil_ui_layout::Vec2::new(0.0, -5.0),
            )),
            position: c,
            modifiers: livspeil_ui_core::Modifiers::NONE,
            time: h.now(),
        },
    ));
    h.run_until_idle(600);
    let long = m.gallery.read(&h.app).state().containers.long;
    assert!(long.read(&h.app).offset().y > 100.0);
    assert!(!h.app.needs_frame());
}

#[test]
fn dock_splitters_resize_and_sides_collapse() {
    use livspeil_ui_core::CursorShape;
    use livspeil_ui_layout::DockEdge;
    use livspeil_ui_widgets::DockView;
    let (mut h, m) = host(Page::Dock);
    let dock = m.gallery.read(&h.app).state().containers.dock;
    let before = dock.read(&h.app).size(DockEdge::Left).unwrap();
    let s = h.app.find("splitter:left").unwrap();
    let p = h.app.bounds(s).unwrap().center();
    h.pointer_move(p.x, p.y);
    assert_eq!(h.app.take_cursor_request(), Some(CursorShape::ResizeEw));
    h.drag(p, livspeil_ui_layout::Point::new(p.x + 40.0, p.y), 6);
    h.run_until_idle(300);
    let after = dock.read(&h.app).size(DockEdge::Left).unwrap();
    assert!((after - before - 40.0).abs() < 1.0, "{before} -> {after}");
    // The Inspector switch collapses the right side (animated).
    let toggle = page_control(&h, "Inspector");
    let c = h.app.bounds(toggle).unwrap().center();
    h.click(c.x, c.y);
    h.run_until_idle(600);
    assert!(dock.read(&h.app).is_collapsed(DockEdge::Right));
    assert!(dock.read(&h.app).shown_size(DockEdge::Right) < 0.5);
    assert!(h.app.find("panel:Inspector").is_none());
    DockView::toggle(dock, &mut h.app, DockEdge::Right);
    h.run_until_idle(600);
    assert!(h.app.find("panel:Inspector").is_some());
    // Selecting a layer updates the inspector.
    click_named(&mut h, "layer:Sun");
    let layer = m.gallery.read(&h.app).state().containers.layer;
    assert_eq!(*layer.read(&h.app), 1);
    assert!(!h.app.needs_frame());
    check_tree(&h.app, Page::Dock);
}

#[test]
fn menu_bar_and_context_menu_work() {
    let (mut h, m) = host(Page::Menus);
    let state = m.gallery.read(&h.app).state();
    let (last, checks) = (state.containers.last_command, state.containers.checks);
    // F10 from the shell focuses the bar; Down opens File.
    h.key("f10");
    h.run_until_idle(300);
    assert_eq!(h.app.focused(), h.app.find("gallery-menubar"));
    h.key("right");
    h.key("down");
    h.run_until_idle(300);
    let popup = m.env.menus().expect("a menu opened");
    assert!(popup.read(&h.app).is_open());
    h.key("enter"); // New Scene
    h.run_until_idle(300);
    assert_eq!(last.read(&h.app), "File › New Scene");
    // Alt+V, then Show Grid (the first item) flips its check mark.
    assert!(*checks.grid.read(&h.app));
    h.key("alt-v");
    h.run_until_idle(300);
    h.key("enter");
    h.run_until_idle(300);
    assert!(!*checks.grid.read(&h.app));
    assert_eq!(last.read(&h.app), "View › Show Grid");
    // The context menu opens on the area with Shift+F10.
    let area = h.app.find("context-area").unwrap();
    m.env.focus(&mut h.app, area);
    h.key("shift-f10");
    h.run_until_idle(300);
    assert!(popup.read(&h.app).is_open());
    h.key("escape");
    h.run_until_idle(300);
    assert!(!popup.read(&h.app).is_open());
    assert!(!h.app.needs_frame());
}

#[test]
fn menu_mnemonics_do_nothing_on_other_pages() {
    let (mut h, m) = host(Page::Buttons);
    h.key("alt-f");
    h.run_until_idle(300);
    assert!(m.env.menus().is_none_or(|p| !p.read(&h.app).is_open()));
    assert_eq!(current(&h, &m), Page::Buttons);
}

#[test]
fn node_canvas_switches_to_the_stress_graph() {
    let (mut h, m) = host(Page::NodeCanvas);
    assert!(h.app.find("node:Mix").is_some());
    let state = m.gallery.read(&h.app).state();
    let graph = state.containers.graph;
    let stress = state.containers.canvases[1];
    let control = page_control(&h, "Graph");
    let (x, y) = segment(&h, control, 1);
    h.click(x, y);
    h.run_until_idle(600);
    assert_eq!(*graph.read(&h.app), 1);
    assert!(h.app.find("node:Node 0").is_some());
    let built = stress.read(&h.app).stats().visible_nodes;
    assert!(built > 20 && built < 200, "{built} cards built");
    assert!(!h.app.needs_frame());
    check_tree(&h.app, Page::NodeCanvas);
}

#[test]
fn titlebar_chrome_is_a_drag_region() {
    use livspeil_ui_core::WindowRequest;
    use livspeil_ui_gallery::{Chrome, mount_with};
    let mut app = App::new();
    let bridge = PaintBridge::new(livspeil_ui_paint::FontSystem::test());
    app.set_text_measurer(bridge.text.measurer());
    let m = mount_with(
        &mut app,
        FontSystem::test(),
        Page::Buttons,
        Chrome::Titlebar,
    );
    let mut h = TestHost::new(app, Size::new(WINDOW_SIZE.0, WINDOW_SIZE.1));
    h.run_until_idle(300);
    assert_eq!(m.gallery.read(&h.app).chrome(), Chrome::Titlebar);
    let bar = h.app.bounds(h.app.find("titlebar").unwrap()).unwrap();
    assert!(bar.y.abs() < 0.5 && bar.width > 900.0);
    h.click(bar.center().x, bar.center().y);
    assert_eq!(h.app.take_window_requests(), vec![WindowRequest::Drag]);
    h.advance(std::time::Duration::from_millis(100));
    h.click(bar.center().x, bar.center().y);
    assert_eq!(h.app.take_window_requests(), vec![WindowRequest::Zoom]);
    // The sidebar starts below the bar; with system chrome there is none.
    let pages = h.app.bounds(h.app.find("pages").unwrap()).unwrap();
    assert!(pages.y > bar.bottom());
    let (h2, _) = host(Page::Buttons);
    assert!(h2.app.find("titlebar").is_none());
}
