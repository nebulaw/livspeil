// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Menus: [`Menu`] (items, separators, submenus), the open-menu view
//! [`MenuPopup`] (Glass levels in the overlay layer, keyboard navigation,
//! submenu hover intent), [`MenuBar`] and [`ContextMenu`].

use std::fmt;
use std::rc::Rc;

use livspeil_ui_core::{
    App, Element, ElementId, EventCx, Key, Keymap, Keystroke, MouseButton, NamedKey, PointerKind,
    Render, Role as CoreRole, SharedString, View, ViewContext, actions as core, div, text,
};
use livspeil_ui_layout::{Point, Rect, Vec2};
use livspeil_ui_text::{LayoutOptions, TextLayout, TextStyle};
use livspeil_ui_theme::{Material, MotionClass, Theme};

use crate::env::{Env, OverlayKey, OverlayKind, PopupBuilder};
use crate::keymap::actions;
use crate::semantics::{Role, SemanticAction, Semantics};
use crate::style::{self, metrics};

/// Delay before hovering an item opens its submenu (or closes another).
pub const SUBMENU_DELAY: f32 = 0.2;
/// Padding inside a menu.
const PAD: f32 = 4.0;
/// Height of a separator row.
const SEPARATOR: f32 = 9.0;
/// Margin kept from the window edges.
const MARGIN: f32 = 4.0;
/// Width of the check-mark and submenu-arrow columns.
const MARK: f32 = 14.0;
/// Gap between label and shortcut.
const SHORTCUT_GAP: f32 = 24.0;

/// Called when an item is chosen (after the menu closed).
pub type MenuCallback = Rc<dyn Fn(&mut App)>;

/// A menu item.
#[derive(Clone)]
pub struct MenuItem {
    label: SharedString,
    shortcut: Option<SharedString>,
    action: Option<SharedString>,
    checked: Option<bool>,
    disabled: bool,
    on_select: Option<MenuCallback>,
}

