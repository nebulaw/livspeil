// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The gallery pages: [`State`] holds their models and stateful widgets
//! (created once, so values survive page switches), [`render`] builds the
//! current page.

use livspeil_ui_core::{
    Animated, App, Element, FontFeature, Model, Role, View, ViewContext, div, text,
};
use livspeil_ui_layout::style::length;
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_theme::{Color, Density, Material, MotionClass, Scheme, SpringSpec, Theme, Tier};
use livspeil_ui_widgets::{
    Button, CheckState, Checkbox, Env, IconButton, Label, NumberField, RadioGroup,
    SegmentedControl, Select, Slider, TextField, Toggle, Tooltip,
};

use crate::containers::{self, Containers};
use crate::{Gallery, Page, edit_motion, edit_theme, tier_control};

/// Models and stateful widgets of every page.
pub struct State {
    /// Buttons: click count.
    pub clicks: Model<u32>,
    /// Toggles: Wi-Fi.
    pub wifi: Model<bool>,
    /// Toggles: Bluetooth.
    pub bluetooth: Model<bool>,
    /// Check boxes under the "All files" parent.
    pub files: [Model<bool>; 3],
    /// Radios: size.
    pub size: Model<Option<usize>>,
    /// Radios: alignment.
    pub align: Model<Option<usize>>,
    /// Segmented: range.
    pub range: Model<usize>,
    /// Segmented: list or grid.
    pub layout: Model<usize>,
    /// Sliders.
    pub volume: View<Slider>,
    /// Sliders.
    pub brightness: View<Slider>,
    /// A disabled slider.
    pub locked: View<Slider>,
    /// Number fields: preview position and size.
    pub x: Model<f64>,
    /// Number fields: preview position and size.
    pub y: Model<f64>,
    /// Number fields: preview position and size.
    pub side: Model<f64>,
    /// Number field views.
    pub numbers: [View<NumberField>; 3],
    /// Text fields: name, search, Georgian, Arabic, disabled.
    pub fields: [View<TextField>; 5],
    /// The last submitted text.
    pub submitted: Model<String>,
    /// Selects: easing, font (placeholder), disabled.
    pub selects: [View<Select>; 3],
    /// Motion: spring response (seconds).
    pub response: Model<f32>,
    /// Motion: damping ratio.
    pub damping: Model<f32>,
    /// Motion: global speed (applied to the app's motion config).
    pub speed: Model<f32>,
    /// Motion sliders: response, damping, speed.
    pub motion_sliders: [View<Slider>; 3],
    /// Motion: the playground box position (0..1).
    pub ball: Model<Animated<f32>>,
    /// Motion: transition demo boxes on the right.
    pub flipped: Model<bool>,
    /// Theme page: density sample values.
    pub visible: Model<bool>,
    /// Theme page: density sample values.
    pub locked_layer: Model<bool>,
    /// Theme page: density sample values.
    pub blend: Model<usize>,
    /// The container pages (scroll, dock, menus, node canvas).
    pub containers: Containers,
    /// The Overlays & tabs page.
    pub overlays: crate::overlays::Overlays,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State").finish_non_exhaustive()
    }
}

fn slider(app: &mut App, value: f32, min: f32, max: f32, step: f32) -> (Model<f32>, Slider) {
    let m = app.new_model(value);
    (m, Slider::new(m, min, max, step))
}

