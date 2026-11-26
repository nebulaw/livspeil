// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The Livspeil UI component gallery: every widget in its states, the
//! materials, the type scale and the motion system, with live theme,
//! density, tier and accessibility switches.
//!
//! The pages are a headless library: [`mount`] builds the gallery in any
//! [`App`], so the binary (`cargo run -p livspeil-ui-gallery`), the tests
//! (`TestHost`) and the screenshot harness (`cargo xtask shot gallery-*`)
//! render the same views.
//!
//! - A Frost sidebar lists the [`Page`]s in [`Group`]s (Controls,
//!   Containers, System) and scrolls when the window is short. Cmd/Ctrl+1..9
//!   and Cmd/Ctrl+0 switch to the first ten pages (later pages have no
//!   shortcut) (the bindings live in
//!   the keymap as `gallery.page-<n>`).
//! - Pages taller than the window scroll under their title
//!   ([`Page::scrolls`]); the others fit themselves to the window. Columns
//!   stack, and the toolbar and control rows wrap, when the window is narrow.
//! - [`Chrome`]: system window decorations, or (macOS) the gallery's own
//!   [`Titlebar`](livspeil_ui_widgets::Titlebar) under a transparent
//!   native one ([`mount_with`]).
//! - The toolbar switches theme (dark/light), density (Simple/Pro),
//!   material tier (T0/T1/T2), Reduce motion and Reduce transparency.
//!   It reads and writes the app's theme and motion configuration, so hot
//!   reloaded `theme.toml` / `motion.toml` show up in it too.

mod containers;
mod overlays;
mod pages;

use std::sync::Mutex;

/// Why backdrop blur is off, if it is (set by the app from the renderer;
/// the Materials page shows it). `None`: blur works or is not known yet.
static BLUR_NOTE: Mutex<Option<String>> = Mutex::new(None);

/// Records why blur is in fallback (`None` clears it). Pages read it when
/// they are built, so it shows from the next rebuild (page switch, theme
/// edit).
pub fn set_blur_note(reason: Option<String>) {
    if let Ok(mut n) = BLUR_NOTE.lock() {
        *n = reason;
    }
}

/// The current blur fallback reason.
#[must_use]
pub fn blur_note() -> Option<String> {
    BLUR_NOTE.lock().ok().and_then(|n| n.clone())
}

use std::cell::OnceCell;
use std::rc::Rc;

use livspeil_ui_core::{
    App, Element, ElementId, Key, Keymap, Keystroke, Model, Phase, Render, Role, View, ViewContext,
    actions, div, text,
};
use livspeil_ui_layout::style::{FlexWrap, JustifyContent, length};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_text::FontSystem;
use livspeil_ui_theme::{Color, Density, Material, Scheme, Theme, Tier};
use livspeil_ui_widgets::keymap::actions as widget_actions;
use livspeil_ui_widgets::{
    DemoGraph, Env, Label, MenuBar, NodeCanvas, ScrollView, SegmentedControl, Titlebar, Toggle,
    Tone, WidgetRoot,
};

pub use containers::{Containers, LAYERS, MenuChecks, STRESS_NODES, VIRTUAL_ROWS, row_title};
pub use overlays::Overlays;
pub use pages::State;

/// The window's logical size (also the screenshot size).
pub const WINDOW_SIZE: (f32, f32) = (1000.0, 640.0);
/// Space left and right of a page.
const PAGE_PADDING: f32 = 24.0;
/// Space under a page.
const PAGE_BOTTOM: f32 = 20.0;
/// Sidebar width including its margin.
pub const SIDEBAR_WIDTH: f32 = 220.0;
/// Action prefix of the page shortcuts: `gallery.page-1` … `gallery.page-10`.
pub const PAGE_ACTION: &str = "gallery.page-";

/// A sidebar group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// Buttons, toggles, fields and other controls.
    Controls,
    /// Scroll views, docks, menus and the node canvas.
    Containers,
    /// Materials, type, motion and theme.
    System,
}

impl Group {
    /// Every group in sidebar order.
    pub const ALL: [Group; 3] = [Group::Controls, Group::Containers, Group::System];

    /// The sidebar heading.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Group::Controls => "Controls",
            Group::Containers => "Containers",
            Group::System => "System",
        }
    }
}

