// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! AccessKit trees from a sample view with `livspeil-ui-widgets`: structure,
//! incremental updates, focus, action round trips, hidden/disabled and a
//! property test over random mutations.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, TreeId, TreeUpdate};
use livspeil_testkit::assert_golden;
use livspeil_ui_a11y::{A11yError, A11yTree, handle_action, node_id};
use livspeil_ui_core::{
    App, Element, ElementId, Model, Render, Role, SemanticActionError, SemanticValue, TestHost,
    View, ViewContext, div, text,
};
use livspeil_ui_layout::Size;
use livspeil_ui_widgets::{Button, Slider, Toggle};
use proptest::prelude::*;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

#[derive(Clone, Debug)]
struct State {
    status: String,
    items: Vec<String>,
    hidden_extra: bool,
    disabled: bool,
    expanded: bool,
}

struct Demo {
    state: Model<State>,
    count: Model<i32>,
    on: Model<bool>,
    slider: View<Slider>,
}

impl Render for Demo {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let (count, on) = (self.count, self.on);
        let st = self.state.read(cx).clone();
        let n = *count.read(cx);
        let is_on = *on.read(cx);
        let theme = cx.theme().clone();
        div()
            .name("demo")
            .flex_col()
            .p(8.0)
            .gap(4.0)
            .child(text("Settings").name("title").role(Role::Heading))
            .child(text(st.status.clone()).name("status"))
            .child(text(format!("Count: {n}")).name("count"))
            .child(
                Button::new("Increment")
                    .on_click(move |_, cx| {
                        count.update(cx, |c, cx| {
                            *c += 1;
                            cx.notify();
                        });
                    })
                    .render(&theme)
                    .key("inc"),
            )
            .child(
                Button::new("Delete")
                    .destructive()
                    .disabled(st.disabled)
                    .on_click(move |_, cx| {
                        count.update(cx, |c, cx| {
                            *c -= 100;
                            cx.notify();
                        });
                    })
                    .render(&theme)
                    .key("delete")
                    .semantics(|s| s.description("Removes it")),
            )
            .child(
                Toggle::new(is_on)
                    .on_change(move |v, cx| {
                        on.update(cx, |o, cx| {
                            *o = v;
                            cx.notify();
                        });
                    })
                    .render(&theme)
                    .key("wifi")
                    .semantics(|s| s.label("Wi-Fi")),
            )
            .child(Element::view(&self.slider))
            .child(
                div()
                    .key("list")
                    .flex_col()
                    .role(Role::List)
                    .semantics(|s| s.label("Rooms").expanded(st.expanded))
                    .children(st.items.iter().map(|it| {
                        div()
                            .key(it.clone())
                            .role(Role::ListItem)
                            .semantics(|s| s.selected(it == "Kitchen"))
                            .child(text(it.clone()))
                    })),
            )
            .child(
                div()
                    .key("secret")
                    .semantics(|s| s.hidden(!st.hidden_extra))
                    .child(text("Secret").name("secret-text")),
            )
            .child(
                // Transparent wrapper: its semantic child attaches to `demo`'s
                // nearest exposed ancestor.
                div().key("wrap").child(
                    div()
                        .key("name-field")
                        .role(Role::TextInput)
                        .focusable()
                        .size(120.0, 20.0)
                        .semantics(|s| s.label("Name").value(SemanticValue::text("Ada"))),
                ),
            )
    }
}

struct Fixture {
    host: TestHost,
    state: Model<State>,
    count: Model<i32>,
    on: Model<bool>,
    value: Model<f32>,
}

impl Fixture {
    fn new() -> Self {
        let mut app = App::new();
        let state = app.new_model(State {
            status: "Ready".into(),
            items: vec!["Hall".into(), "Kitchen".into(), "Study".into()],
            hidden_extra: false,
            disabled: true,
            expanded: true,
        });
        let count = app.new_model(0);
        let on = app.new_model(false);
        let value = app.new_model(50.0_f32);
        let slider = app.new_view(Slider::new(value, 0.0, 100.0, 5.0).label("Volume"));
        let demo = app.new_view(Demo {
            state,
            count,
            on,
            slider,
        });
        app.set_root(demo);
        let mut host = TestHost::new(app, Size::new(400.0, 400.0));
        host.run_until_idle(20);
        Fixture {
            host,
            state,
            count,
            on,
            value,
        }
    }

