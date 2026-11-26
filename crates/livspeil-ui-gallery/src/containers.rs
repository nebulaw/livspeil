// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The container pages: Scrolling (a scroll view and a 100 000-row virtual
//! list), Panels & dock (a mini editor), Menus (a menu bar and a context
//! menu) and Node canvas (sample and stress graphs).

use livspeil_ui_core::{App, Element, FontFeature, Model, Role, View, div, text};
use livspeil_ui_layout::style::length;
use livspeil_ui_layout::{Canvas, DockEdge, Rect, Vec2};
use livspeil_ui_theme::{Color, Theme};
use livspeil_ui_widgets::{
    Button, ContextMenu, DemoGraph, DockView, Env, Menu, MenuBar, MenuItem, NodeCanvas,
    NumberField, Panel, ScrollView, SegmentedControl, Slider, Toggle, VirtualList,
};

use crate::Gallery;
use crate::pages::{caption, card, column, row, set};

/// Rows of the virtual list.
pub const VIRTUAL_ROWS: usize = 100_000;
/// Row height of the virtual list.
pub const VIRTUAL_ROW_HEIGHT: f32 = 28.0;
/// Nodes of the stress graph.
pub const STRESS_NODES: usize = 500;
/// Links of the stress graph.
pub const STRESS_LINKS: usize = 750;
/// Index of the View menu in the menu bar.
const VIEW_MENU: usize = 3;

/// The dock's layers (bottom to top in the stage).
pub const LAYERS: [&str; 5] = ["Sky", "Sun", "Island", "Lighthouse", "Water"];

/// Check marks of the View menu and the context menu.
#[derive(Clone, Copy, Debug)]
pub struct MenuChecks {
    /// Show Grid.
    pub grid: Model<bool>,
    /// Snap to Grid.
    pub snap: Model<bool>,
    /// Show Minimap (context menu).
    pub minimap: Model<bool>,
}

/// State of the container pages.
pub struct Containers {
    /// Scrolling: long content.
    pub long: View<ScrollView>,
    /// Scrolling: the virtual list.
    pub list: View<ScrollView>,
    /// Dock: the dock view.
    pub dock: View<DockView>,
    /// Dock: the selected layer (index into [`LAYERS`]).
    pub layer: Model<usize>,
    /// Dock: layer visibility.
    pub visible: [Model<bool>; 5],
    /// Dock: stage opacity.
    pub opacity: Model<f64>,
    /// Dock: blend mode (Normal, Add).
    pub blend: Model<usize>,
    /// Dock: timeline position (seconds).
    pub time: Model<f32>,
    /// Menus: the menu bar.
    pub menubar: View<MenuBar>,
    /// Menus: the last chosen command.
    pub last_command: Model<String>,
    /// Menus: check marks.
    pub checks: MenuChecks,
    /// Node canvas: sample and stress canvases.
    pub canvases: [View<NodeCanvas<DemoGraph>>; 2],
    /// Node canvas: which graph is shown (0 sample, 1 stress).
    pub graph: Model<usize>,
}

impl std::fmt::Debug for Containers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Containers").finish_non_exhaustive()
    }
}

const WORDS: [&str; 12] = [
    "Aurora", "Harbor", "Lantern", "Meadow", "Tidal", "Ember", "Glacier", "Orchard", "Quartz",
    "Signal", "Velvet", "Willow",
];

/// The label of virtual list row `i`.
#[must_use]
pub fn row_title(i: usize) -> String {
    format!(
        "{} {}",
        WORDS[i % WORDS.len()],
        WORDS[(i / WORDS.len() + 5) % WORDS.len()].to_lowercase()
    )
}

#[allow(clippy::cast_precision_loss)]
fn hue(i: usize) -> f32 {
    (i * 37 % 360) as f32
}

