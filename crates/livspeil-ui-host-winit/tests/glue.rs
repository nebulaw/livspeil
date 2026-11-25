// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The host's window-independent glue driven with real widgets in a
//! `TestHost`: accessibility (initial tree, incremental updates, action
//! routing), the focused text input and its caret, cursor shapes, the
//! keymap hot reload and the clipboard bridge.

use std::time::{Duration, Instant};

use livspeil_ui_a11y::accesskit::{Action, ActionRequest, NodeId, Role as AkRole, TreeId};
use livspeil_ui_a11y::node_id;
use livspeil_ui_core::{
    App, CursorShape, Element, Keymap, Keystroke, Model, Render, TestHost, View, ViewContext, div,
};
use livspeil_ui_host_winit::widget_platform::{
    CaretBlink, apply_config_event, role_cursor, scene_without, text_focus,
};
use livspeil_ui_host_winit::{Accessibility, MemoryClipboard, WidgetClipboard};
use livspeil_ui_layout::{Point, Size};
use livspeil_ui_theme::ConfigEvent;
use livspeil_ui_widgets::{Button, Env, NumberField, Slider, TextField, Toggle};

struct Form {
    count: Model<u32>,
    on: Model<bool>,
    text: View<TextField>,
    slider: View<Slider>,
    number: View<NumberField>,
}

impl Render for Form {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let count = self.count;
        let on = self.on;
        let n = *count.read(cx);
        let is_on = *on.read(cx);
        div()
            .flex_col()
            .p(16.0)
            .gap(12.0)
            .child(
                Button::new(format!("Clicked {n}"))
                    .on_click(move |_, cx| {
                        count.update(cx, |c, cx| {
                            *c += 1;
                            cx.notify();
                        });
                    })
                    .render(&theme),
            )
            .child(
                Toggle::new(is_on)
                    .label("Wi-Fi")
                    .on_change(move |v, cx| {
                        on.update(cx, |o, cx| {
                            *o = v;
                            cx.notify();
                        });
                    })
                    .render(&theme),
            )
            .child(Button::new("Off").disabled(true).render(&theme))
            .child(Element::view(&self.text))
            .child(Element::view(&self.slider))
            .child(Element::view(&self.number))
    }
}

struct Fixture {
    host: TestHost,
    env: Env,
    count: Model<u32>,
    text: View<TextField>,
}

fn fixture() -> Fixture {
    let mut app = App::new();
    let env = Env::headless(&mut app);
    let count = app.new_model(0_u32);
    let on = app.new_model(false);
    let text = app.new_view(TextField::new(&env, "hello").width(160.0).name("text"));
    let value = app.new_model(0.5_f32);
    let slider = app.new_view(Slider::new(value, 0.0, 1.0, 0.1).label("Volume"));
    let num = app.new_model(2.0_f64);
    let number = app.new_view(
        NumberField::new(&env, num, 0.0, 10.0, 1.0)
            .label("X")
            .name("number"),
    );
    let content = app.new_view(Form {
        count,
        on,
        text,
        slider,
        number,
    });
    let root = env.root(&mut app, content);
    app.set_root(root);
    let mut host = TestHost::new(app, Size::new(400.0, 400.0));
    host.run_until_idle(100);
    Fixture {
        host,
        env,
        count,
        text,
    }
}

fn center(h: &TestHost, name: &str) -> Point {
    let id = h.app.find(name).unwrap_or_else(|| panic!("no {name}"));
    h.app.bounds(id).unwrap().center()
}

#[test]
fn initial_tree_is_named_after_the_window() {
    let f = fixture();
    let mut a = Accessibility::new("Livspeil Gallery");
    let t = a.initial_tree(&f.host.app, 2.0);
    let info = t.tree.as_ref().expect("full tree");
    let (_, root) = t.nodes.iter().find(|(id, _)| *id == info.root).unwrap();
    assert_eq!(root.role(), AkRole::Window);
    assert_eq!(root.label(), Some("Livspeil Gallery"));
    assert!(t.nodes.len() > 6, "{}", t.nodes.len());
    let roles: Vec<AkRole> = t.nodes.iter().map(|(_, n)| n.role()).collect();
    for r in [
        AkRole::Button,
        AkRole::Switch,
        AkRole::TextInput,
        AkRole::Slider,
        AkRole::SpinButton,
    ] {
        assert!(roles.contains(&r), "{r:?} missing");
    }
    // Physical pixels at scale 2.
    let b = f
        .host
        .app
        .bounds(f.host.app.find("button:Clicked 0").unwrap())
        .unwrap();
    let id = node_id(f.host.app.find("button:Clicked 0").unwrap());
    let (_, n) = t.nodes.iter().find(|(i, _)| *i == id).unwrap();
    assert_eq!(n.bounds().unwrap().x0, f64::from(b.x) * 2.0);
}