    fn id(&self, name: &str) -> ElementId {
        match self.host.app.find(name) {
            Some(id) => id,
            None => panic!("no element {name}"),
        }
    }

    fn edit(&mut self, f: impl FnOnce(&mut State)) {
        self.state.update(&mut self.host.app, |s, cx| {
            f(s);
            cx.notify();
        });
        self.host.run_until_idle(100);
    }

    fn act(
        &mut self,
        name: &str,
        action: Action,
        data: Option<ActionData>,
    ) -> Result<bool, A11yError> {
        let target = node_id(self.id(name));
        let r = handle_action(
            &mut self.host.app,
            ActionRequest {
                action,
                target_tree: TreeId::ROOT,
                target_node: target,
                data,
            },
        );
        self.host.run_until_idle(100);
        r
    }
}

fn sent_ids(u: &TreeUpdate) -> Vec<NodeId> {
    u.nodes.iter().map(|(id, _)| *id).collect()
}

fn find_node(u: &TreeUpdate, id: NodeId) -> &Node {
    match u.nodes.iter().find(|(i, _)| *i == id) {
        Some((_, n)) => n,
        None => panic!("{id:?} not in update"),
    }
}

#[test]
fn full_tree_snapshot() {
    let f = Fixture::new();
    let mut tree = A11yTree::new();
    let u = tree.update(&f.host.app, 1.0).unwrap();
    let info = u.tree.as_ref().unwrap();
    assert_eq!(info.root, node_id(ElementId::ROOT));
    assert_eq!(info.toolkit_name.as_deref(), Some("Livspeil UI"));
    assert_eq!(u.focus, node_id(ElementId::ROOT));
    assert_eq!(u.nodes.len(), tree.len());
    let dump = tree.dump();
    assert!(!dump.contains("Secret"), "hidden subtree must be dropped");
    assert!(!dump.contains("missing"));
    assert_golden(&golden_dir(), "a11y-tree", &dump);
}

#[test]
fn scale_gives_physical_bounds() {
    let f = Fixture::new();
    let mut a = A11yTree::new();
    let mut b = A11yTree::new();
    a.update(&f.host.app, 1.0).unwrap();
    b.update(&f.host.app, 2.0).unwrap();
    let id = node_id(f.id("inc"));
    let (ra, rb) = (
        a.node(id).unwrap().bounds().unwrap(),
        b.node(id).unwrap().bounds().unwrap(),
    );
    assert!(ra.x1 > ra.x0);
    assert_eq!(
        (rb.x0, rb.y0, rb.x1, rb.y1),
        (ra.x0 * 2.0, ra.y0 * 2.0, ra.x1 * 2.0, ra.y1 * 2.0)
    );
}

#[test]
fn no_change_no_update() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    assert!(tree.update(&f.host.app, 1.0).is_some());
    assert!(tree.update(&f.host.app, 1.0).is_none());
    // A re-render that changes nothing semantic sends nothing.
    f.edit(|_| {});
    assert!(tree.update(&f.host.app, 1.0).is_none());
}

#[test]
fn changing_one_label_updates_exactly_that_node() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    f.edit(|s| s.status = "Saved".into());
    let u = tree.update(&f.host.app, 1.0).unwrap();
    let status = node_id(f.id("status"));
    assert_eq!(sent_ids(&u), vec![status]);
    assert!(u.tree.is_none());
    assert_eq!(find_node(&u, status).value(), Some("Saved"));
}

