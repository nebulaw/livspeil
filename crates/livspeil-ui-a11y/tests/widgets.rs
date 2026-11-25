// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The AccessKit roles of `livspeil-ui-widgets` controls that need more
//! than the basic roles: radio groups, number fields (spin buttons), an
//! open select (combo box, list box, options) and a tooltip.

use std::path::{Path, PathBuf};

use livspeil_testkit::assert_golden;
use livspeil_ui_a11y::{A11yTree, map_role};
use livspeil_ui_core::{App, Element, Model, Render, Role, TestHost, View, ViewContext, div};
use livspeil_ui_layout::Size;
use livspeil_ui_widgets::{Env, NumberField, RadioGroup, Select, Tooltip};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

struct Controls {
    env: Env,
    size: Model<Option<usize>>,
    number: View<NumberField>,
    select: View<Select>,
}

impl Render for Controls {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let size = self.size;
        let chosen = *size.read(cx);
        div()
            .flex_col()
            .p(8.0)
            .gap(8.0)
            .child(
                RadioGroup::new(["Small", "Large"], chosen)
                    .label("Size")
                    .on_change(move |i, cx| {
                        size.update(cx, |s, cx| {
                            *s = Some(i);
                            cx.notify();
                        });
                    })
                    .render(&theme),
            )
            .child(Element::view(&self.number))
            .child(Element::view(&self.select))
            .child(
                Tooltip::new("Opens the curve editor").attach(
                    &self.env,
                    div()
                        .key("curve-help")
                        .size(24.0, 24.0)
                        .focusable()
                        .role(Role::Button)
                        .semantics(|s| s.label("Help")),
                ),
            )
    }
}

#[test]
fn widget_roles_snapshot() {
    let mut app = App::new();
    let env = Env::headless(&mut app);
    let size = app.new_model(Some(0));
    let n = app.new_model(2.5_f64);
    let number = app.new_view(NumberField::new(&env, n, 0.0, 10.0, 0.5).label("Scale"));
    let curve = app.new_model(Some(1));
    let select =
        app.new_view(Select::new(&env, curve, ["Linear", "Ease", "Spring"]).label("Curve"));
    let content = app.new_view(Controls {
        env: env.clone(),
        size,
        number,
        select,
    });
    let root = env.root(&mut app, content);
    app.set_root(root);
    let mut h = TestHost::new(app, Size::new(400.0, 400.0));
    h.run_until_idle(100);

    // Open the select, then show the tooltip.
    let name = Select::element_name(&select, &h.app);
    let trigger = h.app.find(&name).unwrap();
    let c = h.app.bounds(trigger).unwrap().center();
    h.click(c.x, c.y);
    h.run_until_idle(200);
    assert!(select.read(&h.app).is_open());
    let help = h.app.bounds(h.app.find("curve-help").unwrap()).unwrap();
    let mut tree = A11yTree::new();
    tree.update(&h.app, 1.0).unwrap();
    assert_golden(&golden_dir(), "a11y-widgets-select", &tree.dump());

    h.key("escape");
    h.run_until_idle(200);
    assert!(!select.read(&h.app).is_open());
    h.pointer_move(help.center().x, help.center().y);
    for _ in 0..120 {
        h.step();
    }
    h.run_until_idle(200);
    let mut tree = A11yTree::new();
    tree.update(&h.app, 1.0).unwrap();
    let dump = tree.dump();
    assert!(dump.contains("Tooltip"), "{dump}");
    assert_golden(&golden_dir(), "a11y-widgets-tooltip", &dump);
}

#[test]
fn new_roles_map_to_accesskit() {
    use accesskit::Role as R;
    for (core, ak) in [
        (Role::RadioButton, R::RadioButton),
        (Role::RadioGroup, R::RadioGroup),
        (Role::SpinButton, R::SpinButton),
        (Role::ComboBox, R::ComboBox),
        (Role::ListBox, R::ListBox),
        (Role::ListBoxOption, R::ListBoxOption),
        (Role::Tooltip, R::Tooltip),
        (Role::MenuBar, R::MenuBar),
        (Role::Menu, R::Menu),
        (Role::MenuItem, R::MenuItem),
        (Role::MenuItemCheckBox, R::MenuItemCheckBox),
        (Role::Separator, R::Splitter),
    ] {
        assert_eq!(map_role(core), ak, "{core:?}");
    }
}