#[test]
fn updates_are_incremental_and_actions_are_routed() {
    let mut f = fixture();
    let mut a = Accessibility::new("Test");
    // Inactive: nothing is sent.
    assert!(a.update(&f.host.app, 1.0).is_none());
    let full = a.initial_tree(&f.host.app, 1.0);
    let total = full.nodes.len();
    assert!(a.update(&f.host.app, 1.0).is_none(), "no change, no update");

    // Toggle through an accessibility action: only changed nodes are sent.
    let toggle = f.host.app.find("toggle").unwrap();
    let request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: node_id(toggle),
        data: None,
    };
    assert_eq!(
        Accessibility::handle_action(&mut f.host.app, request),
        Ok(true)
    );
    f.host.run_until_idle(100);
    let u = a.update(&f.host.app, 1.0).expect("toggle changed");
    assert!(u.tree.is_none(), "incremental");
    assert!(
        !u.nodes.is_empty() && u.nodes.len() < total,
        "{}",
        u.nodes.len()
    );
    let (_, sw) = u
        .nodes
        .iter()
        .find(|(id, _)| *id == node_id(toggle))
        .unwrap();
    assert_eq!(
        sw.toggled(),
        Some(livspeil_ui_a11y::accesskit::Toggled::True)
    );

    // Press the button: its label changes.
    let button = f.host.app.find("button:Clicked 0").unwrap();
    let request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: node_id(button),
        data: None,
    };
    assert_eq!(
        Accessibility::handle_action(&mut f.host.app, request),
        Ok(true)
    );
    f.host.run_until_idle(100);
    assert_eq!(*f.count.read(&f.host.app), 1);
    let u = a.update(&f.host.app, 1.0).expect("label changed");
    assert!(u.nodes.iter().any(|(_, n)| n.label() == Some("Clicked 1")));

    // Focus moves are reported.
    let request = ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: node_id(toggle),
        data: None,
    };
    assert_eq!(
        Accessibility::handle_action(&mut f.host.app, request),
        Ok(true)
    );
    f.host.run_until_idle(100);
    let u = a.update(&f.host.app, 1.0).expect("focus changed");
    assert_eq!(u.focus, node_id(toggle));

    // Unknown targets are errors, not panics.
    let request = ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: NodeId(12345),
        data: None,
    };
    assert!(Accessibility::handle_action(&mut f.host.app, request).is_err());

    // Deactivation stops updates; reactivation sends everything again.
    a.deactivate();
    assert!(a.update(&f.host.app, 1.0).is_none());
    assert!(a.initial_tree(&f.host.app, 1.0).tree.is_some());
}

#[test]
fn text_focus_follows_the_caret() {
    let mut f = fixture();
    assert_eq!(text_focus(&f.host.app), None);
    let p = center(&f.host, "text");
    f.host.click(p.x, p.y);
    f.host.run_until_idle(100);
    let t = text_focus(&f.host.app).expect("text field focused");
    let caret = t.caret.expect("caret drawn");
    let field = f.host.app.bounds(t.field).unwrap();
    assert_eq!(Some(t.area), f.host.app.bounds(caret));
    assert!(t.area.x >= field.x && t.area.x <= field.x + field.width);
    // Typing moves the caret right.
    f.host.key("end");
    f.host.type_text("!!");
    f.host.run_until_idle(100);
    let t2 = text_focus(&f.host.app).unwrap();
    assert!(t2.area.x > t.area.x, "{:?} -> {:?}", t.area, t2.area);
    // A selection has no caret; the IME goes to the field.
    f.host.key("ctrl-a");
    f.host.run_until_idle(100);
    let t3 = text_focus(&f.host.app).unwrap();
    assert_eq!(t3.caret, None);
    assert_eq!(t3.area, field);
    // Focus elsewhere: no text input.
    let b = center(&f.host, "toggle");
    f.host.click(b.x, b.y);
    f.host.run_until_idle(100);
    assert_eq!(text_focus(&f.host.app), None);
    // A number field is a text input only while editing.
    let n = center(&f.host, "number");
    f.host.click(n.x, n.y);
    f.host.run_until_idle(100);
    let t = text_focus(&f.host.app).expect("number field editing");
    assert_eq!(Some(t.field), f.host.app.find("number"));
    assert_eq!(
        role_cursor(&f.host.app, n),
        CursorShape::Text,
        "editing shows the I-beam"
    );
    f.host.key("escape");
    f.host.run_until_idle(100);
    assert_eq!(text_focus(&f.host.app), None);
}