fn long_content(app: &App) -> Element {
    let theme = app.theme();
    let c = theme.colors();
    let sections = [
        "Scenes",
        "Charms",
        "Wires",
        "Materials",
        "Motion",
        "Layers",
        "Sounds",
        "Lights",
    ];
    let mut col = div().flex_col().p(8.0).gap(2.0);
    for (s, name) in sections.into_iter().enumerate() {
        col = col.child(
            div()
                .key(format!("h-{s}"))
                .px(10.0)
                .style(|st| {
                    st.padding.top = length(if s == 0 { 4.0 } else { 12.0 });
                    st.padding.bottom = length(4.0);
                })
                .child(
                    text(name.to_uppercase())
                        .font_size(11.0)
                        .font_weight(600.0)
                        .text_color(c.text_tertiary),
                ),
        );
        for k in 0..6 {
            let i = s * 6 + k;
            col = col.child(
                div()
                    .key(format!("item-{i}"))
                    .h(30.0)
                    .px(10.0)
                    .flex_row()
                    .items_center()
                    .gap(10.0)
                    .rounded(theme.tokens.radii.sm)
                    .bg(c.surface[if i % 2 == 0 { 1 } else { 2 }])
                    .child(div().size(10.0, 10.0).rounded(5.0).bg(Color::oklch(
                        0.72,
                        0.13,
                        hue(i),
                        1.0,
                    )))
                    .child(text(format!("{} {}", name.trim_end_matches('s'), k + 1)))
                    .child(div().grow())
                    .child(
                        text(format!("{}%", 100 - i * 2))
                            .font_size(11.0)
                            .text_color(c.text_tertiary)
                            .font_feature(FontFeature::TNUM),
                    ),
            );
        }
    }
    col
}

fn virtual_row(app: &App, i: usize) -> Element {
    let theme = app.theme();
    let c = theme.colors();
    let mut r = div()
        .w_full()
        .h_full()
        .px(12.0)
        .flex_row()
        .items_center()
        .gap(12.0)
        .child(
            text(format!("{:>6}", i + 1))
                .font_size(12.0)
                .font_feature(FontFeature::TNUM)
                .text_color(c.text_tertiary)
                .w(48.0),
        )
        .child(
            div()
                .size(8.0, 8.0)
                .rounded(4.0)
                .bg(Color::oklch(0.72, 0.13, hue(i), 1.0)),
        )
        .child(text(row_title(i)).grow())
        .child(
            text(format!("{}.{:02} s", i % 60, (i * 7) % 100))
                .font_size(12.0)
                .font_feature(FontFeature::TNUM)
                .text_color(c.text_secondary),
        );
    if i % 2 == 1 {
        r = r.bg(c.text_primary.with_alpha(0.035));
    }
    r
}

fn file_menu(log: Model<String>) -> Menu {
    let cmd = |label: &'static str| {
        MenuItem::new(label).on_select(move |app| set(log, app, format!("File › {label}")))
    };
    Menu::new()
        .item(cmd("New Scene").icon("bx-grid-plus").shortcut("Ctrl+N"))
        .item(cmd("Open…").icon("bx-folder").shortcut("Ctrl+O"))
        .submenu(
            "Open Recent",
            Menu::new()
                .item(cmd("Lighthouse.room"))
                .item(cmd("Harbor at Night.room"))
                .item(cmd("Glacier.room"))
                .separator()
                .item(cmd("Clear Menu")),
        )
        .separator()
        .item(cmd("Save").shortcut("Ctrl+S"))
        .item(cmd("Save As…").shortcut("Ctrl+Shift+S"))
        .item(cmd("Export Video…").disabled(true))
        .separator()
        .item(cmd("Close Window").shortcut("Ctrl+W"))
}

fn edit_menu(log: Model<String>) -> Menu {
    let cmd = |label: &'static str| {
        MenuItem::new(label).on_select(move |app| set(log, app, format!("Edit › {label}")))
    };
    Menu::new()
        .item(cmd("Undo").shortcut("Ctrl+Z"))
        .item(cmd("Redo").shortcut("Ctrl+Shift+Z").disabled(true))
        .separator()
        .item(cmd("Cut").shortcut("Ctrl+X"))
        .item(cmd("Copy").icon("bx-copy").shortcut("Ctrl+C"))
        .item(cmd("Paste").shortcut("Ctrl+V"))
        .separator()
        .item(cmd("Select All").shortcut("Ctrl+A"))
}

fn toggle_check(app: &mut App, m: Model<bool>) {
    let v = !*m.read(app);
    set(m, app, v);
}

/// The View menu with the current check marks; choosing a check item
/// flips it and rebuilds the menu.
fn view_menu(app: &App, log: Model<String>, checks: MenuChecks, bar: View<MenuBar>) -> Menu {
    let check = |label: &'static str, m: Model<bool>| {
        MenuItem::new(label)
            .checked(*m.read(app))
            .on_select(move |app| {
                toggle_check(app, m);
                set(log, app, format!("View › {label}"));
                refresh_view_menu(app, log, checks, bar);
            })
    };
    let cmd = |label: &'static str| {
        MenuItem::new(label).on_select(move |app| set(log, app, format!("View › {label}")))
    };
    Menu::new()
        .item(check("Show Grid", checks.grid))
        .item(check("Snap to Grid", checks.snap))
        .separator()
        .item(cmd("Zoom In").shortcut("Ctrl+Plus"))
        .item(cmd("Zoom Out").shortcut("Ctrl+Minus"))
        .item(cmd("Actual Size").shortcut("Ctrl+Shift+0"))
        .separator()
        .item(cmd("Enter Full Screen").shortcut("F11"))
}