#[test]
fn reordering_keyed_children_keeps_ids() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    let list = node_id(f.id("list"));
    let before: Vec<NodeId> = tree.node(list).unwrap().children().to_vec();
    let tree_before: HashMap<NodeId, Node> = before
        .iter()
        .flat_map(|i| {
            let n = tree.node(*i).unwrap().clone();
            let kids: Vec<(NodeId, Node)> = n
                .children()
                .iter()
                .map(|c| (*c, tree.node(*c).unwrap().clone()))
                .collect();
            std::iter::once((*i, n)).chain(kids)
        })
        .collect();
    f.edit(|s| s.items.reverse());
    let n0 = tree.len();
    let u = tree.update(&f.host.app, 1.0).unwrap();
    // Items keep their ids: nothing is added or removed. The list changes
    // (child order) and the moved items and their labels change (bounds);
    // the middle item stays put and is not sent.
    assert_eq!(tree.stats().removed, 0);
    assert_eq!(tree.len(), n0);
    let ids = sent_ids(&u);
    assert_eq!(ids.len(), 5, "list + 2 moved items + their labels");
    assert!(ids.contains(&list));
    assert!(!ids.contains(&node_id(f.id("Kitchen"))));
    for (id, n) in &u.nodes {
        if *id != list {
            let old = tree_before.get(id).unwrap();
            assert_eq!(old.value(), n.value());
            assert_eq!(old.role(), n.role());
            assert_ne!(old.bounds(), n.bounds());
        }
    }
    let after: Vec<NodeId> = find_node(&u, list).children().to_vec();
    let mut rev = before.clone();
    rev.reverse();
    assert_eq!(after, rev);
}

#[test]
fn removing_and_adding_children() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    let list = node_id(f.id("list"));
    let n0 = tree.len();
    f.edit(|s| {
        s.items.retain(|i| i != "Study");
        s.items.push("Garage".into());
    });
    let u = tree.update(&f.host.app, 1.0).unwrap();
    let ids = sent_ids(&u);
    assert!(ids.contains(&list));
    let garage = node_id(f.id("Garage"));
    assert!(ids.contains(&garage));
    // Item + its text label added, item + label removed.
    assert_eq!(tree.stats().removed, 2);
    assert_eq!(tree.len(), n0);
}

#[test]
fn focus_changes_are_reported() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    f.host.app.focus_next();
    let u = tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(u.focus, node_id(f.id("inc")));
    assert!(u.nodes.is_empty(), "focus alone sends no nodes");
    assert!(tree.dump().contains("*Button \"Increment\""));
    f.host.app.blur();
    let u = tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(u.focus, node_id(ElementId::ROOT));
}

#[test]
fn click_fires_button_handler() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(f.act("inc", Action::Click, None), Ok(true));
    assert_eq!(*f.count.read(&f.host.app), 1);
    let u = tree.update(&f.host.app, 1.0).unwrap();
    let count = node_id(f.id("count"));
    assert_eq!(sent_ids(&u), vec![count]);
    assert_eq!(find_node(&u, count).value(), Some("Count: 1"));
}

#[test]
fn click_flips_toggle() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    let wifi = node_id(f.id("wifi"));
    assert_eq!(
        tree.node(wifi).unwrap().toggled(),
        Some(accesskit::Toggled::False)
    );
    assert_eq!(f.act("wifi", Action::Click, None), Ok(true));
    assert!(*f.on.read(&f.host.app));
    let u = tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(sent_ids(&u), vec![wifi]);
    assert_eq!(
        find_node(&u, wifi).toggled(),
        Some(accesskit::Toggled::True)
    );
}

#[test]
fn slider_increment_decrement_and_set_value() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    let slider = node_id(f.id("slider"));
    assert_eq!(f.act("slider", Action::Increment, None), Ok(true));
    assert!((*f.value.read(&f.host.app) - 55.0).abs() < 1e-4);
    let u = tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(sent_ids(&u), vec![slider]);
    assert_eq!(find_node(&u, slider).numeric_value(), Some(55.0));
    assert_eq!(f.act("slider", Action::Decrement, None), Ok(true));
    assert_eq!(f.act("slider", Action::Decrement, None), Ok(true));
    assert!((*f.value.read(&f.host.app) - 45.0).abs() < 1e-4);
    // Set value snaps to the step.
    assert_eq!(
        f.act(
            "slider",
            Action::SetValue,
            Some(ActionData::NumericValue(71.0))
        ),
        Ok(true)
    );
    assert!((*f.value.read(&f.host.app) - 70.0).abs() < 1e-4);
    tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(tree.node(slider).unwrap().numeric_value(), Some(70.0));
    assert_eq!(
        f.act("slider", Action::SetValue, None),
        Err(A11yError::BadData(Action::SetValue))
    );
}

#[test]
fn focus_action_shows_focus() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(f.act("slider", Action::Focus, None), Ok(true));
    assert_eq!(f.host.app.focused(), Some(f.id("slider")));
    assert!(f.host.app.focus_visible());
    let u = tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(u.focus, node_id(f.id("slider")));
    assert_eq!(f.act("slider", Action::Blur, None), Ok(true));
    assert_eq!(f.host.app.focused(), None);
    // Not focusable.
    assert!(matches!(
        f.act("count", Action::Focus, None),
        Err(A11yError::Semantic(SemanticActionError::Unsupported(..)))
    ));
}