impl fmt::Debug for MenuItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuItem")
            .field("label", &self.label)
            .field("action", &self.action)
            .field("checked", &self.checked)
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl MenuItem {
    /// An item.
    #[must_use]
    pub fn new(label: impl Into<SharedString>) -> Self {
        MenuItem {
            label: label.into(),
            shortcut: None,
            action: None,
            checked: None,
            disabled: false,
            on_select: None,
        }
    }

    /// Dispatch this keymap action when chosen; its global binding is
    /// shown as the shortcut (unless [`MenuItem::shortcut`] is set).
    #[must_use]
    pub fn action(mut self, action: impl Into<SharedString>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Shortcut text shown on the right.
    #[must_use]
    pub fn shortcut(mut self, s: impl Into<SharedString>) -> Self {
        self.shortcut = Some(s.into());
        self
    }

    /// A check-mark item in state `on`.
    #[must_use]
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }

    /// Disable.
    #[must_use]
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }

    /// Run `f` when chosen.
    #[must_use]
    pub fn on_select(mut self, f: impl Fn(&mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// The label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// One row of a menu.
#[derive(Clone, Debug)]
pub enum MenuEntry {
    /// An item.
    Item(MenuItem),
    /// A separator line.
    Separator,
    /// An item opening a submenu.
    Submenu {
        /// Label.
        label: SharedString,
        /// The submenu.
        menu: Rc<Menu>,
        /// Disabled.
        disabled: bool,
    },
}

impl MenuEntry {
    fn label(&self) -> &str {
        match self {
            MenuEntry::Item(i) => &i.label,
            MenuEntry::Submenu { label, .. } => label,
            MenuEntry::Separator => "",
        }
    }

    fn enabled(&self) -> bool {
        match self {
            MenuEntry::Item(i) => !i.disabled,
            MenuEntry::Submenu { disabled, .. } => !disabled,
            MenuEntry::Separator => false,
        }
    }
}

/// A menu: items, separators and submenus.
#[derive(Clone, Debug, Default)]
pub struct Menu {
    /// The rows.
    pub entries: Vec<MenuEntry>,
}

impl Menu {
    /// An empty menu.
    #[must_use]
    pub fn new() -> Self {
        Menu::default()
    }

    /// Add an item.
    #[must_use]
    pub fn item(mut self, item: MenuItem) -> Self {
        self.entries.push(MenuEntry::Item(item));
        self
    }

    /// Add a separator.
    #[must_use]
    pub fn separator(mut self) -> Self {
        self.entries.push(MenuEntry::Separator);
        self
    }

    /// Add a submenu.
    #[must_use]
    pub fn submenu(mut self, label: impl Into<SharedString>, menu: Menu) -> Self {
        self.entries.push(MenuEntry::Submenu {
            label: label.into(),
            menu: Rc::new(menu),
            disabled: false,
        });
        self
    }

    fn enabled(&self) -> Vec<bool> {
        self.entries.iter().map(MenuEntry::enabled).collect()
    }
}

/// Shortcut text of a keystroke (`Ctrl+Shift+Z`, `F10`, `Del`).
#[must_use]
pub fn format_keystroke(k: &Keystroke) -> String {
    let mut s = String::new();
    for (on, name) in [
        (k.modifiers.ctrl, "Ctrl+"),
        (k.modifiers.alt, "Alt+"),
        (k.modifiers.shift, "Shift+"),
        (k.modifiers.cmd, "Cmd+"),
    ] {
        if on {
            s.push_str(name);
        }
    }
    match k.key {
        Key::Char(c) => s.extend(c.to_uppercase()),
        Key::Named(n) => s.push_str(&match n {
            NamedKey::Enter => "Enter".to_owned(),
            NamedKey::Tab => "Tab".to_owned(),
            NamedKey::Space => "Space".to_owned(),
            NamedKey::Escape => "Esc".to_owned(),
            NamedKey::Backspace => "Backspace".to_owned(),
            NamedKey::Delete => "Del".to_owned(),
            NamedKey::Left => "Left".to_owned(),
            NamedKey::Right => "Right".to_owned(),
            NamedKey::Up => "Up".to_owned(),
            NamedKey::Down => "Down".to_owned(),
            NamedKey::Home => "Home".to_owned(),
            NamedKey::End => "End".to_owned(),
            NamedKey::PageUp => "PgUp".to_owned(),
            NamedKey::PageDown => "PgDn".to_owned(),
            NamedKey::F(n) => format!("F{n}"),
        }),
    }
    s
}

/// The shortcut text of `action`: its last global binding in `keymap`.
#[must_use]
pub fn shortcut_for(keymap: &Keymap, action: &str) -> Option<String> {
    keymap
        .bindings()
        .iter()
        .rev()
        .find(|b| b.context.is_none() && &*b.action == action)
        .map(|b| format_keystroke(&b.keystroke))
}

/// Where a menu of `size` opens at the pointer `at`: to the right and
/// below, flipped left / up when it would leave the viewport, then clamped
/// into it.
#[must_use]
pub fn place_menu(at: Point, size: Vec2, viewport: Vec2) -> Rect {
    let mut x = at.x;
    if x + size.x > viewport.x - MARGIN && at.x - size.x >= MARGIN {
        x = at.x - size.x;
    }
    let mut y = at.y;
    if y + size.y > viewport.y - MARGIN && at.y - size.y >= MARGIN {
        y = at.y - size.y;
    }
    clamp_rect(Rect::new(x, y, size.x, size.y), viewport)
}

/// Where a menu opens below an anchor (a menu bar title or button): left
/// aligned below it, above if there is no room, clamped.
#[must_use]
pub fn place_below(anchor: Rect, size: Vec2, viewport: Vec2) -> Rect {
    let mut y = anchor.bottom();
    if y + size.y > viewport.y - MARGIN && anchor.y - size.y >= MARGIN {
        y = anchor.y - size.y;
    }
    clamp_rect(Rect::new(anchor.x, y, size.x, size.y), viewport)
}

/// Where a submenu opens next to its item: to the right (overlapping the
/// parent's padding), else to the left; top item aligned with the parent
/// item; clamped.
#[must_use]
pub fn place_submenu(item: Rect, size: Vec2, viewport: Vec2) -> Rect {
    let mut x = item.right() + PAD;
    if x + size.x > viewport.x - MARGIN && item.x - PAD - size.x >= MARGIN {
        x = item.x - PAD - size.x;
    }
    clamp_rect(Rect::new(x, item.y - PAD, size.x, size.y), viewport)
}

fn clamp_rect(r: Rect, viewport: Vec2) -> Rect {
    let x = r.x.min(viewport.x - MARGIN - r.width).max(MARGIN);
    let y = r.y.min(viewport.y - MARGIN - r.height).max(MARGIN);
    Rect::new(x, y, r.width, r.height)
}

fn item_height(theme: &Theme) -> f32 {
    (metrics(theme).height - 4.0).max(22.0)
}

fn row_height(e: &MenuEntry, theme: &Theme) -> f32 {
    match e {
        MenuEntry::Separator => SEPARATOR,
        _ => item_height(theme),
    }
}

/// Size of a menu (text measured with `env`'s fonts).
fn menu_size(env: &Env, app: &App, menu: &Menu) -> Vec2 {
    let theme = app.theme();
    let m = metrics(theme);
    let mut fonts = env.fonts();
    let style = TextStyle::sized(m.font);
    let measure = |s: &str, fonts: &mut livspeil_ui_text::FontSystem| {
        TextLayout::new(fonts, s, &style, LayoutOptions::default())
            .width()
            .ceil()
    };
    let mut label_w = 0.0_f32;
    let mut short_w = 0.0_f32;
    let mut h = 2.0 * PAD;
    for e in &menu.entries {
        h += row_height(e, theme);
        label_w = label_w.max(measure(e.label(), &mut fonts));
        if let MenuEntry::Item(i) = e
            && let Some(s) = shortcut_text(app, i)
        {
            short_w = short_w.max(measure(&s, &mut fonts));
        }
    }
    let shortcut = if short_w > 0.0 {
        SHORTCUT_GAP + short_w
    } else {
        0.0
    };
    let w = 2.0 * PAD + 2.0 * (m.pad_x - PAD).max(4.0) + 2.0 * (MARK + 4.0) + label_w + shortcut;
    Vec2::new(w.max(140.0).ceil(), h)
}

fn shortcut_text(app: &App, i: &MenuItem) -> Option<String> {
    i.shortcut.as_deref().map(str::to_owned).or_else(|| {
        i.action
            .as_deref()
            .and_then(|a| shortcut_for(app.keymap(), a))
    })
}

/// Where a menu is opened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuAnchor {
    /// At a point (context menus).
    Point(Point),
    /// Below a rect (menu bar titles, buttons).
    Below(Rect),
}

#[derive(Clone, Debug)]
struct Level {
    menu: Rc<Menu>,
    rect: Rect,
    highlighted: Option<usize>,
    /// The item of the parent level that opened it.
    parent: Option<usize>,
    appearing: bool,
}

impl Level {
    fn item_rect(&self, i: usize, theme: &Theme) -> Rect {
        let mut y = self.rect.y + PAD;
        for e in self.menu.entries.iter().take(i) {
            y += row_height(e, theme);
        }
        let h = self
            .menu
            .entries
            .get(i)
            .map_or(0.0, |e| row_height(e, theme));
        Rect::new(self.rect.x + PAD, y, self.rect.width - 2.0 * PAD, h)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Intent {
    level: usize,
    index: usize,
    elapsed: f32,
}

/// The open menus (one menu system is open at a time; [`Env`] owns the
/// popup). A full-window element in the overlay layer: Glass levels for
/// the menu and its open submenus; pressing outside closes.
///
/// Keyboard (context `Menu`): Up/Down move over enabled items (wrapping),
/// Home/End, Right opens a submenu (or the next menu bar menu), Left
/// closes one (or the previous menu bar menu), Enter/Space choose, Escape
/// closes one level, Tab closes all, typing a letter highlights the next
/// item starting with it.
pub struct MenuPopup {
    env: Env,
    levels: Vec<Level>,
    intent: Option<Intent>,
    restore: Option<ElementId>,
    bar: Option<(View<MenuBar>, usize)>,
}

impl fmt::Debug for MenuPopup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuPopup")
            .field("levels", &self.levels.len())
            .field("intent", &self.intent)
            .finish_non_exhaustive()
    }
}

/// Key of the popup in the overlay layer.
const POPUP_KEY: &str = "menu-popup";

impl MenuPopup {
    /// Whether a menu is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.levels.is_empty()
    }

    /// Number of open levels (menu plus submenus).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// The highlighted item of each open level.
    #[must_use]
    pub fn highlighted(&self) -> Vec<Option<usize>> {
        self.levels.iter().map(|l| l.highlighted).collect()
    }

    /// The rect of each open level, in window coordinates.
    #[must_use]
    pub fn rects(&self) -> Vec<Rect> {
        self.levels.iter().map(|l| l.rect).collect()
    }

    /// Semantics of the deepest level: `Menu` with item parts.
    #[must_use]
    pub fn semantics(&self) -> Option<Semantics> {
        let l = self.levels.last()?;
        Some(level_semantics(l))
    }
}

