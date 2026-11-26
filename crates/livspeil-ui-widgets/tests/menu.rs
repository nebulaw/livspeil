// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Menus: keyboard flows, submenu hover intent, context menu placement,
//! menu bar switching, shortcuts from the keymap.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{center, find, host_sized, pointer, wait};
use livspeil_ui_core::{
    Element, IconRun, Keymap, Keystroke, Modifiers, MouseButton, PaintCmd, PointerKind, TestHost,
    View, div,
};
use livspeil_ui_layout::{Point, Rect, Size, Vec2};
use livspeil_ui_widgets::{
    ContextMenu, Env, Menu, MenuAnchor, MenuBar, MenuItem, MenuPopup, format_keystroke, open_menu,
    place_menu, place_submenu, shortcut_for,
};

type Log = Rc<RefCell<Vec<String>>>;

fn edit_menu(log: &Log) -> Menu {
    let l = |s: &'static str| {
        let log = log.clone();
        move |_: &mut livspeil_ui_core::App| log.borrow_mut().push(s.to_owned())
    };
    Menu::new()
        .item(
            MenuItem::new("Save")
                .action("app.save")
                .on_select(l("save")),
        )
        .item(MenuItem::new("Redo").disabled(true))
        .separator()
        .item(
            MenuItem::new("Show Grid")
                .checked(true)
                .on_select(l("grid")),
        )
        .submenu(
            "Transform",
            Menu::new()
                .item(MenuItem::new("Rotate").on_select(l("rotate")))
                .item(MenuItem::new("Scale").on_select(l("scale"))),
        )
        .item(
            MenuItem::new("Paste")
                .shortcut("Ctrl+V")
                .on_select(l("paste")),
        )
}

struct Fx {
    h: TestHost,
    env: Env,
    log: Log,
    bar: View<MenuBar>,
}

fn fixture() -> Fx {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut out = None;
    let log2 = log.clone();
    let (mut h, env) = host_sized(Size::new(600.0, 400.0), |app, env| {
        let mut km = app.keymap().clone();
        km.extend(&Keymap::parse("[bindings]\n\"ctrl-s\" = \"app.save\"\n").unwrap());
        app.set_keymap(km);
        let bar = app.new_view(
            MenuBar::new(env)
                .menu("Livspeil", Menu::new().item(MenuItem::new("About")))
                .menu("Edit", edit_menu(&log2))
                .menu("View", Menu::new().item(MenuItem::new("Zoom In"))),
        );
        env.set_menubar(bar);
        out = Some(bar);
        let env = env.clone();
        let log3 = log2.clone();
        Box::new(move |_| {
            let area = div().name("area").size(600.0, 300.0).focusable();
            let area = ContextMenu::new(edit_menu(&log3)).attach(&env, area);
            let log4 = log3.clone();
            div()
                .flex_col()
                .w_full()
                .h_full()
                .on_action("app.save", move |_| log4.borrow_mut().push("action".into()))
                .child(Element::view(&bar))
                .child(area)
                .child(div().name("other").size(100.0, 40.0).focusable())
        })
    });
    h.run_until_idle(100);
    Fx {
        h,
        env,
        log,
        bar: out.unwrap(),
    }
}

