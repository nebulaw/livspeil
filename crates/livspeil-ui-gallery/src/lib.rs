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
//! - A Frost sidebar lists the [`Page`]s (Cmd/Ctrl+1..9 and Cmd/Ctrl+0
//!   switch pages; the bindings live in the keymap as `gallery.page-<n>`).
//! - The toolbar switches theme (dark/light), density (Simple/Pro),
//!   material tier (T0/T1/T2), Reduce motion and Reduce transparency.
//!   It reads and writes the app's theme and motion configuration, so hot
//!   reloaded `theme.toml` / `motion.toml` show up in it too.

mod pages;

use std::rc::Rc;

use livspeil_ui_core::{
    App, Element, Keymap, Keystroke, Model, Render, Role, View, ViewContext, div, text,
};
use livspeil_ui_layout::Rect;
use livspeil_ui_layout::style::length;
use livspeil_ui_text::FontSystem;
use livspeil_ui_theme::{Color, Density, Material, Scheme, Theme, Tier};
use livspeil_ui_widgets::{Env, Label, SegmentedControl, Toggle, Tone, WidgetRoot};

pub use pages::State;

/// The window's logical size (also the screenshot size).
pub const WINDOW_SIZE: (f32, f32) = (1000.0, 640.0);
/// Sidebar width including its margin.
pub const SIDEBAR_WIDTH: f32 = 220.0;
/// Space left free at the top for the macOS titlebar (traffic lights).
pub const TOP_INSET: f32 = 28.0;
/// Action prefix of the page shortcuts: `gallery.page-1` … `gallery.page-10`.
pub const PAGE_ACTION: &str = "gallery.page-";

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
    pub const ALL: [Page; 10] = [
        Page::Buttons,
        Page::Toggles,
        Page::Radios,
        Page::Sliders,
        Page::TextFields,
        Page::Select,
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
            Page::Materials => "Materials",
            Page::Typography => "Typography",
            Page::Motion => "Motion",
            Page::Theme => "Theme",
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
            Page::Materials => "materials",
            Page::Typography => "typography",
            Page::Motion => "motion",
            Page::Theme => "theme",
        }
    }

    /// Position in [`Page::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        Page::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// The shortcut digit (1..9, then 0).
    #[must_use]
    pub fn shortcut(self) -> char {
        match self.index() {
            9 => '0',
            // Indices 0..=8 map to '1'..='9'.
            i => char::from_digit(u32::try_from(i + 1).unwrap_or(1), 10).unwrap_or('1'),
        }
    }

    /// The page's shortcut action.
    #[must_use]
    pub fn action(self) -> String {
        format!("{PAGE_ACTION}{}", self.index() + 1)
    }
}

/// The page shortcuts: `cmd-<n>` and `ctrl-<n>` to `gallery.page-<n>`.
#[must_use]
pub fn keymap() -> Keymap {
    let mut km = Keymap::default();
    for page in Page::ALL {
        for m in ["cmd", "ctrl"] {
            if let Ok(k) = Keystroke::parse(&format!("{m}-{}", page.shortcut())) {
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
    /// The root view (gallery plus overlay layer), already the app root.
    pub root: View<WidgetRoot>,
}

/// Build the gallery showing `page` and make it the app's root. Installs
/// the widget keymap and the page shortcuts. Layout text measurement is
/// up to the caller (`app.set_text_measurer`).
pub fn mount(app: &mut App, fonts: FontSystem, page: Page) -> Mounted {
    let env = Env::new(app, fonts);
    let mut km = app.keymap().clone();
    km.extend(&keymap());
    app.set_keymap(km);
    let state = State::new(app, &env);
    let page = app.new_model(page);
    let gallery = app.new_view(Gallery {
        env: env.clone(),
        page,
        state: Rc::new(state),
    });
    let root = env.root(app, gallery);
    app.set_root(root);
    Mounted { env, gallery, root }
}

/// The gallery view: sidebar, toolbar and the current page.
pub struct Gallery {
    env: Env,
    page: Model<Page>,
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

fn sidebar(theme: &Theme, current: Page, model: Model<Page>) -> Element {
    let c = theme.colors();
    let row_h = theme.metrics().row_height;
    let mut list = div()
        .name("pages")
        .flex_col()
        .gap(2.0)
        .semantics(|s| s.role(Role::TabList).label("Pages"));
    for page in Page::ALL {
        let selected = page == current;
        let mut row = div()
            .key(page.slug())
            .name(format!("page:{}", page.title()))
            .h(row_h)
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
            )
            .child(
                text(page.shortcut().to_string())
                    .font_size(11.0)
                    .text_color(c.text_tertiary),
            );
        row = if selected {
            row.bg(theme.accent().with_alpha(0.24))
        } else {
            row.hover(|r| r.bg(c.text_primary.with_alpha(0.06)))
        };
        list = list.child(row);
    }
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
                .flex_col()
                .px(8.0)
                .style(|s| {
                    s.padding.top = length(TOP_INSET + 12.0);
                    s.padding.bottom = length(12.0);
                })
                .gap(8.0)
                .child(
                    div().px(10.0).child(
                        text("COMPONENTS")
                            .font_size(11.0)
                            .font_weight(600.0)
                            .text_color(c.text_tertiary),
                    ),
                )
                .child(list),
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
        .h(52.0)
        .flex_row()
        .items_center()
        .gap(12.0)
        .px(20.0)
        .semantics(|s| s.role(Role::Toolbar).label("Appearance"))
        .child(div().grow())
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
        let model = self.page;
        let view = cx.view();
        let reduce_motion = cx.motion().reduce_motion;
        let content = pages::render(page, &self.state, &self.env, view, cx);
        let mut root = div()
            .name("gallery")
            .w_full()
            .h_full()
            .bg(theme.colors().surface[0])
            .flex_row()
            .clip();
        for p in Page::ALL {
            root = root.on_action(&p.action(), move |cx| set_page(model, cx, p));
        }
        root.child(backdrop(&theme))
            .child(sidebar(&theme, page, model))
            .child(
                div()
                    .key("main")
                    .grow()
                    .h_full()
                    .style(|s| s.min_size.width = length(0.0))
                    .flex_col()
                    .child(toolbar(&theme, reduce_motion, view))
                    .child(
                        div()
                            .key(page.slug())
                            .name("page")
                            .grow()
                            .flex_col()
                            .px(24.0)
                            .style(|s| s.padding.bottom = length(20.0))
                            .gap(16.0)
                            .clip()
                            .child(
                                Label::new(page.title())
                                    .tone(Tone::Primary)
                                    .render(&theme)
                                    .font_size(theme.tokens.type_scale.sizes[5])
                                    .font_weight(600.0)
                                    .semantics(|s| s.role(Role::Heading)),
                            )
                            .child(content),
                    ),
            )
    }
}