fn level_semantics(l: &Level) -> Semantics {
    let mut s = Semantics::new(Role::Menu);
    for (i, e) in l.menu.entries.iter().enumerate() {
        let part = match e {
            MenuEntry::Separator => Semantics::new(Role::Separator),
            MenuEntry::Item(it) => {
                let mut p = Semantics::new(if it.checked.is_some() {
                    Role::MenuItemCheckBox
                } else {
                    Role::MenuItem
                })
                .label(&*it.label)
                .interactive(it.disabled, &[SemanticAction::Click])
                .selected(l.highlighted == Some(i));
                p.focusable = false;
                if let Some(c) = it.checked {
                    p = p.toggled(if c {
                        crate::semantics::Toggled::True
                    } else {
                        crate::semantics::Toggled::False
                    });
                }
                p
            }
            MenuEntry::Submenu {
                label, disabled, ..
            } => {
                let mut p = Semantics::new(Role::MenuItem)
                    .label(&**label)
                    .expanded(false)
                    .interactive(*disabled, &[SemanticAction::Expand]);
                p.focusable = false;
                p
            }
        };
        s = s.child(part);
    }
    s
}

/// The popup of `env` (created on first use).
fn popup_view(env: &Env, app: &mut App) -> View<MenuPopup> {
    if let Some(v) = env.menu_popup() {
        return v;
    }
    let v = app.new_view(MenuPopup {
        env: env.clone(),
        levels: Vec::new(),
        intent: None,
        restore: None,
        bar: None,
    });
    env.set_menu_popup(v);
    v
}

/// Open `menu` at `anchor`; it takes keyboard focus (returned on close).
/// Replaces any open menu.
pub fn open_menu(env: &Env, app: &mut App, menu: Menu, anchor: MenuAnchor) -> View<MenuPopup> {
    open_with(env, app, Rc::new(menu), anchor, None, None)
}