impl State {
    /// Create every page's state.
    pub fn new(app: &mut App, env: &Env) -> State {
        let (_, volume) = slider(app, 60.0, 0.0, 100.0, 1.0);
        let volume = app.new_view(volume.label("Volume").width(240.0).show_value());
        let (_, bright) = slider(app, 0.75, 0.0, 1.0, 0.05);
        let brightness = app.new_view(bright.label("Brightness").width(240.0).show_value());
        let (_, locked) = slider(app, 30.0, 0.0, 100.0, 1.0);
        let locked = app.new_view(locked.label("Locked").width(240.0).disabled(true));

        let x = app.new_model(48.0_f64);
        let y = app.new_model(32.0_f64);
        let side = app.new_model(56.0_f64);
        let nf = |app: &mut App, m, max, label: &str| {
            app.new_view(
                NumberField::new(env, m, 0.0, max, 1.0)
                    .label(label)
                    .width(120.0)
                    .name(format!("number:{label}")),
            )
        };
        let numbers = [
            nf(app, x, 200.0, "X"),
            nf(app, y, 120.0, "Y"),
            nf(app, side, 100.0, "Size"),
        ];

        let submitted = app.new_model(String::new());
        let tf = |app: &mut App, label: &str, value: &str, placeholder: &str, disabled: bool| {
            app.new_view(
                TextField::new(env, value)
                    .label(label)
                    .placeholder(placeholder)
                    .width(280.0)
                    .disabled(disabled)
                    .name(format!("text:{label}"))
                    .on_submit(move |t, app| {
                        let t = t.to_owned();
                        submitted.update(app, |s, cx| {
                            *s = t;
                            cx.notify();
                        });
                    }),
            )
        };
        let fields = [
            tf(app, "Name", "", "Your name", false),
            tf(app, "Search", "", "Search charms", false),
            tf(app, "Georgian", "გამარჯობა, სამყარო", "", false),
            tf(app, "Arabic", "مرحبا بالعالم", "", false),
            tf(app, "Disabled", "Read only", "", true),
        ];

        let easing = app.new_model(Some(2));
        let font = app.new_model(None);
        let off = app.new_model(Some(0));
        let selects = [
            app.new_view(
                Select::new(env, easing, ["Linear", "Ease out", "Spring"])
                    .label("Easing")
                    .width(180.0)
                    .name("select:Easing"),
            ),
            app.new_view(
                Select::new(env, font, ["Inter", "Inter Display", "JetBrains Mono"])
                    .label("Font")
                    .placeholder("Choose a font…")
                    .width(180.0)
                    .name("select:Font"),
            ),
            app.new_view(
                Select::new(env, off, ["Unavailable"])
                    .label("Disabled")
                    .width(180.0)
                    .disabled(true)
                    .name("select:Disabled"),
            ),
        ];

        let (response, r) = slider(app, 0.35, 0.05, 1.0, 0.01);
        let (damping, d) = slider(app, 0.5, 0.1, 1.0, 0.01);
        let speed_now = app.motion().speed;
        let (speed, s) = slider(app, speed_now, 0.0, 2.0, 0.1);
        let motion_sliders = [
            app.new_view(r.label("Response").width(150.0).show_value()),
            app.new_view(d.label("Damping").width(150.0).show_value()),
            app.new_view(s.label("Speed").width(150.0).show_value()),
        ];
        // The speed slider drives the app's motion configuration.
        let _ = app.observe(speed.id(), move |app| {
            let v = *speed.read(app);
            if (app.motion().speed - v).abs() > f32::EPSILON {
                let mut m = app.motion().clone();
                m.speed = v;
                app.set_motion(m);
            }
        });

        State {
            clicks: app.new_model(0),
            wifi: app.new_model(true),
            bluetooth: app.new_model(false),
            files: [
                app.new_model(true),
                app.new_model(false),
                app.new_model(true),
            ],
            size: app.new_model(Some(1)),
            align: app.new_model(Some(0)),
            range: app.new_model(1),
            layout: app.new_model(0),
            volume,
            brightness,
            locked,
            x,
            y,
            side,
            numbers,
            fields,
            submitted,
            selects,
            response,
            damping,
            speed,
            motion_sliders,
            ball: app.new_model(Animated::new(0.0)),
            flipped: app.new_model(false),
            visible: app.new_model(true),
            locked_layer: app.new_model(false),
            blend: app.new_model(0),
            containers: Containers::new(app, env),
            overlays: crate::overlays::Overlays::new(app, env),
        }
    }
}

pub(crate) fn set<T: PartialEq + 'static>(m: Model<T>, app: &mut App, v: T) {
    if *m.read(app) != v {
        m.update(app, |x, cx| {
            *x = v;
            cx.notify();
        });
    }
}

// ---- building blocks -------------------------------------------------

pub(crate) fn caption(theme: &Theme, s: impl Into<String>) -> Element {
    text(s.into())
        .font_size(theme.tokens.type_scale.sizes[1])
        .text_color(theme.colors().text_secondary)
}