fn refresh_view_menu(app: &mut App, log: Model<String>, checks: MenuChecks, bar: View<MenuBar>) {
    let menu = view_menu(app, log, checks, bar);
    bar.update(app, |b, cx| {
        b.replace_menu(VIEW_MENU, menu);
        cx.notify();
    });
}

fn context_menu(app: &App, log: Model<String>, checks: MenuChecks) -> Menu {
    let cmd = |label: &'static str| {
        MenuItem::new(label).on_select(move |app| set(log, app, format!("Canvas › {label}")))
    };
    let check = |label: &'static str, m: Model<bool>| {
        MenuItem::new(label)
            .checked(*m.read(app))
            .on_select(move |app| {
                toggle_check(app, m);
                set(log, app, format!("Canvas › {label}"));
            })
    };
    Menu::new()
        .item(cmd("Add Node…").icon("bx-grid-plus").shortcut("Shift+A"))
        .item(cmd("Paste").shortcut("Ctrl+V"))
        .item(cmd("Delete").shortcut("Del").disabled(true))
        .separator()
        .item(check("Show Grid", checks.grid))
        .item(check("Show Minimap", checks.minimap))
        .separator()
        .submenu(
            "Align",
            Menu::new()
                .item(cmd("Left"))
                .item(cmd("Center"))
                .item(cmd("Right")),
        )
        .item(cmd("Frame All").shortcut("Home"))
}

// ---- dock ----

fn layer_color(i: usize) -> Color {
    match i {
        0 => Color::oklch(0.55, 0.09, 250.0, 1.0),
        1 => Color::oklch(0.85, 0.15, 85.0, 1.0),
        2 => Color::oklch(0.6, 0.12, 145.0, 1.0),
        3 => Color::oklch(0.92, 0.02, 90.0, 1.0),
        _ => Color::oklch(0.5, 0.12, 225.0, 1.0),
    }
}

/// The layers list (left panel).
fn layers_panel(app: &App, layer: Model<usize>, visible: [Model<bool>; 5]) -> Element {
    let theme = app.theme();
    let c = theme.colors();
    let current = *layer.read(app);
    let mut list = div()
        .name("layers")
        .flex_col()
        .gap(2.0)
        .p(6.0)
        .semantics(|s| s.role(Role::ListBox).label("Layers"));
    // Top of the stack first, like an editor's layers list.
    for i in (0..LAYERS.len()).rev() {
        let name = LAYERS[i];
        let selected = i == current;
        let shown = *visible[i].read(app);
        let mut r = div()
            .key(name)
            .name(format!("layer:{name}"))
            .h(26.0)
            .px(8.0)
            .flex_row()
            .items_center()
            .gap(8.0)
            .rounded(5.0)
            .focusable()
            .on_click(move |_, cx| set(layer, cx, i))
            .semantics(|s| s.role(Role::ListBoxOption).label(name).selected(selected))
            .child(
                div()
                    .size(12.0, 12.0)
                    .rounded(3.0)
                    .bg(layer_color(i))
                    .opacity(if shown { 1.0 } else { 0.35 }),
            )
            .child(
                text(name)
                    .text_color(if selected {
                        c.text_primary
                    } else {
                        c.text_secondary
                    })
                    .opacity(if shown { 1.0 } else { 0.5 }),
            );
        r = if selected {
            r.bg(theme.accent().with_alpha(0.28))
        } else {
            r.hover(|h| h.bg(c.text_primary.with_alpha(0.06)))
        };
        list = list.child(r);
    }
    Panel::new("Layers").frost().child(list).render(theme)
}