/// A gallery page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    /// Button variants and states, icon buttons.
    Buttons,
    /// Toggles and check boxes (with a mixed parent).
    Toggles,
    /// Radio groups and segmented controls.
    Radios,
    /// Sliders and number fields (drag to scrub).
    Sliders,
    /// Text fields, including Georgian and right-to-left text.
    TextFields,
    /// Selects and tooltips.
    Select,
    /// A scroll view over long content and a virtual list of 100 000 rows.
    Scroll,
    /// Panels in a dock: splitters and collapsible sides.
    Dock,
    /// A menu bar and a context menu.
    Menus,
    /// The node canvas over a sample graph or a 500-node stress graph.
    NodeCanvas,
    /// Icon buttons, tabs, popovers, dialogs, sheets and toasts.
    Overlays,
    /// Solid, Frost, Glass and Ink over a colorful background.
    Materials,
    /// Type scale, weights and tabular digits.
    Typography,
    /// Spring playground and motion settings.
    Motion,
    /// Color roles and density.
    Theme,
}

impl Page {
    /// Every page in sidebar order.
    pub const ALL: [Page; 15] = [
        Page::Buttons,
        Page::Toggles,
        Page::Radios,
        Page::Sliders,
        Page::TextFields,
        Page::Select,
        Page::Scroll,
        Page::Dock,
        Page::Menus,
        Page::NodeCanvas,
        Page::Overlays,
        Page::Materials,
        Page::Typography,
        Page::Motion,
        Page::Theme,
    ];

    /// Sidebar title.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Page::Buttons => "Buttons",
            Page::Toggles => "Toggles & checks",
            Page::Radios => "Radios & segmented",
            Page::Sliders => "Sliders & numbers",
            Page::TextFields => "Text fields",
            Page::Select => "Select & tooltips",
            Page::Scroll => "Scrolling",
            Page::Dock => "Panels & dock",
            Page::Menus => "Menus",
            Page::NodeCanvas => "Node canvas",
            Page::Materials => "Materials",
            Page::Typography => "Typography",
            Page::Motion => "Motion",
            Page::Theme => "Theme",
            Page::Overlays => "Overlays & tabs",
        }
    }

    /// Short name (screenshot scene names: `gallery-<slug>`).
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Page::Buttons => "buttons",
            Page::Toggles => "toggles",
            Page::Radios => "radios",
            Page::Sliders => "sliders",
            Page::TextFields => "text-fields",
            Page::Select => "select",
            Page::Scroll => "scroll",
            Page::Dock => "dock",
            Page::Menus => "menus",
            Page::NodeCanvas => "node-canvas",
            Page::Materials => "materials",
            Page::Typography => "typography",
            Page::Motion => "motion",
            Page::Theme => "theme",
            Page::Overlays => "overlays",
        }
    }

    /// Whether the page scrolls when it is taller than the window. The
    /// others size themselves to the window (their content scrolls or pans
    /// by itself).
    #[must_use]
    pub fn scrolls(self) -> bool {
        !matches!(
            self,
            Page::Scroll | Page::Dock | Page::Menus | Page::NodeCanvas
        )
    }

    /// The sidebar group.
    #[must_use]
    pub fn group(self) -> Group {
        match self {
            Page::Buttons
            | Page::Toggles
            | Page::Radios
            | Page::Sliders
            | Page::TextFields
            | Page::Select => Group::Controls,
            Page::Scroll | Page::Dock | Page::Menus | Page::NodeCanvas | Page::Overlays => {
                Group::Containers
            }
            Page::Materials | Page::Typography | Page::Motion | Page::Theme => Group::System,
        }
    }

    /// Position in [`Page::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        Page::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// The shortcut digit of the first ten pages (1..9, then 0).
    #[must_use]
    pub fn shortcut(self) -> Option<char> {
        match self.index() {
            9 => Some('0'),
            // Indices 0..=8 map to '1'..='9'.
            i @ 0..=8 => char::from_digit(u32::try_from(i + 1).ok()?, 10),
            _ => None,
        }
    }

    /// The page's action (`gallery.page-<n>`, 1-based; bound to a shortcut
    /// for the first ten pages).
    #[must_use]
    pub fn action(self) -> String {
        format!("{PAGE_ACTION}{}", self.index() + 1)
    }
}

/// How the window's title area is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Chrome {
    /// The system draws the titlebar (window decorations on Linux and
    /// Windows, a standard macOS titlebar): the gallery starts at the top.
    #[default]
    System,
    /// The gallery draws a [`Titlebar`](livspeil_ui_widgets::Titlebar)
    /// (a window drag region with the traffic-light inset) for a macOS
    /// window with a transparent, full-size-content titlebar.
    Titlebar,
}