/// A Solid card with a small heading.
pub(crate) fn card(theme: &Theme, title: &'static str) -> Element {
    div()
        .flex_col()
        .gap(12.0)
        .p(16.0)
        .rounded(theme.tokens.radii.md)
        .material(Material::Solid)
        .semantics(|s| s.role(Role::Group).label(title))
        .child(
            text(title.to_uppercase())
                .font_size(11.0)
                .font_weight(600.0)
                .text_color(theme.colors().text_tertiary),
        )
}

pub(crate) fn row() -> Element {
    div().flex_row().items_center().gap(12.0)
}

/// Two equal columns.
pub(crate) fn columns(left: Element, right: Element) -> Element {
    let col = |e: Element| {
        e.grow().style(|s| {
            s.flex_basis = length(0.0);
            s.min_size.width = length(0.0);
        })
    };
    div()
        .flex_row()
        .gap(16.0)
        .child(col(left))
        .child(col(right))
}

pub(crate) fn column() -> Element {
    div().flex_col().gap(16.0)
}

/// A label column of fixed width then a control.
fn labeled(theme: &Theme, label: &str, control: Element) -> Element {
    row()
        .child(div().w(84.0).child(caption(theme, label.to_owned())))
        .child(control)
}

// ---- pages -----------------------------------------------------------

/// The content of `page`.
pub(crate) fn render(
    page: Page,
    state: &State,
    env: &Env,
    view: View<Gallery>,
    cx: &mut ViewContext<'_, Gallery>,
) -> Element {
    let theme = cx.theme().clone();
    match page {
        Page::Buttons => buttons(&theme, state, env, cx),
        Page::Toggles => toggles(&theme, state, cx),
        Page::Radios => radios(&theme, state, cx),
        Page::Sliders => sliders(&theme, state, cx),
        Page::TextFields => text_fields(&theme, state, cx),
        Page::Select => selects(&theme, state, env),
        Page::Scroll => containers::scroll(&theme, &state.containers),
        Page::Dock => containers::dock(&theme, &state.containers, view, cx),
        Page::Menus => containers::menus(&theme, &state.containers, env, cx),
        Page::NodeCanvas => containers::node_canvas(&theme, &state.containers, cx),
        Page::Materials => materials(&theme),
        Page::Typography => typography(&theme),
        Page::Motion => motion(&theme, state, view, cx),
        Page::Theme => theme_page(&theme, state, cx),
        Page::Overlays => crate::overlays::page(&theme, &state.overlays, cx),
    }
}

fn buttons(theme: &Theme, state: &State, env: &Env, cx: &App) -> Element {
    let clicks = state.clicks;
    let n = *clicks.read(cx);
    let variants = |disabled: bool| {
        row()
            .child(
                Button::new("Save")
                    .primary()
                    .disabled(disabled)
                    .render(theme),
            )
            .child(Button::new("Cancel").disabled(disabled).render(theme))
            .child(Button::new("More").ghost().disabled(disabled).render(theme))
            .child(
                Button::new("Delete")
                    .destructive()
                    .disabled(disabled)
                    .render(theme),
            )
    };
    let icon = |icon: &'static str, label: &'static str| {
        Tooltip::new(label).attach(env, IconButton::new(icon, label).render(theme))
    };
    column()
        .child(
            card(theme, "Variants")
                .child(variants(false))
                .child(caption(
                    theme,
                    "Primary, secondary, ghost and destructive. Tab moves focus; Space or Enter presses.",
                )),
        )
        .child(card(theme, "Disabled").child(variants(true)))
        .child(
            columns(
                card(theme, "Icon buttons").child(
                    row()
                        .child(icon("bx-bookmarks", "Bookmarks"))
                        .child(icon("bx-copy", "Duplicate"))
                        .child(icon("bx-folder", "Open folder"))
                        .child(
                            IconButton::new("bx-unlink", "Unlink")
                                .disabled(true)
                                .render(theme),
                        ),
                ),
                card(theme, "Counter").child(
                    row()
                        .child(
                            Button::new("Click me")
                                .primary()
                                .on_click(move |_, cx| {
                                    clicks.update(cx, |c, cx| {
                                        *c += 1;
                                        cx.notify();
                                    });
                                })
                                .render(theme),
                        )
                        .child(
                            Label::new(match n {
                                0 => "Not clicked yet".to_owned(),
                                1 => "Clicked once".to_owned(),
                                n => format!("Clicked {n} times"),
                            })
                            .render(theme)
                            .font_feature(FontFeature::TNUM),
                        ),
                ),
            ),
        )
}