fn open_with(
    env: &Env,
    app: &mut App,
    menu: Rc<Menu>,
    anchor: MenuAnchor,
    bar: Option<(View<MenuBar>, usize)>,
    highlight_first: Option<bool>,
) -> View<MenuPopup> {
    let view = popup_view(env, app);
    let vp = app.viewport();
    let size = menu_size(env, app, &menu);
    let vpv = Vec2::new(vp.width, vp.height);
    let rect = match anchor {
        MenuAnchor::Point(p) => place_menu(p, size, vpv),
        MenuAnchor::Below(r) => place_below(r, size, vpv),
    };
    let was_open = view.read(app).is_open();
    let focused = app.focused();
    let restore_prev = view.read(app).restore;
    let highlighted = if highlight_first == Some(true) {
        style::edge_enabled(&menu.enabled(), 1)
    } else {
        None
    };
    view.update(app, |p, cx| {
        p.levels = vec![Level {
            menu,
            rect,
            highlighted,
            parent: None,
            appearing: true,
        }];
        p.intent = None;
        p.bar = bar;
        // Keep the original focus when switching between open menus.
        p.restore = if was_open { restore_prev } else { focused };
        cx.notify();
    });
    let build: PopupBuilder = Rc::new(move |_, _| vec![Element::view(&view).key(POPUP_KEY)]);
    env.show_overlay(app, OverlayKey::Popup(view.id()), OverlayKind::Popup(build));
    if let Some(id) = env.overlay_child(app, POPUP_KEY) {
        env.focus(app, id);
    }
    view
}

/// Close every level; with `refocus`, focus returns to where it was.
pub fn close_menu(env: &Env, app: &mut App, refocus: bool) {
    let Some(view) = env.menu_popup() else {
        return;
    };
    if !view.read(app).is_open() {
        return;
    }
    let (restore, bar) = view.update(app, |p, cx| {
        p.levels.clear();
        p.intent = None;
        cx.notify();
        (p.restore.take(), p.bar.take())
    });
    env.hide_overlay(app, OverlayKey::Popup(view.id()));
    if let Some((bar, _)) = bar {
        bar.update(app, |b, cx| {
            b.open = None;
            cx.notify();
        });
    }
    if refocus {
        match restore {
            Some(r) => env.focus(app, r),
            None => {
                app.blur();
                env.sync_focus(app);
            }
        }
    }
}

fn choose(view: View<MenuPopup>, app: &mut App, level: usize, index: usize) {
    let (env, entry, item_rect) = {
        let p = view.read(app);
        let Some(l) = p.levels.get(level) else {
            return;
        };
        let Some(e) = l.menu.entries.get(index) else {
            return;
        };
        (p.env.clone(), e.clone(), l.item_rect(index, app.theme()))
    };
    match entry {
        MenuEntry::Separator => {}
        MenuEntry::Submenu { disabled: true, .. } => {}
        MenuEntry::Item(MenuItem { disabled: true, .. }) => {}
        MenuEntry::Submenu { menu, .. } => {
            open_submenu(view, app, level, index, item_rect, menu, true);
        }
        MenuEntry::Item(item) => {
            close_menu(&env, app, true);
            if let Some(f) = &item.on_select {
                f(app);
            }
            if let Some(a) = &item.action {
                app.dispatch_action(a);
            }
        }
    }
}

fn open_submenu(
    view: View<MenuPopup>,
    app: &mut App,
    level: usize,
    index: usize,
    item: Rect,
    menu: Rc<Menu>,
    keyboard: bool,
) {
    let env = view.read(app).env.clone();
    let size = menu_size(&env, app, &menu);
    let vp = app.viewport();
    let rect = place_submenu(item, size, Vec2::new(vp.width, vp.height));
    let highlighted = if keyboard {
        style::edge_enabled(&menu.enabled(), 1)
    } else {
        None
    };
    view.update(app, |p, cx| {
        p.levels.truncate(level + 1);
        if let Some(l) = p.levels.get_mut(level) {
            l.highlighted = Some(index);
        }
        p.levels.push(Level {
            menu,
            rect,
            highlighted,
            parent: Some(index),
            appearing: true,
        });
        p.intent = None;
        cx.notify();
    });
}

/// The pointer is over item `index` of `level`.
fn hover(view: View<MenuPopup>, app: &mut App, level: usize, index: usize) {
    view.update(app, |p, cx| {
        // Moving into a submenu keeps its parent item highlighted.
        if level > 0
            && let Some(parent) = p.levels.get(level).and_then(|l| l.parent)
            && let Some(up) = p.levels.get_mut(level - 1)
        {
            up.highlighted = Some(parent);
        }
        let Some(l) = p.levels.get_mut(level) else {
            return;
        };
        let enabled = l.menu.entries.get(index).is_some_and(MenuEntry::enabled);
        let new = enabled.then_some(index);
        let changed = l.highlighted != new;
        l.highlighted = new;
        let is_sub = matches!(l.menu.entries.get(index), Some(MenuEntry::Submenu { .. }));
        let open_child = p.levels.get(level + 1).and_then(|c| c.parent);
        let intent_here = p
            .intent
            .is_some_and(|i| i.level == level && i.index == index);
        if open_child == Some(index) {
            p.intent = None;
        } else if (is_sub && enabled || open_child.is_some()) && !intent_here {
            // Open (or switch away from) a submenu after the hover delay.
            p.intent = Some(Intent {
                level,
                index,
                elapsed: 0.0,
            });
        } else if !is_sub && open_child.is_none() {
            p.intent = None;
        }
        if changed || p.intent.is_some() {
            cx.notify();
        }
    });
}