impl Chrome {
    /// What suits the platform the gallery was built for: our titlebar on
    /// macOS, system decorations elsewhere (X11 and Wayland behave
    /// natively: server-side decorations, or winit's client-side ones
    /// where the compositor has none).
    #[must_use]
    pub fn native() -> Chrome {
        if cfg!(target_os = "macos") {
            Chrome::Titlebar
        } else {
            Chrome::System
        }
    }
}

/// The page shortcuts: `cmd-<n>` and `ctrl-<n>` to `gallery.page-<n>`.
#[must_use]
pub fn keymap() -> Keymap {
    let mut km = Keymap::default();
    for page in Page::ALL {
        let Some(digit) = page.shortcut() else {
            continue;
        };
        for m in ["cmd", "ctrl"] {
            if let Ok(k) = Keystroke::parse(&format!("{m}-{digit}")) {
                km.bind(k, &page.action(), None);
            }
        }
    }
    km
}

/// What [`mount`] built.
#[derive(Debug)]
pub struct Mounted {
    /// The widget environment (fonts, clipboard, focus tracking).
    pub env: Env,
    /// The gallery view.
    pub gallery: View<Gallery>,
    /// The widget root (gallery plus overlay layer).
    pub root: View<WidgetRoot>,
    /// The app root: handles the page shortcuts whatever has focus.
    pub shell: View<Shell>,
    /// The Node canvas page's canvases (sample, stress), for scenarios.
    pub canvases: [View<NodeCanvas<DemoGraph>>; 2],
    /// Which of [`Mounted::canvases`] the page shows.
    pub canvas_choice: Model<usize>,
}

/// Build the gallery showing `page` with system window chrome
/// ([`Chrome::System`]) and make it the app's root. See [`mount_with`].
pub fn mount(app: &mut App, fonts: FontSystem, page: Page) -> Mounted {
    mount_with(app, fonts, page, Chrome::System)
}

/// Build the gallery showing `page` with `chrome` and make it the app's
/// root. Installs the widget keymap and the page shortcuts. Layout text
/// measurement is up to the caller (`app.set_text_measurer`).
pub fn mount_with(app: &mut App, fonts: FontSystem, page: Page, chrome: Chrome) -> Mounted {
    let env = Env::new(app, fonts);
    let mut km = app.keymap().clone();
    km.extend(&keymap());
    app.set_keymap(km);
    let state = State::new(app, &env);
    let menubar = state.containers.menubar;
    let canvases = state.containers.canvases;
    let canvas_choice = state.containers.graph;
    let page = app.new_model(page);
    let sidebar = app.new_view(
        ScrollView::new(&env, move |app, _| {
            sidebar_list(app.theme(), *page.read(app), page)
        })
        .name("sidebar-scroll")
        .label("Pages")
        .focusable(false),
    );
    let state = Rc::new(state);
    // The page's content needs the gallery's handle, which does not exist
    // yet.
    let handle: Rc<OnceCell<View<Gallery>>> = Rc::default();
    let (content_env, content_state, content_view) = (env.clone(), state.clone(), handle.clone());
    let scroll = app.new_view(
        ScrollView::new(&env, move |app, _| {
            let current = *page.read(app);
            let content = content_view.get().map(|view| {
                pages::render(current, &content_state, &content_env, *view, app)
                    // As tall as its content: the scroll view does the rest.
                    .style(|s| s.flex_shrink = 0.0)
            });
            div().flex_col().child(
                div()
                    .key(current.slug())
                    .flex_col()
                    .px(PAGE_PADDING)
                    .style(|s| s.padding.bottom = length(PAGE_BOTTOM))
                    .children(content),
            )
        })
        .name("page-scroll")
        .label("Page")
        .focusable(false),
    );
    // A new page starts at its top.
    let _ = app.observe(page.id(), move |app| {
        scroll.update(app, |s, cx| {
            s.scroller_mut().jump_to(Vec2::ZERO);
            cx.notify();
        });
    });
    let gallery = app.new_view(Gallery {
        env: env.clone(),
        page,
        chrome,
        sidebar,
        scroll,
        state,
    });
    let _ = handle.set(gallery);
    let root = env.root(app, gallery);
    let shell = app.new_view(Shell {
        env: env.clone(),
        page,
        root,
        menubar,
    });
    app.set_root(shell);
    // Actions bubble from the focused element, and with nothing focused
    // from the core's window element only. Start with focus on the shell
    // (the window's first child, focusable but not a tab stop) so the page
    // shortcuts work before anything is clicked.
    app.focus(ElementId::ROOT.child_index(0));
    Mounted {
        env,
        gallery,
        root,
        shell,
        canvases,
        canvas_choice,
    }
}