fn switch(theme: &Theme, m: Model<bool>, label: &'static str, cx: &App) -> Element {
    Toggle::new(*m.read(cx))
        .label(label)
        .on_change(move |v, cx| set(m, cx, v))
        .render(theme)
}

fn toggles(theme: &Theme, state: &State, cx: &App) -> Element {
    let files = state.files;
    let on: Vec<bool> = files.iter().map(|m| *m.read(cx)).collect();
    let all = match on.iter().filter(|b| **b).count() {
        0 => CheckState::Unchecked,
        3 => CheckState::Checked,
        _ => CheckState::Mixed,
    };
    let names = ["Photos", "Music", "Documents"];
    let mut checks = column().gap(8.0).child(
        Checkbox::new(all)
            .label("All files")
            .on_change(move |_, cx| {
                let v = all != CheckState::Checked;
                for m in files {
                    set(m, cx, v);
                }
            })
            .render(theme),
    );
    for (i, name) in names.into_iter().enumerate() {
        let m = files[i];
        checks = checks.child(
            div().style(|s| s.padding.left = length(24.0)).child(
                Checkbox::new(CheckState::from_bool(on[i]))
                    .label(name)
                    .on_change(move |s, cx| set(m, cx, s == CheckState::Checked))
                    .render(theme),
            ),
        );
    }
    columns(
        card(theme, "Switches")
            .child(switch(theme, state.wifi, "Wi-Fi", cx))
            .child(switch(theme, state.bluetooth, "Bluetooth", cx))
            .child(
                Toggle::new(true)
                    .label("Always on")
                    .disabled(true)
                    .render(theme),
            )
            .child(
                Toggle::new(false)
                    .label("Unavailable")
                    .disabled(true)
                    .render(theme),
            ),
        column()
            .child(
                card(theme, "Check boxes")
                    .child(checks)
                    .child(caption(theme, "The parent shows a mixed state.")),
            )
            .child(
                card(theme, "Disabled").child(
                    Checkbox::new(CheckState::Checked)
                        .label("Required")
                        .disabled(true)
                        .render(theme),
                ),
            ),
    )
}

fn radios(theme: &Theme, state: &State, cx: &App) -> Element {
    let (size, align, range, layout) = (state.size, state.align, state.range, state.layout);
    let sizes = ["Small", "Medium", "Large"];
    let ranges = ["Day", "Week", "Month", "Year"];
    let size_now = *size.read(cx);
    let range_now = *range.read(cx);
    columns(
        column()
            .child(
                card(theme, "Radio group").child(
                    RadioGroup::new(sizes, size_now)
                        .label("Size")
                        .on_change(move |i, cx| set(size, cx, Some(i)))
                        .render(theme),
                ),
            )
            .child(
                card(theme, "Horizontal").child(
                    RadioGroup::new(["Left", "Center", "Right"], *align.read(cx))
                        .label("Alignment")
                        .horizontal()
                        .disable_option(2)
                        .on_change(move |i, cx| set(align, cx, Some(i)))
                        .render(theme),
                ),
            ),
        card(theme, "Segmented controls")
            .child(
                SegmentedControl::new(ranges, range_now)
                    .label("Range")
                    .on_change(move |i, cx| set(range, cx, i))
                    .render(theme),
            )
            .child(
                SegmentedControl::new(["List", "Grid"], *layout.read(cx))
                    .label("Layout")
                    .on_change(move |i, cx| set(layout, cx, i))
                    .render(theme),
            )
            .child(
                SegmentedControl::new(["On", "Off"], 0)
                    .label("Disabled")
                    .disabled(true)
                    .render(theme),
            )
            .child(caption(
                theme,
                format!(
                    "Size: {} · Range: {}",
                    size_now.and_then(|i| sizes.get(i)).unwrap_or(&"none"),
                    ranges.get(range_now).unwrap_or(&"none")
                ),
            )),
    )
}