/// The viewport placeholder (center): a tiny scene of the layers.
fn viewport(app: &App, state: DockModels) -> Element {
    let theme = app.theme();
    let c = theme.colors();
    let current = *state.layer.read(app);
    #[allow(clippy::cast_possible_truncation)]
    let opacity = *state.opacity.read(app) as f32;
    let rects = [
        Rect::new(0.0, 0.0, 280.0, 120.0),
        Rect::new(196.0, 26.0, 40.0, 40.0),
        Rect::new(40.0, 96.0, 150.0, 44.0),
        Rect::new(120.0, 50.0, 16.0, 56.0),
        Rect::new(0.0, 120.0, 280.0, 60.0),
    ];
    let mut stage = div()
        .key("stage")
        .name("stage")
        .size(280.0, 180.0)
        .rounded(theme.tokens.radii.md)
        .clip()
        .bg(c.surface[1])
        .opacity(opacity);
    for (i, r) in rects.into_iter().enumerate() {
        if !*state.visible[i].read(app) {
            continue;
        }
        let radius = match i {
            1 => 20.0,
            2 => 22.0,
            3 => 3.0,
            _ => 0.0,
        };
        let mut shape = div()
            .key(LAYERS[i])
            .absolute(r)
            .rounded(radius)
            .bg(layer_color(i))
            .hit_test(false);
        if i == current {
            shape = shape.border(2.0, theme.accent());
        }
        stage = stage.child(shape);
    }
    div()
        .w_full()
        .h_full()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(10.0)
        .bg(c.surface[0])
        .child(stage)
        .child(
            text(format!("Viewport · {}", LAYERS[current]))
                .font_size(12.0)
                .text_color(c.text_tertiary),
        )
}

fn inspector_panel(
    app: &App,
    state: DockModels,
    opacity: View<NumberField>,
    blend: Model<usize>,
) -> Element {
    let theme = app.theme();
    let c = theme.colors();
    let current = *state.layer.read(app);
    let visible = state.visible[current];
    let row = |el: Element| {
        div()
            .h(36.0)
            .px(12.0)
            .flex_row()
            .items_center()
            .gap(8.0)
            .child(el)
    };
    Panel::new("Inspector")
        .child(
            div().px(12.0).pt_row().child(
                text(LAYERS[current])
                    .font_size(15.0)
                    .font_weight(600.0)
                    .text_color(c.text_primary),
            ),
        )
        .child(row(Toggle::new(*visible.read(app))
            .label("Visible")
            .on_change(move |v, cx| set(visible, cx, v))
            .render(theme)))
        .child(row(Element::view(&opacity)))
        .child(row(SegmentedControl::new(
            ["Normal", "Add"],
            *blend.read(app),
        )
        .label("Blend")
        .segment_width(64.0)
        .on_change(move |i, cx| set(blend, cx, i))
        .render(theme)))
        .render(theme)
}

trait PadTop {
    fn pt_row(self) -> Self;
}

impl PadTop for Element {
    fn pt_row(self) -> Self {
        self.style(|s| {
            s.padding.top = length(10.0);
            s.padding.bottom = length(4.0);
        })
    }
}

fn timeline_panel(app: &App, time: View<Slider>, at: Model<f32>) -> Element {
    let theme = app.theme();
    let c = theme.colors();
    let t = *at.read(app);
    let mut keys = div()
        .key("keys")
        .h(10.0)
        .w(240.0)
        .rounded(5.0)
        .bg(c.surface[3]);
    for k in [0.1_f32, 0.35, 0.6, 0.85] {
        keys = keys.child(
            div()
                .absolute(Rect::new(k * 240.0 - 3.0, 2.0, 6.0, 6.0))
                .rounded(1.0)
                .bg(theme.accent())
                .transform(livspeil_ui_layout::Transform2D::rotate(
                    std::f32::consts::FRAC_PI_4,
                )),
        );
    }
    Panel::new("Timeline")
        .child(
            div()
                .px(12.0)
                .pt_row()
                .flex_row()
                .items_center()
                .gap(12.0)
                .child(text("Time").text_color(c.text_secondary))
                .child(Element::view(&time))
                .child(
                    text(format!("{t:.1} s"))
                        .font_feature(FontFeature::TNUM)
                        .text_color(c.text_secondary),
                ),
        )
        .child(
            div()
                .px(12.0)
                .pt_row()
                .flex_row()
                .items_center()
                .gap(12.0)
                .child(text("Keys").text_color(c.text_secondary))
                .child(keys),
        )
        .render(theme)
}

#[derive(Clone, Copy)]
struct DockModels {
    layer: Model<usize>,
    visible: [Model<bool>; 5],
    opacity: Model<f64>,
}