/// The app root around the [`WidgetRoot`]. Actions bubble from the focused
/// element to the root, so with nothing focused only the root sees them:
/// the page shortcuts, the first Tab, and F10 / Alt+letter for the Menus
/// page's menu bar live here, and [`mount`] focuses it.
pub struct Shell {
    env: Env,
    page: Model<Page>,
    root: View<WidgetRoot>,
    menubar: View<MenuBar>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell").finish_non_exhaustive()
    }
}

impl Render for Shell {
    fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
        let model = self.page;
        let bar = self.menubar;
        let mut root = div().name("gallery-shell").w_full().h_full().tab_index(-1);
        for p in Page::ALL {
            root = root.on_action(&p.action(), move |cx| set_page(model, cx, p));
        }
        let (next, prev) = (self.env.clone(), self.env.clone());
        root.on_action(actions::FOCUS_NEXT, move |cx| {
            cx.focus_next();
            next.sync_focus(cx);
        })
        .on_action(actions::FOCUS_PREV, move |cx| {
            cx.focus_prev();
            prev.sync_focus(cx);
        })
        // The widget root handles these once focus is inside it; the menu
        // bar ignores them while the Menus page is not shown.
        .on_action(widget_actions::FOCUS_MENUBAR, move |cx| {
            MenuBar::focus(bar, cx);
        })
        .on_key(Phase::Bubble, move |ev, cx| {
            let m = ev.modifiers;
            if let (true, true, false, false, Key::Char(c)) =
                (ev.pressed, m.alt, m.ctrl, m.cmd, ev.key)
                && MenuBar::open_mnemonic(bar, cx, c)
            {
                cx.stop_propagation();
            }
        })
        .child(Element::view(&self.root))
    }
}

/// The gallery view: sidebar, toolbar and the current page.
pub struct Gallery {
    env: Env,
    page: Model<Page>,
    chrome: Chrome,
    sidebar: View<ScrollView>,
    scroll: View<ScrollView>,
    state: Rc<State>,
}

impl std::fmt::Debug for Gallery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gallery").finish_non_exhaustive()
    }
}

impl Gallery {
    /// The current page.
    #[must_use]
    pub fn page(&self, app: &App) -> Page {
        *self.page.read(app)
    }

    /// Show `page`.
    pub fn set_page(&self, app: &mut App, page: Page) {
        set_page(self.page, app, page);
    }

    /// Page state (models and stateful widgets).
    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The window chrome.
    #[must_use]
    pub fn chrome(&self) -> Chrome {
        self.chrome
    }

    /// The sidebar's scroll view.
    #[must_use]
    pub fn sidebar(&self) -> View<ScrollView> {
        self.sidebar
    }

    /// The scroll view of pages that can be taller than the window
    /// ([`Page::scrolls`]).
    #[must_use]
    pub fn page_scroll(&self) -> View<ScrollView> {
        self.scroll
    }
}

fn set_page(model: Model<Page>, app: &mut App, page: Page) {
    if *model.read(app) != page {
        model.update(app, |p, cx| {
            *p = page;
            cx.notify();
        });
    }
}

/// Change the theme with `f` (every view re-renders).
pub(crate) fn edit_theme(app: &mut App, f: impl FnOnce(&mut Theme)) {
    let mut t = app.theme().clone();
    f(&mut t);
    if &t != app.theme() {
        app.set_theme(t);
    }
}

/// Change the motion configuration with `f` and re-render `view`.
pub(crate) fn edit_motion<V: Render>(
    app: &mut App,
    view: View<V>,
    f: impl FnOnce(&mut livspeil_ui_theme::MotionConfig),
) {
    let mut m = app.motion().clone();
    f(&mut m);
    if &m != app.motion() {
        app.set_motion(m);
        view.update(app, |_, cx| cx.notify());
    }
}