fn sliders(theme: &Theme, state: &State, cx: &App) -> Element {
    let c = theme.colors();
    // Narrowing for display only: the fields stay within the preview.
    #[allow(clippy::cast_possible_truncation)]
    let (x, y, side) = (
        *state.x.read(cx) as f32,
        *state.y.read(cx) as f32,
        *state.side.read(cx) as f32,
    );
    let mut fields = column().gap(10.0);
    for n in &state.numbers {
        fields = fields.child(Element::view(n));
    }
    column()
        .child(
            card(theme, "Sliders")
                .child(labeled(theme, "Volume", Element::view(&state.volume)))
                .child(labeled(
                    theme,
                    "Brightness",
                    Element::view(&state.brightness),
                ))
                .child(labeled(theme, "Disabled", Element::view(&state.locked)))
                .child(caption(
                    theme,
                    "Drag, click the track, or use the arrow keys (Shift for large steps).",
                )),
        )
        .child(
            card(theme, "Number fields: drag to scrub")
                .child(
                    row().gap(24.0).child(fields).child(
                        div()
                            .key("preview")
                            .name("preview")
                            .size(300.0, 176.0)
                            .rounded(theme.tokens.radii.sm)
                            .bg(c.surface[0])
                            .clip()
                            .child(
                                div()
                                    .key("box")
                                    .absolute(Rect::new(x, y, side, side))
                                    .rounded(theme.tokens.radii.sm)
                                    .bg(theme.accent()),
                            ),
                    ),
                )
                .child(caption(
                    theme,
                    "Drag a field sideways to scrub: Shift ×10, Alt ×0.1. Click to type a value.",
                )),
        )
}

fn text_fields(theme: &Theme, state: &State, cx: &App) -> Element {
    let labels = ["Name", "Search", "Georgian", "Arabic (RTL)", "Disabled"];
    let mut card = card(theme, "Text fields");
    for (label, f) in labels.into_iter().zip(&state.fields) {
        card = card.child(labeled(theme, label, Element::view(f)));
    }
    let last = state.submitted.read(cx);
    column()
        .child(card)
        .child(crate::pages::card(theme, "Submitted").child(caption(
            theme,
            if last.is_empty() {
                "Press Enter in a field to submit it.".to_owned()
            } else {
                format!("Submitted: {last}")
            },
        )))
}

fn selects(theme: &Theme, state: &State, env: &Env) -> Element {
    let mut list = card(theme, "Selects");
    for (label, s) in ["Easing", "Font", "Disabled"]
        .into_iter()
        .zip(&state.selects)
    {
        list = list.child(labeled(theme, label, Element::view(s)));
    }
    let tip = |label: &'static str, tip: &'static str| {
        Tooltip::new(tip).attach(env, Button::new(label).render(theme))
    };
    columns(
        list.child(caption(
            theme,
            "Click or press Space to open; arrows and type-ahead choose.",
        )),
        card(theme, "Tooltips")
            .child(
                row()
                    .child(tip("Hover me", "Tooltips appear after 500 ms"))
                    .child(tip("Another", "And hide when you leave")),
            )
            .child(row().child(
                Tooltip::new("Icon buttons are named by their tooltip").attach(
                    env,
                    IconButton::new("bx-message-circle-notification", "Information").render(theme),
                ),
            ))
            .child(caption(theme, "Rest the pointer on a control.")),
    )
}