impl Fx {
    fn popup(&self) -> View<MenuPopup> {
        self.env.menus().expect("a menu was opened")
    }
    fn depth(&self) -> usize {
        self.env.menus().map_or(0, |p| p.read(&self.h.app).depth())
    }
    fn hl(&self) -> Vec<Option<usize>> {
        self.popup().read(&self.h.app).highlighted()
    }
    fn key(&mut self, k: &str) {
        self.h.key(k);
        self.h.run_until_idle(100);
    }
    fn right_click(&mut self, p: Point) {
        pointer(
            &mut self.h,
            PointerKind::Down(MouseButton::Right),
            p,
            Modifiers::NONE,
        );
        pointer(
            &mut self.h,
            PointerKind::Up(MouseButton::Right),
            p,
            Modifiers::NONE,
        );
        self.h.run_until_idle(100);
    }
    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

#[test]
fn placement_flips_and_clamps() {
    let vp = Vec2::new(400.0, 300.0);
    let s = Vec2::new(150.0, 100.0);
    assert_eq!(
        place_menu(Point::new(10.0, 10.0), s, vp),
        Rect::new(10.0, 10.0, 150.0, 100.0)
    );
    let r = place_menu(Point::new(390.0, 290.0), s, vp);
    assert_eq!((r.x, r.y), (240.0, 190.0), "flipped left and up");
    let big = place_menu(Point::new(10.0, 250.0), Vec2::new(150.0, 290.0), vp);
    assert_eq!(big.y, 6.0, "clamped when it fits neither way");
    let item = Rect::new(300.0, 100.0, 140.0, 24.0);
    let sub = place_submenu(item, s, Vec2::new(800.0, 600.0));
    assert_eq!((sub.x, sub.y), (444.0, 96.0));
    let sub = place_submenu(item, s, vp);
    assert_eq!(sub.right(), 296.0, "opens to the left at the edge");
}

#[test]
fn shortcuts_come_from_the_keymap() {
    let km =
        Keymap::parse("[bindings]\n\"ctrl-shift-z\" = \"edit.redo\"\n\"f5\" = \"run\"\n").unwrap();
    assert_eq!(
        shortcut_for(&km, "edit.redo").as_deref(),
        Some("Ctrl+Shift+Z")
    );
    assert_eq!(shortcut_for(&km, "run").as_deref(), Some("F5"));
    assert_eq!(shortcut_for(&km, "nope"), None);
    assert_eq!(
        format_keystroke(&Keystroke::parse("alt-delete").unwrap()),
        "Alt+Del"
    );
}

#[test]
fn context_menu_at_pointer_kept_on_screen() {
    let mut f = fixture();
    f.right_click(Point::new(100.0, 100.0));
    assert_eq!(f.depth(), 1);
    let r = f.popup().read(&f.h.app).rects()[0];
    assert_eq!((r.x, r.y), (100.0, 100.0));
    assert!(f.h.app.find("menuitem:Save").is_some());
    // The shortcut of app.save comes from the keymap.
    assert!(
        f.h.app.dump_tree().contains("\"Ctrl+S\""),
        "{}",
        f.h.app.dump_tree()
    );
    // Near the corner: flipped so it stays inside the window.
    f.key("escape");
    assert_eq!(f.depth(), 0);
    f.right_click(Point::new(590.0, 290.0));
    let r = f.popup().read(&f.h.app).rects()[0];
    assert!(
        r.right() <= 590.0 && r.bottom() <= 396.0 && r.y >= 4.0,
        "{r:?}"
    );
    // Pressing outside closes.
    f.h.click(5.0, 395.0);
    f.h.run_until_idle(100);
    assert_eq!(f.depth(), 0);
}

#[test]
fn keyboard_navigation_and_submenus() {
    let mut f = fixture();
    let area = find(&f.h, "area");
    f.h.app.focus(area);
    f.key("shift-f10");
    assert_eq!(f.depth(), 1);
    assert_eq!(
        f.hl(),
        vec![Some(0)],
        "opened from the keyboard: first item"
    );
    f.key("down"); // skips the disabled item and the separator
    assert_eq!(f.hl(), vec![Some(3)]);
    f.key("down");
    assert_eq!(f.hl(), vec![Some(4)]);
    f.key("right");
    assert_eq!(f.depth(), 2);
    assert_eq!(f.hl(), vec![Some(4), Some(0)]);
    f.key("end");
    assert_eq!(f.hl(), vec![Some(4), Some(1)]);
    f.key("left");
    assert_eq!(f.depth(), 1);
    f.key("up");
    f.key("up");
    f.key("up"); // wraps from the first to the last
    assert_eq!(f.hl(), vec![Some(5)]);
    // Type-ahead.
    f.h.type_text("s");
    f.h.run_until_idle(100);
    assert_eq!(f.hl(), vec![Some(0)]);
    f.h.type_text("s");
    f.h.run_until_idle(100);
    assert_eq!(f.hl(), vec![Some(3)], "next item starting with s");
    // Into the submenu and choose.
    f.key("down");
    f.key("enter");
    assert_eq!(f.depth(), 2);
    f.key("down");
    f.key("enter");
    assert_eq!(f.depth(), 0);
    assert_eq!(f.log(), vec!["scale"]);
    assert_eq!(f.h.app.focused(), Some(area), "focus returns");
    // Escape closes one level at a time.
    f.key("shift-f10");
    f.key("up");
    f.key("up");
    f.key("right");
    assert_eq!(f.depth(), 2);
    f.key("escape");
    assert_eq!(f.depth(), 1);
    f.key("escape");
    assert_eq!(f.depth(), 0);
    assert_eq!(f.h.app.focused(), Some(area));
    // Choosing an item with an action dispatches it too.
    f.key("shift-f10");
    f.key("enter");
    assert_eq!(f.log(), vec!["scale", "save", "action"]);
}

#[test]
fn submenu_hover_intent() {
    let mut f = fixture();
    f.right_click(Point::new(50.0, 60.0));
    let t = center(&f.h, "menuitem:Transform");
    f.h.pointer_move(t.x, t.y);
    f.h.step();
    assert_eq!(f.depth(), 1, "not at once");
    wait(&mut f.h, 100);
    assert_eq!(f.depth(), 1, "still waiting");
    wait(&mut f.h, 150);
    f.h.run_until_idle(100);
    assert_eq!(f.depth(), 2);
    // Crossing another item on the way does not close it at once, and
    // reaching the submenu keeps it.
    let p = center(&f.h, "menuitem:Paste");
    f.h.pointer_move(p.x, p.y);
    wait(&mut f.h, 50);
    assert_eq!(f.depth(), 2);
    let r = center(&f.h, "menuitem:Rotate");
    f.h.pointer_move(r.x, r.y);
    wait(&mut f.h, 400);
    f.h.run_until_idle(100);
    assert_eq!(f.depth(), 2);
    assert_eq!(f.hl(), vec![Some(4), Some(0)]);
    // Resting on another item closes the submenu after the delay.
    let p = center(&f.h, "menuitem:Paste");
    f.h.pointer_move(p.x, p.y);
    wait(&mut f.h, 300);
    f.h.run_until_idle(100);
    assert_eq!(f.depth(), 1);
    // Clicking a disabled item does nothing; an enabled one chooses it.
    let d = center(&f.h, "menuitem:Redo");
    f.h.click(d.x, d.y);
    f.h.run_until_idle(100);
    assert_eq!(f.depth(), 1);
    let g = center(&f.h, "menuitem:Show Grid");
    f.h.click(g.x, g.y);
    f.h.run_until_idle(100);
    assert_eq!(f.depth(), 0);
    assert_eq!(f.log(), vec!["grid"]);
}

#[test]
fn menubar_keyboard_and_switching() {
    let mut f = fixture();
    let other = find(&f.h, "other");
    f.h.app.focus(other);
    f.key("f10");
    let bar_el = find(&f.h, "menubar");
    assert_eq!(f.h.app.focused(), Some(bar_el));
    assert_eq!(f.bar.read(&f.h.app).highlighted(), Some(0));
    f.key("right");
    assert_eq!(f.bar.read(&f.h.app).highlighted(), Some(1));
    f.key("down");
    assert_eq!(f.bar.read(&f.h.app).open_menu(), Some(1));
    assert_eq!(f.hl(), vec![Some(0)]);
    let edit = f.h.app.bounds(find(&f.h, "menubar:Edit")).unwrap();
    let r = f.popup().read(&f.h.app).rects()[0];
    assert_eq!((r.x, r.y), (edit.x, edit.bottom()), "below its title");
    f.key("right");
    assert_eq!(f.bar.read(&f.h.app).open_menu(), Some(2));
    f.key("left");
    f.key("left");
    assert_eq!(f.bar.read(&f.h.app).open_menu(), Some(0));
    f.key("left");
    assert_eq!(f.bar.read(&f.h.app).open_menu(), Some(2), "wraps");
    f.key("escape");
    assert_eq!(f.depth(), 0);
    assert_eq!(f.bar.read(&f.h.app).open_menu(), None);
    assert_eq!(f.h.app.focused(), Some(bar_el), "back on the bar");
    f.key("escape");
    assert_eq!(f.h.app.focused(), Some(other), "back where it was");
    // Alt+letter.
    f.key("alt-v");
    assert_eq!(f.bar.read(&f.h.app).open_menu(), Some(2));
    f.key("escape");
    // Mouse: press a title, hover another to switch, press it to close.
    let e = center(&f.h, "menubar:Edit");
    f.h.pointer_move(e.x, e.y);
    f.h.pointer_down(e.x, e.y);
    f.h.pointer_up(e.x, e.y);
    f.h.run_until_idle(100);
    assert_eq!(f.bar.read(&f.h.app).open_menu(), Some(1));
    let v = center(&f.h, "menubar:View");
    f.h.pointer_move(v.x, v.y);
    f.h.run_until_idle(100);
    assert_eq!(f.bar.read(&f.h.app).open_menu(), Some(2));
    f.h.pointer_down(v.x, v.y);
    f.h.pointer_up(v.x, v.y);
    f.h.run_until_idle(100);
    assert_eq!(f.depth(), 0);
    assert_eq!(f.bar.read(&f.h.app).open_menu(), None);
}

#[test]
fn menu_semantics() {
    let mut f = fixture();
    f.right_click(Point::new(50.0, 60.0));
    let s = f.popup().read(&f.h.app).semantics().unwrap().to_string();
    assert!(s.starts_with("Menu"), "{s}");
    assert!(s.contains("MenuItem label=\"Save\""), "{s}");
    assert!(
        s.contains("MenuItem label=\"Redo\" selected=false disabled"),
        "{s}"
    );
    assert!(s.contains("Separator"), "{s}");
    assert!(s.contains("MenuItemCheckBox label=\"Show Grid\""), "{s}");
    assert!(s.contains("toggled=True"), "{s}");
    let item = find(&f.h, "menuitem:Show Grid");
    let node = f.h.app.semantic_node(item).unwrap();
    assert_eq!(node.role(), Some(livspeil_ui_core::Role::MenuItemCheckBox));
    let bar = f.bar.read(&f.h.app).semantics().to_string();
    assert!(bar.starts_with("MenuBar"), "{bar}");
}

#[test]
fn a_registered_bar_that_is_not_shown_ignores_mnemonics() {
    let mut out = None;
    let (mut h, env) = host_sized(Size::new(400.0, 300.0), |app, env| {
        let bar =
            app.new_view(MenuBar::new(env).menu("Edit", Menu::new().item(MenuItem::new("Undo"))));
        env.set_menubar(bar);
        out = Some(bar);
        Box::new(|_| div().name("other").size(100.0, 40.0).focusable())
    });
    h.run_until_idle(100);
    env.focus_root(&mut h.app);
    h.key("alt-e");
    h.run_until_idle(100);
    let bar = out.unwrap();
    assert_eq!(bar.read(&h.app).open_menu(), None);
    assert!(env.menus().is_none_or(|p| !p.read(&h.app).is_open()));
}

#[test]
fn popups_fade_in_without_scaling() {
    let mut f = fixture();
    let p = Point::new(100.0, 100.0);
    pointer(
        &mut f.h,
        PointerKind::Down(MouseButton::Right),
        p,
        Modifiers::NONE,
    );
    pointer(
        &mut f.h,
        PointerKind::Up(MouseButton::Right),
        p,
        Modifiers::NONE,
    );
    // Sample every frame of the open animation: the scale never leaves 1.
    for _ in 0..6 {
        f.h.step();
        if let Some(level) = f.h.app.find("menu-level-0") {
            assert_eq!(f.h.app.scale_of(level), Some(1.0), "menu must not scale");
        }
    }
    assert_eq!(f.depth(), 1);
}

#[test]
fn items_show_their_icon_in_the_mark_column_unless_checkable() {
    let (mut h, env) = host_sized(Size::new(400.0, 300.0), |_, _| {
        Box::new(|_| div().size(100.0, 100.0))
    });
    let menu = Menu::new()
        .item(MenuItem::new("Copy").icon("bx-copy"))
        .item(MenuItem::new("Grid").icon("bx-gear").checked(true))
        .item(MenuItem::new("Plain"));
    open_menu(
        &env,
        &mut h.app,
        menu,
        MenuAnchor::Point(Point::new(20.0, 20.0)),
    );
    h.run_until_idle(200);
    let icons: Vec<IconRun> = h
        .scene()
        .unwrap()
        .commands
        .iter()
        .filter_map(|c| match c {
            PaintCmd::Icon(i) => Some(i.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(icons.len(), 1, "{icons:?}");
    assert_eq!(&*icons[0].name, "bx-copy");
    assert_eq!((icons[0].rect.width, icons[0].rect.height), (14.0, 14.0));
    // It sits in the row, left of where the label starts.
    let row = h.app.bounds(find(&h, "menuitem:Copy")).unwrap();
    assert!(icons[0].rect.x >= row.x && icons[0].rect.right() <= row.x + 40.0);
    assert!((icons[0].rect.center().y - row.center().y).abs() <= 0.5);
}