fn group_heading(theme: &Theme, group: Group) -> Element {
    div()
        .key(group.title())
        .px(10.0)
        .style(|s| {
            s.padding.top = length(10.0);
            s.padding.bottom = length(2.0);
        })
        .child(
            text(group.title().to_uppercase())
                .font_size(11.0)
                .font_weight(600.0)
                .text_color(theme.colors().text_tertiary),
        )
}

/// The page list (the sidebar scroll view's content).
fn sidebar_list(theme: &Theme, current: Page, model: Model<Page>) -> Element {
    let c = theme.colors();
    let row_h = theme.metrics().row_height;
    let mut list = div()
        .name("pages")
        .flex_col()
        .gap(2.0)
        .px(8.0)
        .style(|s| {
            s.padding.top = length(2.0);
            s.padding.bottom = length(12.0);
        })
        .semantics(|s| s.role(Role::TabList).label("Pages"));
    for group in Group::ALL {
        list = list.child(group_heading(theme, group));
        for page in Page::ALL.into_iter().filter(|p| p.group() == group) {
            let selected = page == current;
            let mut row = div()
                .key(page.slug())
                .name(format!("page:{}", page.title()))
                .h(row_h)
                .style(|s| s.flex_shrink = 0.0)
                .px(10.0)
                .flex_row()
                .items_center()
                .gap(8.0)
                .rounded(theme.tokens.radii.sm)
                .focusable()
                .on_click(move |_, cx| set_page(model, cx, page))
                .semantics(|s| s.role(Role::Tab).label(page.title()).selected(selected))
                .child(
                    text(page.title())
                        .font_size(theme.metrics().body)
                        .font_weight(if selected { 600.0 } else { 400.0 })
                        .text_color(if selected {
                            c.text_primary
                        } else {
                            c.text_secondary
                        })
                        .grow(),
                );
            if let Some(digit) = page.shortcut() {
                row = row.child(
                    text(digit.to_string())
                        .font_size(11.0)
                        .text_color(c.text_tertiary),
                );
            }
            row = if selected {
                row.bg(theme.accent().with_alpha(0.24))
            } else {
                row.hover(|r| r.bg(c.text_primary.with_alpha(0.06)))
            };
            list = list.child(row);
        }
    }
    list
}

fn sidebar(theme: &Theme, scroll: View<ScrollView>) -> Element {
    div()
        .key("sidebar")
        .w(SIDEBAR_WIDTH)
        .style(|s| s.flex_shrink = 0.0)
        .h_full()
        .p(8.0)
        .child(
            div()
                .w_full()
                .h_full()
                .material(Material::Frost)
                .rounded(theme.tokens.radii.lg)
                .clip()
                .style(|s| s.padding.top = length(4.0))
                .child(Element::view(&scroll)),
        )
}

fn toolbar(theme: &Theme, motion_reduce: bool, view: View<Gallery>) -> Element {
    let seg_w = 54.0;
    let scheme = match theme.scheme {
        Scheme::Dark => 0,
        Scheme::Light => 1,
    };
    let density = match theme.density {
        Density::Simple => 0,
        Density::Pro => 1,
    };
    div()
        .key("toolbar")
        .name("toolbar")
        // Right-aligned; wraps onto more lines in a narrow window.
        .style(|s| {
            s.min_size.height = length(52.0);
            s.flex_shrink = 0.0;
            s.flex_wrap = FlexWrap::Wrap;
            s.justify_content = JustifyContent::FLEX_END;
            s.padding.top = length(10.0);
            s.padding.bottom = length(10.0);
        })
        .flex_row()
        .items_center()
        .gap(12.0)
        .px(20.0)
        .semantics(|s| s.role(Role::Toolbar).label("Appearance"))
        .child(
            SegmentedControl::new(["Dark", "Light"], scheme)
                .label("Theme")
                .segment_width(seg_w)
                .on_change(|i, cx| {
                    edit_theme(cx, |t| {
                        t.scheme = if i == 0 { Scheme::Dark } else { Scheme::Light };
                    });
                })
                .render(theme),
        )
        .child(
            SegmentedControl::new(["Simple", "Pro"], density)
                .label("Density")
                .segment_width(seg_w)
                .on_change(|i, cx| {
                    edit_theme(cx, |t| {
                        t.density = if i == 0 {
                            Density::Simple
                        } else {
                            Density::Pro
                        };
                    });
                })
                .render(theme),
        )
        .child(tier_control(theme))
        .child(
            Toggle::new(motion_reduce)
                .label("Reduce motion")
                .on_change(move |v, cx| edit_motion(cx, view, |m| m.reduce_motion = v))
                .render(theme),
        )
        .child(
            Toggle::new(theme.reduce_transparency)
                .label("Reduce transparency")
                .on_change(|v, cx| edit_theme(cx, |t| t.reduce_transparency = v))
                .render(theme),
        )
}