fn materials(theme: &Theme) -> Element {
    let c = theme.colors();
    let area_h = 300.0;
    let stripe = |i: usize, color: Color| {
        #[allow(clippy::cast_precision_loss)]
        let x = i as f32 * 120.0 - 40.0;
        div()
            .absolute(Rect::new(x, -40.0, 70.0, 400.0))
            .bg(color)
            .transform(livspeil_ui_layout::Transform2D::rotate(0.35))
            .hit_test(false)
    };
    let blob = |r: Rect, color: Color| {
        div()
            .absolute(r)
            .rounded(r.width / 2.0)
            .bg(color)
            .hit_test(false)
    };
    let sample = |m: Material, name: &'static str, note: &'static str, x: f32, y: f32| {
        div()
            .key(name)
            .absolute(Rect::new(x, y, 144.0, 104.0))
            .material(m)
            .rounded(theme.tokens.radii.lg)
            .flex_col()
            .p(14.0)
            .gap(6.0)
            .semantics(|s| s.role(Role::Group).label(name))
            .child(text(name).font_size(17.0).font_weight(600.0))
            .child(
                text(note)
                    .font_size(12.0)
                    .text_color(c.text_secondary)
                    .w(116.0),
            )
    };
    let tier = match (theme.tier, theme.reduce_transparency) {
        (_, true) => "Reduce transparency: opaque tints.",
        (Tier::T0, _) => "T0: no blur, opaque tints.",
        (Tier::T1, _) => "T1: reduced blur, no saturation boost.",
        (Tier::T2, _) => "T2: the full recipe.",
    };
    column()
        .child(
            div()
                .key("stage")
                .name("material-stage")
                .w_full()
                .h(area_h)
                .rounded(theme.tokens.radii.lg)
                .clip()
                .bg(Color::oklch(0.32, 0.08, 270.0, 1.0))
                .child(stripe(0, Color::oklch(0.65, 0.2, 25.0, 1.0)))
                .child(stripe(1, Color::oklch(0.78, 0.16, 85.0, 1.0)))
                .child(stripe(2, Color::oklch(0.7, 0.17, 150.0, 1.0)))
                .child(stripe(3, Color::oklch(0.66, 0.15, 230.0, 1.0)))
                .child(stripe(4, Color::oklch(0.6, 0.2, 300.0, 1.0)))
                .child(stripe(5, Color::oklch(0.7, 0.2, 350.0, 1.0)))
                .child(stripe(6, Color::oklch(0.8, 0.15, 110.0, 1.0)))
                .child(blob(
                    Rect::new(260.0, 150.0, 180.0, 180.0),
                    Color::oklch(0.9, 0.1, 95.0, 1.0),
                ))
                .child(sample(
                    Material::Solid,
                    "Solid",
                    "Dense tools and inspectors",
                    20.0,
                    40.0,
                ))
                .child(sample(
                    Material::Frost,
                    "Frost",
                    "Sidebars, docks, the menubar",
                    190.0,
                    110.0,
                ))
                .child(sample(
                    Material::Glass,
                    "Glass",
                    "Tooltips and menus",
                    360.0,
                    40.0,
                ))
                .child(sample(
                    Material::Ink,
                    "Ink",
                    "Text over the scene",
                    530.0,
                    110.0,
                )),
        )
        .child(
            card(theme, "Quality")
                .child(
                    row().child(tier_control(theme)).child(
                        Toggle::new(theme.reduce_transparency)
                            .label("Reduce transparency")
                            .on_change(|v, cx| edit_theme(cx, |t| t.reduce_transparency = v))
                            .render(theme),
                    ),
                )
                .child(caption(theme, tier))
                .children(crate::blur_note().map(|why| {
                    caption(
                        theme,
                        format!(
                            "Blur is off on this GPU: Frost and Glass show plain colours ({why}). Run with --gpu-info for details."
                        ),
                    )
                    .name("blur-fallback-note")
                })),
        )
}