fn step(
    view: View<MenuPopup>,
    app: &mut App,
    f: impl FnOnce(&[bool], Option<usize>) -> Option<usize>,
) {
    view.update(app, |p, cx| {
        p.intent = None;
        if let Some(l) = p.levels.last_mut() {
            let enabled = l.menu.enabled();
            if let Some(i) = f(&enabled, l.highlighted) {
                l.highlighted = Some(i);
                cx.notify();
            }
        }
    });
}

fn key_right(view: View<MenuPopup>, app: &mut App) {
    let (level, hl, entry, bar) = {
        let p = view.read(app);
        let Some(l) = p.levels.last() else { return };
        let hl = l.highlighted;
        (
            p.levels.len() - 1,
            hl,
            hl.and_then(|i| l.menu.entries.get(i).cloned()),
            p.bar,
        )
    };
    if let (
        Some(i),
        Some(MenuEntry::Submenu {
            disabled: false, ..
        }),
    ) = (hl, &entry)
    {
        choose(view, app, level, i);
    } else if let Some((bar, at)) = bar {
        MenuBar::open_index(bar, app, at + 1, true);
    }
}

fn key_left(view: View<MenuPopup>, app: &mut App) {
    let (depth, bar) = {
        let p = view.read(app);
        (p.levels.len(), p.bar)
    };
    if depth > 1 {
        view.update(app, |p, cx| {
            p.levels.pop();
            p.intent = None;
            cx.notify();
        });
    } else if let Some((bar, at)) = bar {
        let n = bar.read(app).menus.len();
        MenuBar::open_index(bar, app, (at + n - 1) % n.max(1), true);
    }
}

fn key_escape(view: View<MenuPopup>, app: &mut App) {
    let (depth, env) = {
        let p = view.read(app);
        (p.levels.len(), p.env.clone())
    };
    if depth > 1 {
        view.update(app, |p, cx| {
            p.levels.pop();
            cx.notify();
        });
    } else {
        close_menu(&env, app, true);
    }
}

fn type_ahead(view: View<MenuPopup>, app: &mut App, t: &str) {
    let Some(c) = t.chars().next() else { return };
    step(view, app, |_, _| None); // clears a pending intent
    view.update(app, |p, cx| {
        let Some(l) = p.levels.last_mut() else { return };
        let n = l.menu.entries.len();
        let start = l.highlighted.map_or(0, |h| h + 1);
        let hit = (0..n).map(|k| (start + k) % n.max(1)).find(|&i| {
            let e = &l.menu.entries[i];
            e.enabled()
                && e.label()
                    .chars()
                    .next()
                    .is_some_and(|f| f.to_lowercase().eq(c.to_lowercase()))
        });
        if let Some(i) = hit {
            l.highlighted = Some(i);
            cx.notify();
        }
    });
}

impl Render for MenuPopup {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let m = metrics(&theme);
        let c = theme.colors();
        let vp = cx.viewport();

        // Hover intent: open or switch submenus after the delay.
        if let Some(mut it) = self.intent {
            it.elapsed += cx.frame_delta();
            if it.elapsed >= SUBMENU_DELAY {
                self.intent = None;
                let level = self.levels.get(it.level).cloned();
                if let Some(l) = level.filter(|l| l.highlighted == Some(it.index)) {
                    self.levels.truncate(it.level + 1);
                    if let Some(MenuEntry::Submenu {
                        menu,
                        disabled: false,
                        ..
                    }) = l.menu.entries.get(it.index)
                    {
                        let size = menu_size(&self.env, cx, menu);
                        let rect = place_submenu(
                            l.item_rect(it.index, &theme),
                            size,
                            Vec2::new(vp.width, vp.height),
                        );
                        self.levels.push(Level {
                            menu: menu.clone(),
                            rect,
                            highlighted: None,
                            parent: Some(it.index),
                            appearing: true,
                        });
                    }
                }
                cx.notify();
            } else {
                self.intent = Some(it);
                cx.notify();
            }
        }