#[test]
fn disabled_is_exposed_but_inert() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    let del = node_id(f.id("delete"));
    let n = tree.node(del).unwrap();
    assert!(n.is_disabled());
    assert!(!n.supports_action(Action::Click));
    assert!(!n.supports_action(Action::Focus));
    assert_eq!(n.description(), Some("Removes it"));
    assert_eq!(
        f.act("delete", Action::Click, None),
        Err(A11yError::Semantic(SemanticActionError::Disabled(
            f.id("delete")
        )))
    );
    assert_eq!(*f.count.read(&f.host.app), 0);
    f.edit(|s| s.disabled = false);
    let u = tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(sent_ids(&u), vec![del]);
    assert!(find_node(&u, del).supports_action(Action::Click));
    assert_eq!(f.act("delete", Action::Click, None), Ok(true));
    assert_eq!(*f.count.read(&f.host.app), -100);
}

#[test]
fn hidden_subtrees_come_and_go() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    let secret = node_id(f.id("secret-text"));
    assert!(tree.node(secret).is_none());
    assert!(matches!(
        f.act("secret-text", Action::Click, None),
        Err(A11yError::Semantic(SemanticActionError::Hidden(_)))
    ));
    f.edit(|s| s.hidden_extra = true);
    let u = tree.update(&f.host.app, 1.0).unwrap();
    let demo = node_id(f.id("demo"));
    // The text appears and the window (nearest exposed ancestor) lists it.
    let mut ids = sent_ids(&u);
    ids.sort_unstable_by_key(|i| i.0);
    let mut want = vec![node_id(ElementId::ROOT), secret];
    want.sort_unstable_by_key(|i| i.0);
    assert_eq!(ids, want, "demo is transparent: {demo:?}");
    f.edit(|s| s.hidden_extra = false);
    let u = tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(sent_ids(&u), vec![node_id(ElementId::ROOT)]);
    assert_eq!(tree.stats().removed, 1);
}

#[test]
fn state_flags_update() {
    let mut f = Fixture::new();
    let mut tree = A11yTree::new();
    tree.update(&f.host.app, 1.0).unwrap();
    let list = node_id(f.id("list"));
    assert_eq!(tree.node(list).unwrap().is_expanded(), Some(true));
    f.edit(|s| s.expanded = false);
    let u = tree.update(&f.host.app, 1.0).unwrap();
    assert_eq!(sent_ids(&u), vec![list]);
    assert_eq!(find_node(&u, list).is_expanded(), Some(false));
    let kitchen = node_id(f.id("Kitchen"));
    assert_eq!(tree.node(kitchen).unwrap().is_selected(), Some(true));
}

#[test]
fn bad_requests() {
    let mut f = Fixture::new();
    let other = TreeId(accesskit::Uuid::from_u128(7));
    let r = handle_action(
        &mut f.host.app,
        ActionRequest {
            action: Action::Click,
            target_tree: other,
            target_node: node_id(ElementId::ROOT),
            data: None,
        },
    );
    assert_eq!(r, Err(A11yError::UnknownTree(other)));
    assert_eq!(
        f.act("inc", Action::ShowContextMenu, None),
        Err(A11yError::UnsupportedAction(Action::ShowContextMenu))
    );
    let r = handle_action(
        &mut f.host.app,
        ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: NodeId(12345),
            data: None,
        },
    );
    assert_eq!(
        r,
        Err(A11yError::Semantic(SemanticActionError::UnknownElement(
            ElementId(12345)
        )))
    );
    // Scroll into view with no scroll container: accepted, unhandled.
    assert_eq!(f.act("inc", Action::ScrollIntoView, None), Ok(false));
}

// ---- property test ----------------------------------------------------