fn typography(theme: &Theme) -> Element {
    let c = theme.colors();
    let names = [
        "Caption",
        "Footnote",
        "Body",
        "Body large",
        "Title 3",
        "Title 2",
        "Title 1",
    ];
    let mut scale = card(theme, "Type scale (Inter)").gap(6.0);
    for (size, name) in theme.tokens.type_scale.sizes.iter().zip(names) {
        scale = scale.child(
            row()
                .child(
                    div().w(84.0).style(|s| s.flex_shrink = 0.0).child(
                        text(format!("{size} {name}"))
                            .font_size(11.0)
                            .text_color(c.text_tertiary),
                    ),
                )
                .child(text("Quick brown fox").font_size(*size)),
        );
    }
    let mut weights = card(theme, "Weights").gap(6.0);
    for (w, name) in [
        (400.0, "Regular 400"),
        (500.0, "Medium 500"),
        (600.0, "Semibold 600"),
    ] {
        weights = weights.child(text(name).font_size(17.0).font_weight(w));
    }
    let digits = |tnum: bool| {
        let mut col = div().flex_col().gap(2.0);
        for d in ["1111.11", "8888.88", "1234.50"] {
            let t = text(d).font_size(15.0);
            col = col.child(if tnum {
                t.font_feature(FontFeature::TNUM)
            } else {
                t
            });
        }
        div()
            .flex_col()
            .gap(4.0)
            .child(caption(theme, if tnum { "tnum" } else { "proportional" }))
            .child(col)
    };
    columns(
        scale,
        column()
            .child(weights)
            .child(
                card(theme, "Digits")
                    .child(row().gap(32.0).child(digits(false)).child(digits(true))),
            )
            .child(
                card(theme, "Scripts")
                    .child(text("ქართული ენა · გამარჯობა").font_size(15.0))
                    .child(text("Livspeil: مرحبا بالعالم 2026").font_size(15.0)),
            ),
    )
}

fn motion(
    theme: &Theme,
    state: &State,
    view: View<Gallery>,
    cx: &mut ViewContext<'_, Gallery>,
) -> Element {
    let c = theme.colors();
    // Follow hot-reloaded speed changes.
    let speed_now = cx.motion().speed;
    if (*state.speed.read(cx) - speed_now).abs() > f32::EPSILON {
        set(state.speed, cx, speed_now);
    }
    // Step the playground spring (render-driven animation).
    let dt = cx.frame_delta();
    let ball = state.ball;
    let moving = ball.update(cx, |a, _| a.is_animating() && a.step(dt));
    if moving {
        cx.notify();
    }
    let pos = ball.read(cx).value();
    let flipped = *state.flipped.read(cx);
    let (response, damping) = (state.response, state.damping);
    let flip = state.flipped;
    let track_w = 280.0;
    let knob = 36.0;
    let animate = Button::new("Animate")
        .primary()
        .on_click(move |_, cx| {
            let m = cx.motion().clone();
            let target = if ball.read(cx).target() > 0.5 {
                0.0
            } else {
                1.0
            };
            let spring = SpringSpec {
                response: *response.read(cx) / m.speed.max(0.01),
                damping: *damping.read(cx),
            };
            let instant = m.speed <= 0.0 || m.reduce_motion || m.profile == "off";
            ball.update(cx, |a, cx| {
                if instant {
                    a.set(target);
                } else {
                    a.spring_to(target, spring);
                }
                cx.notify();
            });
            flip.update(cx, |f, cx| {
                *f = !*f;
                cx.notify();
            });
        })
        .render(theme);
    let profiles = ["snappy", "fluid", "calm", "off"];
    let profile = profiles
        .iter()
        .position(|p| *p == cx.motion().profile)
        .unwrap_or(0);
    let lane = |key: &'static str, class: MotionClass| {
        row().child(div().w(72.0).child(caption(theme, key))).child(
            div().w(180.0).h(20.0).rounded(10.0).bg(c.surface[0]).child(
                div()
                    .key(key)
                    .absolute(Rect::new(2.0, 2.0, 16.0, 16.0))
                    .rounded(8.0)
                    .bg(theme.accent())
                    .translate(Vec2::new(if flipped { 160.0 } else { 0.0 }, 0.0))
                    .transition(class),
            ),
        )
    };
    columns(
        card(theme, "Spring playground")
            .child(labeled(theme, "Response", Element::view(&state.motion_sliders[0])))
            .child(labeled(theme, "Damping", Element::view(&state.motion_sliders[1])))
            .child(
                div()
                    .key("track")
                    .name("spring-track")
                    .w(track_w)
                    .h(knob + 8.0)
                    .rounded(theme.tokens.radii.sm)
                    .bg(c.surface[0])
                    .child(
                        div()
                            .key("knob")
                            .absolute(Rect::new(
                                4.0 + pos * (track_w - knob - 8.0),
                                4.0,
                                knob,
                                knob,
                            ))
                            .rounded(theme.tokens.radii.sm)
                            .bg(theme.accent()),
                    ),
            )
            .child(row().child(animate))
            .child(caption(
                theme,
                "Low damping overshoots; a longer response is slower. Speed and Reduce motion apply too.",
            )),
        column()
            .child(
                card(theme, "Motion settings")
                    .child(
                        SegmentedControl::new(["Snappy", "Fluid", "Calm", "Off"], profile)
                            .label("Profile")
                            .segment_width(64.0)
                            .on_change(move |i, cx| {
                                let name = profiles.get(i).copied().unwrap_or("snappy");
                                edit_motion(cx, view, |m| m.profile = name.to_owned());
                            })
                            .render(theme),
                    )
                    .child(labeled(theme, "Speed", Element::view(&state.motion_sliders[2])))
                    .child(
                        Toggle::new(cx.motion().reduce_motion)
                            .label("Reduce motion")
                            .on_change(move |v, cx| edit_motion(cx, view, |m| m.reduce_motion = v))
                            .render(theme),
                    ),
            )
            .child(
                card(theme, "Profile springs")
                    .child(lane("Micro", MotionClass::Micro))
                    .child(lane("Standard", MotionClass::Standard))
                    .child(lane("Large", MotionClass::Large)),
            ),
    )
}