#[test]
fn caret_blinks_only_while_a_text_field_is_focused() {
    let mut f = fixture();
    let t0 = Instant::now();
    let ms = Duration::from_millis;
    let mut blink = CaretBlink::with_timing(t0, ms(500), ms(10_000));
    let caret_of = |app: &App| {
        text_focus(app).and_then(|t| {
            let id = t.caret?;
            Some((id, app.bounds(id)?))
        })
    };
    blink.set_caret(t0, caret_of(&f.host.app));
    assert!(!blink.is_running(), "nothing focused: no timer");
    let p = center(&f.host, "text");
    f.host.click(p.x, p.y);
    f.host.run_until_idle(100);
    blink.set_caret(t0, caret_of(&f.host.app));
    assert!(blink.is_running());
    assert!(blink.poll(t0 + ms(500)));
    let hidden = blink.hidden_element().expect("caret hidden");
    // The hidden caret is left out of the frame.
    let scene = f.host.scene().unwrap().clone();
    let without = scene_without(&scene, hidden);
    assert_eq!(without.commands.len() + 1, scene.commands.len());
    // Unfocus: the timer stops and the caret is shown.
    let b = center(&f.host, "toggle");
    f.host.click(b.x, b.y);
    f.host.run_until_idle(100);
    assert!(blink.set_caret(t0 + ms(600), caret_of(&f.host.app)));
    assert!(!blink.is_running());
    assert_eq!(blink.hidden_element(), None);
}

#[test]
fn cursor_shapes_follow_roles() {
    let f = fixture();
    let app = &f.host.app;
    assert_eq!(
        role_cursor(app, center(&f.host, "button:Clicked 0")),
        CursorShape::Pointer
    );
    assert_eq!(
        role_cursor(app, center(&f.host, "toggle")),
        CursorShape::Pointer
    );
    assert_eq!(role_cursor(app, center(&f.host, "text")), CursorShape::Text);
    assert_eq!(
        role_cursor(app, center(&f.host, "number")),
        CursorShape::ResizeEw
    );
    let slider = app.find("slider").expect("slider");
    let s = app.bounds(slider).unwrap().center();
    assert_eq!(role_cursor(app, s), CursorShape::ResizeEw);
    // Disabled controls and empty space: the arrow.
    assert_eq!(
        role_cursor(app, center(&f.host, "button:Off")),
        CursorShape::Default
    );
    assert_eq!(
        role_cursor(app, Point::new(395.0, 395.0)),
        CursorShape::Default
    );
}

#[test]
fn keymap_reload_keeps_app_bindings() {
    let mut f = fixture();
    let base = f.host.app.keymap().clone();
    let slider_binding = base.bindings().len();
    assert!(slider_binding > Keymap::defaults().bindings().len());
    apply_config_event(
        &mut f.host.app,
        &base,
        ConfigEvent::Raw {
            tag: "keymap",
            contents: Some("[bindings]\n\"ctrl-k\" = \"app.custom\"\n".to_owned()),
        },
    )
    .unwrap();
    let km = f.host.app.keymap();
    assert_eq!(km.bindings().len(), slider_binding + 1);
    let k = Keystroke::parse("ctrl-k").unwrap();
    assert_eq!(km.action_for(&k, &[]).as_deref(), Some("app.custom"));
    // Removing the file restores the app's keymap (not bare defaults).
    apply_config_event(
        &mut f.host.app,
        &base,
        ConfigEvent::Raw {
            tag: "keymap",
            contents: None,
        },
    )
    .unwrap();
    assert_eq!(f.host.app.keymap(), &base);
    // Parse errors change nothing.
    assert!(
        apply_config_event(
            &mut f.host.app,
            &base,
            ConfigEvent::Raw {
                tag: "keymap",
                contents: Some("[bindings\n".to_owned()),
            },
        )
        .is_err()
    );
    assert_eq!(f.host.app.keymap(), &base);
}

#[test]
fn widget_clipboard_reaches_the_host_clipboard() {
    let mut f = fixture();
    f.env
        .set_clipboard(Box::new(WidgetClipboard::new(Box::new(MemoryClipboard(
            Some("pasted".to_owned()),
        )))));
    let p = center(&f.host, "text");
    f.host.click(p.x, p.y);
    f.host.run_until_idle(100);
    // Copy writes through to the host clipboard.
    f.host.key("ctrl-a");
    f.host.key("ctrl-c");
    assert_eq!(f.env.clipboard_text().as_deref(), Some("hello"));
    // Paste reads from it.
    f.env.set_clipboard_text("world");
    f.host.key("ctrl-a");
    f.host.key("ctrl-v");
    f.host.run_until_idle(100);
    assert_eq!(f.text.read(&f.host.app).text(), "world");
}