        let item_h = item_height(&theme);
        let mut root = div()
            .name("menu-popup")
            .absolute(Rect::new(0.0, 0.0, vp.width, vp.height))
            .tab_index(-1)
            .focus_scope()
            .key_context("Menu");
        for (li, l) in self.levels.iter_mut().enumerate() {
            let sem = level_semantics(l);
            let appearing = std::mem::replace(&mut l.appearing, false);
            if appearing {
                cx.notify();
            }
            let mut level = div()
                .key(format!("level-{li}"))
                .name(format!("menu-level-{li}"))
                .absolute(l.rect)
                .flex_col()
                .p(PAD)
                .rounded(theme.tokens.radii.sm + 2.0)
                .material(Material::Glass)
                .opacity(if appearing { 0.0 } else { 1.0 })
                .scale(if appearing { 0.98 } else { 1.0 })
                .transition(MotionClass::Standard)
                .on_pointer_down(|_, cx| cx.stop_propagation());
            for (i, e) in l.menu.entries.iter().enumerate() {
                let row = match e {
                    MenuEntry::Separator => div()
                        .key(format!("item-{i}"))
                        .h(SEPARATOR)
                        .flex_col()
                        .justify_center()
                        .child(div().h(1.0).bg(style::hairline(&theme).with_alpha(0.12)))
                        .semantics(|s| s.role(CoreRole::Separator)),
                    MenuEntry::Item(_) | MenuEntry::Submenu { .. } => {
                        let hl = l.highlighted == Some(i);
                        let enabled = e.enabled();
                        let fg = if !enabled {
                            c.text_tertiary
                        } else if hl {
                            style::on_color(theme.accent())
                        } else {
                            c.text_primary
                        };
                        let (check, shortcut, arrow) = match e {
                            MenuEntry::Item(it) => {
                                (it.checked == Some(true), shortcut_text(cx, it), false)
                            }
                            _ => (false, None, true),
                        };
                        let mut row = div()
                            .key(format!("item-{i}"))
                            .name(format!("menuitem:{}", e.label()))
                            .h(item_h)
                            .px((m.pad_x - PAD).max(4.0))
                            .flex_row()
                            .items_center()
                            .gap(4.0)
                            .rounded(theme.tokens.radii.xs)
                            .bg(if hl {
                                theme.accent()
                            } else {
                                theme.accent().with_alpha(0.0)
                            })
                            .child(
                                div().key("mark").w(MARK).hit_test(false).child(
                                    text(if check { "✓" } else { "" })
                                        .font_size(m.font)
                                        .text_color(fg),
                                ),
                            )
                            .child(
                                text(SharedString::from(e.label()))
                                    .key("label")
                                    .font_size(m.font)
                                    .text_color(fg)
                                    .hit_test(false)
                                    .grow(),
                            );
                        if let Some(s) = shortcut {
                            row = row.child(
                                text(s)
                                    .key("shortcut")
                                    .font_size(m.font)
                                    .text_color(if hl && enabled {
                                        fg.with_alpha(0.75)
                                    } else {
                                        c.text_tertiary
                                    })
                                    .hit_test(false),
                            );
                        }
                        row = row.child(
                            div().key("arrow").w(MARK).hit_test(false).child(
                                text(if arrow { "›" } else { "" })
                                    .font_size(m.font)
                                    .text_color(fg),
                            ),
                        );
                        let li2 = li;
                        row = row.on_pointer_move(move |_, cx| hover(view, cx, li2, i));
                        if enabled {
                            row = row.on_click(move |_, cx| choose(view, cx, li2, i));
                        }
                        row
                    }
                };
                let row = match sem.children.get(i) {
                    Some(part) => part.apply(row),
                    None => row,
                };
                level = level.child(row);
            }
            level = level.semantics(|s| s.role(CoreRole::Menu));
            root = root.child(level);
        }
        let env = self.env.clone();
        let env2 = env.clone();
        root.on_pointer_down(move |ev, cx| {
            // A press outside every level.
            let (bar, pos) = (view.read(cx).bar, ev.position);
            if let Some((bar, at)) = bar
                && let Some(i) = bar.read(cx).title_at(cx, pos)
            {
                if i == at {
                    close_menu(&env, cx, true);
                } else {
                    MenuBar::open_index(bar, cx, i, false);
                }
            } else {
                close_menu(
                    &env,
                    cx,
                    !matches!(ev.kind, PointerKind::Down(MouseButton::Left)),
                );
            }
            cx.stop_propagation();
        })
        .on_pointer_move(move |ev, cx| {
            if cx.target() != cx.element_id() {
                return;
            }
            let (bar, pos) = (view.read(cx).bar, ev.position);
            if let Some((bar, at)) = bar
                && let Some(i) = bar.read(cx).title_at(cx, pos)
                && i != at
            {
                MenuBar::open_index(bar, cx, i, false);
                return;
            }
            // Off the menus: clear the deepest highlight unless it opened
            // a submenu.
            view.update(cx, |p, vcx| {
                let depth = p.levels.len();
                if let Some(l) = p.levels.last_mut()
                    && l.highlighted.is_some()
                    && depth > 0
                {
                    l.highlighted = None;
                    p.intent = None;
                    vcx.notify();
                }
            });
        })
        .on_action(actions::NEXT, move |cx| {
            step(view, cx, |e, h| style::step_enabled(e, h, 1, true));
        })
        .on_action(actions::PREV, move |cx| {
            step(view, cx, |e, h| style::step_enabled(e, h, -1, true));
        })
        .on_action(actions::FIRST, move |cx| {
            step(view, cx, |e, _| style::edge_enabled(e, 1));
        })
        .on_action(actions::LAST, move |cx| {
            step(view, cx, |e, _| style::edge_enabled(e, -1));
        })
        .on_action(actions::MENU_RIGHT, move |cx| key_right(view, cx))
        .on_action(actions::MENU_LEFT, move |cx| key_left(view, cx))
        .on_action(core::ACTIVATE, move |cx| {
            let (level, hl) = {
                let p = view.read(cx);
                (
                    p.levels.len().saturating_sub(1),
                    p.levels.last().and_then(|l| l.highlighted),
                )
            };
            if let Some(i) = hl {
                choose(view, cx, level, i);
            }
        })
        .on_action(core::CANCEL, move |cx| key_escape(view, cx))
        .on_action(core::FOCUS_NEXT, move |cx| close_menu(&env2, cx, true))
        .on_action(core::FOCUS_PREV, move |cx| {
            let env = view.read(cx).env.clone();
            close_menu(&env, cx, true);
        })
        .on_text_input(move |t, cx| type_ahead(view, cx, t))
    }
}