impl Containers {
    /// Create the container pages' state.
    pub fn new(app: &mut App, env: &Env) -> Containers {
        let long = app.new_view(
            ScrollView::new(env, |app, _| long_content(app))
                .name("long-scroll")
                .label("Library"),
        );
        let list = app.new_view(
            VirtualList::build(env, VIRTUAL_ROWS, VIRTUAL_ROW_HEIGHT, virtual_row)
                .name("virtual-list-scroll")
                .label("100,000 rows"),
        );

        let layer = app.new_model(3);
        let visible = [
            app.new_model(true),
            app.new_model(true),
            app.new_model(true),
            app.new_model(true),
            app.new_model(true),
        ];
        let opacity = app.new_model(1.0_f64);
        let blend = app.new_model(0);
        let time = app.new_model(2.4_f32);
        let opacity_field = app.new_view(
            NumberField::new(env, opacity, 0.0, 1.0, 0.01)
                .label("Opacity")
                .width(150.0)
                .name("number:Opacity"),
        );
        let time_slider =
            app.new_view(Slider::new(time, 0.0, 10.0, 0.1).label("Time").width(200.0));
        let models = DockModels {
            layer,
            visible,
            opacity,
        };
        let dock = app.new_view(
            DockView::new(move |app| viewport(app, models))
                .name("editor-dock")
                .center_min(160.0, 120.0)
                .panel(DockEdge::Left, 170.0, 120.0, move |app| {
                    layers_panel(app, layer, visible)
                })
                .panel(DockEdge::Right, 210.0, 170.0, move |app| {
                    inspector_panel(app, models, opacity_field, blend)
                })
                .panel(DockEdge::Bottom, 104.0, 60.0, move |app| {
                    timeline_panel(app, time_slider, time)
                }),
        );

        let last_command = app.new_model(String::new());
        let checks = MenuChecks {
            grid: app.new_model(true),
            snap: app.new_model(false),
            minimap: app.new_model(true),
        };
        let menubar = app.new_view(
            MenuBar::new(env)
                .name("gallery-menubar")
                .menu(
                    "Livspeil",
                    Menu::new().item(MenuItem::new("About Livspeil Gallery").on_select(
                        move |app| set(last_command, app, "Livspeil › About".to_owned()),
                    )),
                )
                .menu("File", file_menu(last_command))
                .menu("Edit", edit_menu(last_command))
                .menu("View", Menu::new()),
        );
        refresh_view_menu(app, last_command, checks, menubar);
        env.set_menubar(menubar);

        let sample = app.new_model(DemoGraph::sample());
        let stress = app.new_model(DemoGraph::grid(STRESS_NODES, STRESS_LINKS));
        let canvases = [
            app.new_view(
                NodeCanvas::new(env, sample)
                    .name("node-canvas")
                    .with_canvas(Canvas {
                        pan: Vec2::new(0.0, 28.0),
                        zoom: 0.75,
                        ..Canvas::default()
                    }),
            ),
            app.new_view(
                NodeCanvas::new(env, stress)
                    .name("node-canvas-stress")
                    .with_canvas(Canvas {
                        pan: Vec2::new(24.0, 24.0),
                        zoom: 0.32,
                        ..Canvas::default()
                    }),
            ),
        ];
        Containers {
            long,
            list,
            dock,
            layer,
            visible,
            opacity,
            blend,
            time,
            menubar,
            last_command,
            checks,
            canvases,
            graph: app.new_model(0),
        }
    }
}

/// A rounded, clipped frame that fills the rest of the page.
fn frame(theme: &Theme, key: &'static str, child: Element) -> Element {
    div()
        .key(key)
        .grow()
        .style(|s| s.min_size.height = length(0.0))
        .rounded(theme.tokens.radii.lg)
        .clip()
        // Inset the content by the border so it does not paint over it.
        .p(1.0)
        .border(1.0, theme.colors().text_primary.with_alpha(0.1))
        .child(child)
}

fn fill_column() -> Element {
    column().grow().style(|s| s.min_size.height = length(0.0))
}

/// Scrolling.
pub(crate) fn scroll(theme: &Theme, s: &Containers) -> Element {
    let pane = |title: &'static str, view: &View<ScrollView>, note: &'static str| {
        card(theme, title)
            .grow()
            .style(|st| {
                st.flex_basis = length(0.0);
                st.min_size.width = length(0.0);
                st.min_size.height = length(0.0);
            })
            .child(frame(theme, "frame", Element::view(view)))
            .child(caption(theme, note))
    };
    div()
        .flex_row()
        .gap(16.0)
        .grow()
        .style(|st| st.min_size.height = length(0.0))
        .child(pane(
            "Scroll view",
            &s.long,
            "Trackpads move 1:1 with rubber banding and momentum; wheels and keys glide.",
        ))
        .child(pane(
            "Virtual list · 100,000 rows",
            &s.list,
            "Only the rows in view (plus 4 each side) are built.",
        ))
}