#[derive(Clone, Debug)]
enum Op {
    Insert(usize, u8),
    Remove(usize),
    Swap(usize, usize),
    Rename(u8),
    ToggleHidden,
    ToggleDisabled,
    Click,
    Increment,
    FocusNext,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (0usize..8, any::<u8>()).prop_map(|(i, n)| Op::Insert(i, n)),
        (0usize..8).prop_map(Op::Remove),
        (0usize..8, 0usize..8).prop_map(|(a, b)| Op::Swap(a, b)),
        any::<u8>().prop_map(Op::Rename),
        Just(Op::ToggleHidden),
        Just(Op::ToggleDisabled),
        Just(Op::Click),
        Just(Op::Increment),
        Just(Op::FocusNext),
    ]
}

/// What a platform adapter holds: updates applied in order, nodes not
/// reachable from the root dropped.
#[derive(Default)]
struct Mirror {
    nodes: HashMap<NodeId, Node>,
    root: Option<NodeId>,
    focus: Option<NodeId>,
}

impl Mirror {
    fn apply(&mut self, u: TreeUpdate) -> Result<(), String> {
        let mut seen = HashSet::new();
        for (id, _) in &u.nodes {
            if !seen.insert(*id) {
                return Err(format!("duplicate {id:?} in update"));
            }
        }
        if let Some(t) = &u.tree {
            self.root = Some(t.root);
        }
        let root = self.root.ok_or("update before tree info")?;
        let sent: Vec<NodeId> = u.nodes.iter().map(|(i, _)| *i).collect();
        for (id, n) in u.nodes {
            self.nodes.insert(id, n);
        }
        // Reachability and uniqueness of parents.
        let mut parent: HashMap<NodeId, NodeId> = HashMap::new();
        let mut reach = HashSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let n = self.nodes.get(&id).ok_or(format!("child {id:?} missing"))?;
            reach.insert(id);
            for c in n.children() {
                if let Some(p) = parent.insert(*c, id) {
                    return Err(format!("{c:?} has two parents {p:?} and {id:?}"));
                }
                stack.push(*c);
            }
        }
        for id in sent {
            if !reach.contains(&id) {
                return Err(format!("sent {id:?} is not attached"));
            }
        }
        self.nodes.retain(|id, _| reach.contains(id));
        if !self.nodes.contains_key(&u.focus) {
            return Err(format!("focus {:?} not in tree", u.focus));
        }
        self.focus = Some(u.focus);
        Ok(())
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn random_mutations_keep_tree_consistent(ops in proptest::collection::vec(op(), 1..16)) {
        let mut f = Fixture::new();
        let mut tree = A11yTree::new();
        let mut mirror = Mirror::default();
        let first = tree.update(&f.host.app, 1.5).unwrap();
        prop_assert!(mirror.apply(first).is_ok());
        let mut next = 0u32;
        for op in ops {
            match op {
                Op::Insert(i, n) => {
                    next += 1;
                    f.edit(|s| {
                        let i = i.min(s.items.len());
                        s.items.insert(i, format!("item{n}-{next}"));
                    });
                }
                Op::Remove(i) => f.edit(|s| {
                    if i < s.items.len() {
                        s.items.remove(i);
                    }
                }),
                Op::Swap(a, b) => f.edit(|s| {
                    if a < s.items.len() && b < s.items.len() {
                        s.items.swap(a, b);
                    }
                }),
                Op::Rename(n) => f.edit(|s| s.status = format!("status {n}")),
                Op::ToggleHidden => f.edit(|s| s.hidden_extra = !s.hidden_extra),
                Op::ToggleDisabled => f.edit(|s| s.disabled = !s.disabled),
                Op::Click => { let _ = f.act("inc", Action::Click, None); }
                Op::Increment => { let _ = f.act("slider", Action::Increment, None); }
                Op::FocusNext => { f.host.app.focus_next(); }
            }
            if let Some(u) = tree.update(&f.host.app, 1.5) {
                let r = mirror.apply(u);
                prop_assert!(r.is_ok(), "{:?}", r);
            }
            // The incrementally maintained mirror equals a fresh full tree.
            let mut fresh = A11yTree::new();
            let full = fresh.update(&f.host.app, 1.5).unwrap();
            let full_nodes: HashMap<NodeId, Node> = full.nodes.into_iter().collect();
            prop_assert_eq!(full_nodes.len(), mirror.nodes.len());
            prop_assert!(full_nodes == mirror.nodes);
            prop_assert_eq!(Some(full.focus), mirror.focus);
            prop_assert_eq!(fresh.dump(), tree.dump());
        }
    }
}