/// A horizontal menu bar (Frost) with one title per menu.
///
/// - Press a title to open its menu; while one is open, moving over
///   another title switches to it; pressing the open title closes it.
/// - F10 (`menu.focus_bar`, via [`crate::WidgetRoot`]) focuses the bar:
///   Left/Right move between titles, Down/Enter/Space open the menu with
///   its first item highlighted, Escape returns focus. Alt+letter opens
///   the menu whose title starts with the letter.
/// - In an open menu, Left/Right at the top level switch to the
///   neighbouring menu.
pub struct MenuBar {
    env: Env,
    menus: Vec<(SharedString, Rc<Menu>)>,
    open: Option<usize>,
    highlighted: Option<usize>,
    name: Option<SharedString>,
    restore: Option<ElementId>,
}

impl fmt::Debug for MenuBar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuBar")
            .field("open", &self.open)
            .field("highlighted", &self.highlighted)
            .finish_non_exhaustive()
    }
}

impl MenuBar {
    /// A menu bar; register it with [`Env::set_menubar`] for F10 and
    /// Alt+letter.
    #[must_use]
    pub fn new(env: &Env) -> Self {
        MenuBar {
            env: env.clone(),
            menus: Vec::new(),
            open: None,
            highlighted: None,
            name: None,
            restore: None,
        }
    }

    /// Add a menu.
    #[must_use]
    pub fn menu(mut self, title: impl Into<SharedString>, menu: Menu) -> Self {
        self.menus.push((title.into(), Rc::new(menu)));
        self
    }

    /// Element name (default `menubar`).
    #[must_use]
    pub fn name(mut self, n: impl Into<SharedString>) -> Self {
        self.name = Some(n.into());
        self
    }

    /// Replace menu `index` (for example to update check marks after a
    /// choice); an open popup keeps showing the old menu until reopened.
    /// Returns whether `index` exists.
    pub fn replace_menu(&mut self, index: usize, menu: Menu) -> bool {
        match self.menus.get_mut(index) {
            Some(slot) => {
                slot.1 = Rc::new(menu);
                true
            }
            None => false,
        }
    }

    /// The open menu's index.
    #[must_use]
    pub fn open_menu(&self) -> Option<usize> {
        self.open
    }

    /// The keyboard-highlighted title.
    #[must_use]
    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    fn element_name(&self) -> &str {
        self.name.as_deref().unwrap_or("menubar")
    }

    fn title_rect(&self, app: &App, i: usize) -> Option<Rect> {
        let bar = app.find(self.element_name())?;
        app.bounds(bar.child(&format!("title-{i}")))
    }

    fn title_at(&self, app: &App, p: Point) -> Option<usize> {
        (0..self.menus.len()).find(|&i| self.title_rect(app, i).is_some_and(|r| r.contains(p)))
    }

    /// Open menu `i` (wrapping); `keyboard` highlights its first item.
    pub fn open_index(view: View<MenuBar>, app: &mut App, i: usize, keyboard: bool) {
        let (env, menu, rect) = {
            let b = view.read(app);
            let n = b.menus.len();
            if n == 0 {
                return;
            }
            let i = i % n;
            (
                b.env.clone(),
                b.menus[i].1.clone(),
                b.title_rect(app, i).unwrap_or(Rect::ZERO),
            )
        };
        let i = i % view.read(app).menus.len();
        view.update(app, |b, cx| {
            b.open = Some(i);
            b.highlighted = Some(i);
            cx.notify();
        });
        open_with(
            &env,
            app,
            menu,
            MenuAnchor::Below(rect),
            Some((view, i)),
            Some(keyboard),
        );
    }

    /// Focus the bar for keyboard navigation (F10).
    pub fn focus(view: View<MenuBar>, app: &mut App) {
        let (env, name) = {
            let b = view.read(app);
            (b.env.clone(), b.element_name().to_owned())
        };
        close_menu(&env, app, false);
        let prev = app.focused();
        if let Some(id) = app.find(&name) {
            view.update(app, |b, cx| {
                if prev != Some(id) {
                    b.restore = prev;
                }
                b.highlighted = Some(0);
                cx.notify();
            });
            env.focus(app, id);
        }
    }

    /// Open the menu whose title starts with `c` (Alt+letter); returns
    /// whether one matched. A bar that is not in the tree (registered but
    /// not shown) opens nothing.
    pub fn open_mnemonic(view: View<MenuBar>, app: &mut App, c: char) -> bool {
        if app.find(view.read(app).element_name()).is_none() {
            return false;
        }
        let hit = view.read(app).menus.iter().position(|(t, _)| {
            t.chars()
                .next()
                .is_some_and(|f| f.to_lowercase().eq(c.to_lowercase()))
        });
        if let Some(i) = hit {
            MenuBar::open_index(view, app, i, true);
        }
        hit.is_some()
    }