/// Panels & dock.
pub(crate) fn dock(theme: &Theme, s: &Containers, view: View<Gallery>, cx: &App) -> Element {
    let dock = s.dock;
    let side = |label: &'static str, edge: DockEdge| {
        Toggle::new(!dock.read(cx).is_collapsed(edge))
            .label(label)
            .on_change(move |on, cx| {
                DockView::set_collapsed(dock, cx, edge, !on);
                view.update(cx, |_, vcx| vcx.notify());
            })
            .render(theme)
    };
    fill_column()
        .child(
            row()
                .gap(20.0)
                .child(side("Layers", DockEdge::Left))
                .child(side("Inspector", DockEdge::Right))
                .child(side("Timeline", DockEdge::Bottom))
                .child(div().grow())
                .child(caption(
                    theme,
                    "Drag a splitter to resize; double-click it to reset.",
                )),
        )
        .child(frame(theme, "dock-frame", Element::view(&dock)))
}

/// Menus.
pub(crate) fn menus(theme: &Theme, s: &Containers, env: &Env, cx: &App) -> Element {
    let c = theme.colors();
    let last = s.last_command.read(cx);
    let checks = s.checks;
    let on = |m: Model<bool>| if *m.read(cx) { "on" } else { "off" };
    let window = div()
        .key("window")
        .h(280.0)
        .flex_col()
        .rounded(theme.tokens.radii.md)
        .clip()
        .bg(c.surface[0])
        .border(1.0, c.text_primary.with_alpha(0.1))
        .child(Element::view(&s.menubar))
        .child(
            div()
                .p(16.0)
                .flex_col()
                .gap(8.0)
                .child(text(if last.is_empty() {
                    "No command chosen yet.".to_owned()
                } else {
                    format!("Last command: {last}")
                }))
                .child(caption(
                    theme,
                    format!(
                        "Grid {} · Snap {} · Minimap {}",
                        on(checks.grid),
                        on(checks.snap),
                        on(checks.minimap)
                    ),
                )),
        );
    let area = div()
        .key("area")
        .name("context-area")
        .h(280.0)
        .rounded(theme.tokens.radii.md)
        .bg(c.surface[0])
        .border(1.0, c.text_primary.with_alpha(0.1))
        .flex_col()
        .items_center()
        .justify_center()
        .gap(6.0)
        .focusable()
        .semantics(|s| s.role(Role::Group).label("Canvas"))
        .child(text("Right-click here").text_color(c.text_secondary))
        .child(caption(theme, "or focus it and press Shift+F10"));
    let area = ContextMenu::new(context_menu(cx, s.last_command, checks)).attach(env, area);
    column()
        .child(crate::pages::columns(
            card(theme, "Menu bar").child(window),
            card(theme, "Context menu").child(area),
        ))
        .child(caption(
            theme,
            "F10 focuses the menu bar, Alt+F/E/V open a menu; arrows move, Enter chooses, Escape closes.",
        ))
}

/// Node canvas.
pub(crate) fn node_canvas(theme: &Theme, s: &Containers, cx: &App) -> Element {
    let graph = s.graph;
    let which = (*graph.read(cx)).min(1);
    let canvas = s.canvases[which];
    let total = if which == 0 {
        "6 nodes, 5 links".to_owned()
    } else {
        format!("{STRESS_NODES} nodes, {STRESS_LINKS} links; only the cards in view are built")
    };
    fill_column()
        .child(
            row()
                .child(
                    SegmentedControl::new(["Sample", "Stress · 500"], which)
                        .label("Graph")
                        .segment_width(96.0)
                        .on_change(move |i, cx| set(graph, cx, i))
                        .render(theme),
                )
                .child(
                    Button::new("Fit")
                        .on_click(move |_, cx| NodeCanvas::fit(canvas, cx))
                        .render(theme),
                )
                .child(div().grow())
                .child(caption(theme, total)),
        )
        .child(frame(theme, "canvas-frame", Element::view(&canvas)))
        .child(caption(
            theme,
            "Space-drag or two fingers pan, Ctrl/Cmd+scroll zooms, drag from a port to connect.",
        ))
}