fn theme_page(theme: &Theme, state: &State, cx: &App) -> Element {
    let c = theme.colors();
    let blend = state.blend;
    let swatch = |name: &'static str, color: Color| {
        div()
            .flex_col()
            .gap(4.0)
            .w(64.0)
            .child(
                div()
                    .h(40.0)
                    .rounded(theme.tokens.radii.sm)
                    .bg(color)
                    .border(1.0, c.text_primary.with_alpha(0.12)),
            )
            .child(text(name).font_size(11.0).text_color(c.text_secondary))
    };
    let swatches = [
        ("surface.0", c.surface[0]),
        ("surface.1", c.surface[1]),
        ("surface.2", c.surface[2]),
        ("surface.3", c.surface[3]),
        ("text", c.text_primary),
        ("secondary", c.text_secondary),
        ("tertiary", c.text_tertiary),
        ("accent", theme.accent()),
        ("danger", c.danger),
        ("warning", c.warning),
        ("success", c.success),
        ("focus ring", c.focus_ring),
    ];
    let mut grid = card(theme, "Color roles");
    for chunk in swatches.chunks(4) {
        let mut r = row().gap(8.0);
        for (n, col) in chunk {
            r = r.child(swatch(n, *col));
        }
        grid = grid.child(r);
    }
    let m = theme.metrics();
    let scheme = usize::from(theme.scheme == Scheme::Light);
    let density = usize::from(theme.density == Density::Pro);
    let inspector_row = |control: Element| row().h(m.row_height).child(control);
    columns(
        grid,
        column()
            .child(
                card(theme, "Appearance")
                    .child(
                        SegmentedControl::new(["Dark", "Light"], scheme)
                            .label("Scheme")
                            .on_change(|i, cx| {
                                edit_theme(cx, |t| {
                                    t.scheme = if i == 0 { Scheme::Dark } else { Scheme::Light };
                                });
                            })
                            .render(theme),
                    )
                    .child(
                        SegmentedControl::new(["Simple", "Pro"], density)
                            .label("Density switch")
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
                    ),
            )
            .child(
                card(theme, "Density sample")
                    .gap(0.0)
                    .child(inspector_row(switch(theme, state.visible, "Visible", cx)))
                    .child(inspector_row(switch(
                        theme,
                        state.locked_layer,
                        "Locked",
                        cx,
                    )))
                    .child(inspector_row(
                        SegmentedControl::new(["Normal", "Add"], *state.blend.read(cx))
                            .label("Blend")
                            .on_change(move |i, cx| set(blend, cx, i))
                            .render(theme),
                    ))
                    .child(caption(
                        theme,
                        format!("Rows {} px, body {} px", m.row_height, m.body),
                    )),
            ),
    )
}