/// The material tier switch (toolbar and Materials page).
pub(crate) fn tier_control(theme: &Theme) -> Element {
    let tier = match theme.tier {
        Tier::T0 => 0,
        Tier::T1 => 1,
        Tier::T2 => 2,
    };
    SegmentedControl::new(["T0", "T1", "T2"], tier)
        .label("Material tier")
        .segment_width(36.0)
        .on_change(|i, cx| {
            edit_theme(cx, |t| {
                t.tier = match i {
                    0 => Tier::T0,
                    1 => Tier::T1,
                    _ => Tier::T2,
                };
            });
        })
        .render(theme)
}

/// Soft colored shapes behind the sidebar, so Frost has something to blur.
fn backdrop(theme: &Theme) -> Element {
    let a = if theme.scheme == Scheme::Dark {
        1.0
    } else {
        0.7
    };
    let blob = |key: &'static str, r: Rect, color: Color| {
        div()
            .key(key)
            .absolute(r)
            .rounded(r.width / 2.0)
            .bg(color)
            .hit_test(false)
    };
    div()
        .key("backdrop")
        .absolute(Rect::new(0.0, 0.0, WINDOW_SIZE.0, WINDOW_SIZE.1))
        .hit_test(false)
        .child(blob(
            "b1",
            Rect::new(-40.0, 60.0, 220.0, 220.0),
            Color::oklch(0.62, 0.17, 285.0, a),
        ))
        .child(blob(
            "b2",
            Rect::new(56.0, 300.0, 150.0, 150.0),
            Color::oklch(0.72, 0.14, 200.0, a),
        ))
        .child(blob(
            "b3",
            Rect::new(-30.0, 470.0, 160.0, 160.0),
            Color::oklch(0.74, 0.16, 40.0, a),
        ))
}

impl Render for Gallery {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let page = *self.page.read(cx);
        let view = cx.view();
        let reduce_motion = cx.motion().reduce_motion;
        if page == Page::Motion {
            pages::step_motion(&self.state, cx);
        }
        let content = if page.scrolls() {
            // Whatever re-renders the gallery also changes the page.
            self.scroll.update(cx, |_, cx| cx.notify());
            div()
                .key("scroll-area")
                .grow()
                .style(|s| s.min_size.height = length(0.0))
                .child(Element::view(&self.scroll))
        } else {
            div()
                .key(page.slug())
                .grow()
                .flex_col()
                .px(PAGE_PADDING)
                .style(|s| {
                    s.padding.bottom = length(PAGE_BOTTOM);
                    s.min_size.height = length(0.0);
                })
                .child(pages::render(page, &self.state, &self.env, view, cx))
        };
        let title = Label::new(page.title())
            .tone(Tone::Primary)
            .render(&theme)
            .font_size(theme.tokens.type_scale.sizes[5])
            .font_weight(600.0)
            .semantics(|s| s.role(Role::Heading));
        let main = div()
            .key("main")
            .grow()
            .h_full()
            .style(|s| s.min_size.width = length(0.0))
            .flex_col()
            .child(toolbar(&theme, reduce_motion, view))
            .child(
                div()
                    .name("page")
                    .grow()
                    .flex_col()
                    .style(|s| s.min_size.height = length(0.0))
                    .gap(16.0)
                    .clip()
                    .child(
                        div()
                            .px(PAGE_PADDING)
                            .style(|s| s.flex_shrink = 0.0)
                            .child(title),
                    )
                    .child(content),
            );
        let body = div()
            .key("body")
            .w_full()
            .grow()
            .style(|s| s.min_size.height = length(0.0))
            .flex_row()
            .child(sidebar(&theme, self.sidebar))
            .child(main);
        let mut root = div()
            .name("gallery")
            .w_full()
            .h_full()
            .bg(theme.colors().surface[0])
            .flex_col()
            .clip()
            .child(backdrop(&theme));
        if self.chrome == Chrome::Titlebar {
            // Under a transparent macOS titlebar: the traffic lights sit in
            // the bar's left inset, and the bar is the window's drag region.
            root = root.child(Titlebar::new("Livspeil Gallery").render(&theme));
        }
        root.child(body)
    }
}