    /// Semantics: `MenuBar` with a `MenuItem` per title.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        let mut s = Semantics::new(Role::MenuBar);
        for (i, (t, _)) in self.menus.iter().enumerate() {
            let mut p = Semantics::new(Role::MenuItem)
                .label(&**t)
                .expanded(self.open == Some(i))
                .interactive(false, &[SemanticAction::Expand]);
            p.focusable = false;
            s = s.child(p);
        }
        s
    }
}

impl Render for MenuBar {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let m = metrics(&theme);
        let c = theme.colors();
        let focused = self.env.is_focused(cx, self.element_name());
        let sem = self.semantics();
        let mut bar = div()
            .name(self.element_name().to_owned())
            .w_full()
            .h(m.height)
            .px(4.0)
            .flex_row()
            .items_center()
            .gap(2.0)
            .material(Material::Frost)
            .tab_index(-1)
            .key_context("MenuBar");
        for (i, (title, _)) in self.menus.iter().enumerate() {
            let on = self.open == Some(i) || (focused && self.highlighted == Some(i));
            let el = div()
                .key(format!("title-{i}"))
                .name(format!("menubar:{title}"))
                .h(m.height - 6.0)
                .px(m.pad_x - 2.0)
                .items_center()
                .rounded(theme.tokens.radii.xs)
                .bg(if on {
                    theme.accent().with_alpha(0.3)
                } else {
                    theme.accent().with_alpha(0.0)
                })
                .hover(|r| r.bg(c.surface[3].with_alpha(0.6)))
                .transition(MotionClass::Micro)
                .child(
                    text(title.clone())
                        .font_size(m.font)
                        .font_weight(if i == 0 { 600.0 } else { 400.0 })
                        .text_color(c.text_primary)
                        .hit_test(false),
                )
                .on_pointer_down(move |ev, cx| {
                    if ev.kind != PointerKind::Down(MouseButton::Left) {
                        return;
                    }
                    let open = view.read(cx).open;
                    if open == Some(i) {
                        let env = view.read(cx).env.clone();
                        close_menu(&env, cx, true);
                    } else {
                        MenuBar::open_index(view, cx, i, false);
                    }
                    cx.stop_propagation();
                });
            let el = match sem.children.get(i) {
                Some(p) => p.apply(el),
                None => el,
            };
            bar = bar.child(el);
        }
        let bar = sem.apply(bar);
        let nav = move |dir: isize| {
            move |cx: &mut EventCx<'_>| {
                view.update(cx, |b, vcx| {
                    let n = b.menus.len();
                    if n == 0 {
                        return;
                    }
                    let cur = b.highlighted.unwrap_or(0);
                    #[allow(clippy::cast_possible_wrap)]
                    let next = (cur.cast_signed() + dir).rem_euclid(n.cast_signed());
                    b.highlighted = Some(next.cast_unsigned());
                    vcx.notify();
                });
            }
        };
        let open = move |cx: &mut EventCx<'_>| {
            let i = view.read(cx).highlighted.unwrap_or(0);
            MenuBar::open_index(view, cx, i, true);
        };
        bar.on_action(actions::NEXT, nav(1))
            .on_action(actions::PREV, nav(-1))
            .on_action(actions::OPEN, open)
            .on_action(core::ACTIVATE, open)
            .on_action(core::CANCEL, move |cx| {
                let (env, restore) = view.update(cx, |b, vcx| {
                    b.highlighted = None;
                    vcx.notify();
                    (b.env.clone(), b.restore.take())
                });
                match restore {
                    Some(r) => env.focus(cx, r),
                    None => {
                        cx.blur();
                        env.sync_focus(cx);
                    }
                }
            })
    }
}

/// Context menus: secondary click (or Shift+F10 / the `menu.context`
/// action while focused) opens a menu at the pointer (or the element's
/// center), kept on screen.
#[derive(Clone, Debug)]
pub struct ContextMenu {
    menu: Rc<Menu>,
}

impl ContextMenu {
    /// A context menu.
    #[must_use]
    pub fn new(menu: Menu) -> Self {
        ContextMenu {
            menu: Rc::new(menu),
        }
    }

    /// Attach to `el`.
    #[must_use]
    pub fn attach(self, env: &Env, el: Element) -> Element {
        let (env1, env2) = (env.clone(), env.clone());
        let (m1, m2) = (self.menu.clone(), self.menu);
        el.on_pointer_down(move |ev, cx| {
            if ev.kind == PointerKind::Down(MouseButton::Right) {
                open_with(
                    &env1,
                    cx,
                    m1.clone(),
                    MenuAnchor::Point(ev.position),
                    None,
                    None,
                );
                cx.stop_propagation();
            }
        })
        .on_action(actions::CONTEXT_MENU, move |cx| {
            let at = cx.bounds().center();
            open_with(
                &env2,
                cx,
                m2.clone(),
                MenuAnchor::Point(at),
                None,
                Some(true),
            );
        })
    }
}
