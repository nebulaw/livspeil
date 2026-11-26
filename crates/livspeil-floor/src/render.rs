// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Option A renderer: Smithay's GLES renderer plus custom shaders.
//!
//! Scene order per frame: background, then per window (bottom to top) the
//! soft shadow, the rounded titlebar/body, the client surface clipped by a
//! rounded mask (custom texture shader through
//! `GlesFrame::override_default_tex_program`), and finally one "Frost"
//! panel whose backdrop is blurred with dual Kawase on a cropped region.
//!
//! The blur needs the scene as a texture, so with blur on the scene is
//! rendered to an offscreen texture first and copied to the window; with
//! blur off the scene is drawn straight to the window. There is no damage
//! tracking: every frame redraws everything (see the decision doc).

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, Frame, ImportAll, Offscreen, Renderer, Texture,
            element::{
                Element, Kind, RenderElement,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            },
            gles::{
                GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTarget, GlesTexProgram,
                GlesTexture, Uniform, UniformName, UniformType,
            },
            sync::SyncPoint,
            utils::with_renderer_surface_state,
        },
    },
    desktop::{PopupManager, WindowSurfaceType},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Buffer, Physical, Rectangle, Size, Transform},
};

use crate::{
    geometry::{
        BUTTON_SIZE, CONTACT_ALPHA, CONTACT_BLUR, CONTACT_OFFSET_Y, CORNER_RADIUS, FrameLayout,
        Hit, Rect, SHADOW_ALPHA, SHADOW_BLUR, SHADOW_OFFSET_Y, kawase_offset, neutral_srgb,
    },
    shaders,
    state::{FloorState, FloorWindow},
};

/// Backdrop blur radius in logical pixels (Frost: 24, UI spec section 5).
pub const BLUR_RADIUS: f32 = 24.0;

/// Tint opacity of the demo panel over its backdrop.
pub const PANEL_TINT: f32 = 0.40;

/// A Frost panel, in output pixels. Width and height must be multiples of
/// `2^levels` (4 for the default radius) so every level has whole pixels.
#[derive(Debug, Clone, Copy)]
pub struct Panel {
    pub rect: Rect,
}

/// Which effects to draw; used by the benchmark to price each of them.
#[derive(Debug, Clone, Copy)]
pub struct Effects {
    pub shadows: bool,
    pub rounded: bool,
    pub blur: bool,
    /// Draw a software pointer (the TTY build has no host cursor).
    pub cursor: bool,
}

impl Default for Effects {
    fn default() -> Self {
        Effects {
            shadows: true,
            rounded: true,
            blur: true,
            cursor: false,
        }
    }
}

/// Render errors.
#[derive(Debug)]
pub enum RenderError {
    Gles(GlesError),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::Gles(e) => write!(f, "gles: {e}"),
        }
    }
}
impl std::error::Error for RenderError {}
impl From<GlesError> for RenderError {
    fn from(e: GlesError) -> Self {
        RenderError::Gles(e)
    }
}

struct Programs {
    shadow: GlesPixelProgram,
    rounded_box: GlesPixelProgram,
    background: GlesPixelProgram,
    tint_panel: GlesPixelProgram,
    clip_surface: GlesTexProgram,
    kawase_down: GlesTexProgram,
    kawase_up: GlesTexProgram,
    frost: GlesTexProgram,
}

struct BlurChain {
    /// Panel rect and output size the textures were made for.
    key: (Rect, (i32, i32)),
    half: GlesTexture,
    quarter: GlesTexture,
    half_up: GlesTexture,
    out: GlesTexture,
}

/// Renderer state (compiled programs and cached offscreen textures).
pub struct FloorRenderer {
    programs: Programs,
    scene: Option<(GlesTexture, (i32, i32))>,
    chain: Option<BlurChain>,
}

impl std::fmt::Debug for FloorRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FloorRenderer").finish_non_exhaustive()
    }
}

fn u(name: &'static str, ty: UniformType) -> UniformName<'static> {
    UniformName::new(name, ty)
}

fn px_rect(r: Rect) -> Rectangle<i32, Physical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

fn full_damage(r: Rectangle<i32, Physical>) -> [Rectangle<i32, Physical>; 1] {
    [Rectangle::from_size(r.size)]
}

fn grey(l: f32) -> [f32; 3] {
    let v = f32::from(neutral_srgb(l)) / 255.0;
    [v, v, v]
}

fn grey_a(l: f32, a: f32) -> [f32; 4] {
    let g = grey(l);
    [g[0], g[1], g[2], a]
}

impl FloorRenderer {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, RenderError> {
        let programs = Programs {
            shadow: renderer.compile_custom_pixel_shader(
                shaders::shadow(),
                &[
                    u("box_rect", UniformType::_4f),
                    u("radius", UniformType::_1f),
                    u("key", UniformType::_4f),
                    u("contact", UniformType::_4f),
                ],
            )?,
            rounded_box: renderer.compile_custom_pixel_shader(
                shaders::rounded_box(),
                &[
                    u("radii", UniformType::_4f),
                    u("fill", UniformType::_4f),
                    u("hairline", UniformType::_4f),
                ],
            )?,
            background: renderer.compile_custom_pixel_shader(
                shaders::background(),
                &[u("top", UniformType::_3f), u("bottom", UniformType::_3f)],
            )?,
            tint_panel: renderer.compile_custom_pixel_shader(
                shaders::tint_panel(),
                &[
                    u("radius", UniformType::_1f),
                    u("tint_color", UniformType::_4f),
                ],
            )?,
            clip_surface: renderer.compile_custom_texture_shader(
                shaders::clip_surface(),
                &[
                    u("buf_size", UniformType::_2f),
                    u("geo", UniformType::_4f),
                    u("radii", UniformType::_4f),
                ],
            )?,
            kawase_down: renderer.compile_custom_texture_shader(
                shaders::kawase_down(),
                &[
                    u("halfpixel", UniformType::_2f),
                    u("offset", UniformType::_1f),
                ],
            )?,
            kawase_up: renderer.compile_custom_texture_shader(
                shaders::kawase_up(),
                &[
                    u("halfpixel", UniformType::_2f),
                    u("offset", UniformType::_1f),
                ],
            )?,
            frost: renderer.compile_custom_texture_shader(
                shaders::frost(),
                &[
                    u("size", UniformType::_2f),
                    u("radius", UniformType::_1f),
                    u("tint_color", UniformType::_4f),
                    u("saturation", UniformType::_1f),
                    u("rim", UniformType::_1f),
                ],
            )?,
        };
        Ok(FloorRenderer {
            programs,
            scene: None,
            chain: None,
        })
    }

    /// Draw one frame into `target` (a bound framebuffer of `size`).
    /// `target_transform` is `Flipped180` for the winit window and `Normal`
    /// for textures.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        renderer: &mut GlesRenderer,
        target: &mut GlesTarget<'_>,
        size: (i32, i32),
        target_transform: Transform,
        state: &FloorState,
        panel: Option<Panel>,
        fx: Effects,
    ) -> Result<SyncPoint, RenderError> {
        let out_size: Size<i32, Physical> = size.into();

        // Client surfaces become render elements before any frame exists
        // (importing buffers needs the renderer).
        let mut drawn: Vec<DrawnWindow> = Vec::new();
        for (i, w) in state.windows.iter().enumerate() {
            if !w.is_mapped() {
                continue;
            }
            drawn.push(collect_window(renderer, state, i, w));
        }

        let sync;
        let use_blur = fx.blur && panel.is_some();
        if use_blur {
            self.ensure_textures(renderer, size, panel.map(|p| p.rect))?;
            // 1. Scene into an offscreen texture.
            if let Some((mut scene, sz)) = self.scene.take() {
                let res = (|| -> Result<(), RenderError> {
                    let mut fb = renderer.bind(&mut scene)?;
                    let mut frame = renderer.render(&mut fb, out_size, Transform::Normal)?;
                    self.draw_scene(&mut frame, state, &drawn, size, fx)?;
                    let _ = frame.finish()?;
                    Ok(())
                })();
                self.scene = Some((scene, sz));
                res?;
            }
            // 2. Blur chain on the cropped panel region.
            if let Some(p) = panel {
                self.blur_panel(renderer, p.rect)?;
            }
            // 3. Scene to the window, then the frost panel on top.
            let mut frame = renderer.render(target, out_size, target_transform)?;
            if let Some((scene, _)) = self.scene.as_ref() {
                let dst = Rectangle::from_size(out_size);
                let src: Rectangle<f64, Buffer> = Rectangle::from_size(scene.size().to_f64());
                frame.render_texture_from_to(
                    scene,
                    src,
                    dst,
                    &full_damage(dst),
                    &[],
                    Transform::Normal,
                    1.0,
                    None,
                    &[],
                )?;
            }
            if let (Some(p), Some(chain)) = (panel, self.chain.as_ref()) {
                self.draw_frost(&mut frame, p.rect, &chain.out)?;
            }
            self.draw_cursor(&mut frame, state, fx)?;
            sync = frame.finish()?;
        } else {
            let mut frame = renderer.render(target, out_size, target_transform)?;
            self.draw_scene(&mut frame, state, &drawn, size, fx)?;
            if let Some(p) = panel {
                // Without blur the panel is a plain translucent tint.
                let dst = px_rect(p.rect);
                frame.render_pixel_shader_to(
                    &self.programs.tint_panel,
                    Rectangle::from_size(Size::<f64, Buffer>::from((
                        f64::from(p.rect.w),
                        f64::from(p.rect.h),
                    ))),
                    dst,
                    (p.rect.w, p.rect.h).into(),
                    None,
                    1.0,
                    &[
                        Uniform::new("radius", CORNER_RADIUS),
                        Uniform::new("tint_color", grey_a(0.15, PANEL_TINT)),
                    ],
                )?;
            }
            self.draw_cursor(&mut frame, state, fx)?;
            sync = frame.finish()?;
        }
        Ok(sync)
    }

    fn ensure_textures(
        &mut self,
        renderer: &mut GlesRenderer,
        size: (i32, i32),
        panel: Option<Rect>,
    ) -> Result<(), RenderError> {
        let make = |r: &mut GlesRenderer, w: i32, h: i32| {
            Offscreen::<GlesTexture>::create_buffer(
                r,
                Fourcc::Abgr8888,
                (w.max(1), h.max(1)).into(),
            )
        };
        if self.scene.as_ref().map(|(_, s)| *s) != Some(size) {
            self.scene = Some((make(renderer, size.0, size.1)?, size));
        }
        if let Some(p) = panel {
            let key = (p, size);
            if self.chain.as_ref().map(|c| c.key) != Some(key) {
                let pad = padding();
                let (pw, ph) = (p.w + 2 * pad, p.h + 2 * pad);
                self.chain = Some(BlurChain {
                    key,
                    half: make(renderer, pw / 2, ph / 2)?,
                    quarter: make(renderer, pw / 4, ph / 4)?,
                    half_up: make(renderer, pw / 2, ph / 2)?,
                    out: make(renderer, p.w, p.h)?,
                });
            }
        }
        Ok(())
    }

    /// scene --down--> half --down--> quarter --up--> half_up --up(crop)--> out
    fn blur_panel(&mut self, renderer: &mut GlesRenderer, panel: Rect) -> Result<(), RenderError> {
        let (Some((scene, _)), Some(chain)) = (self.scene.as_ref(), self.chain.as_mut()) else {
            return Ok(());
        };
        let pad = padding();
        let offset = kawase_offset(BLUR_RADIUS);
        let padded = Rect::new(
            panel.x - pad,
            panel.y - pad,
            panel.w + 2 * pad,
            panel.h + 2 * pad,
        );
        let scene_size = scene.size();

        // Pass 1: crop of the scene -> half.
        pass(
            renderer,
            &mut chain.half,
            scene,
            Rectangle::new(
                (f64::from(padded.x), f64::from(padded.y)).into(),
                (f64::from(padded.w), f64::from(padded.h)).into(),
            ),
            &self.programs.kawase_down,
            (1.0 / scene_size.w as f32, 1.0 / scene_size.h as f32),
            offset,
        )?;
        // Pass 2: half -> quarter.
        let half_size = chain.half.size();
        pass(
            renderer,
            &mut chain.quarter,
            &chain.half,
            Rectangle::from_size(half_size.to_f64()),
            &self.programs.kawase_down,
            (1.0 / half_size.w as f32, 1.0 / half_size.h as f32),
            offset,
        )?;
        // Pass 3: quarter -> half_up.
        let q_size = chain.quarter.size();
        pass(
            renderer,
            &mut chain.half_up,
            &chain.quarter,
            Rectangle::from_size(q_size.to_f64()),
            &self.programs.kawase_up,
            (0.5 / q_size.w as f32, 0.5 / q_size.h as f32),
            offset,
        )?;
        // Pass 4: crop the panel out of half_up and scale to full size.
        let hu = chain.half_up.size();
        pass(
            renderer,
            &mut chain.out,
            &chain.half_up,
            Rectangle::new(
                (f64::from(pad) / 2.0, f64::from(pad) / 2.0).into(),
                (f64::from(panel.w) / 2.0, f64::from(panel.h) / 2.0).into(),
            ),
            &self.programs.kawase_up,
            (0.5 / hu.w as f32, 0.5 / hu.h as f32),
            offset,
        )?;
        Ok(())
    }

    fn draw_frost(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        panel: Rect,
        blurred: &GlesTexture,
    ) -> Result<(), RenderError> {
        let dst = px_rect(panel);
        frame.render_texture_from_to(
            blurred,
            Rectangle::from_size(blurred.size().to_f64()),
            dst,
            &full_damage(dst),
            &[],
            Transform::Normal,
            1.0,
            Some(&self.programs.frost),
            &[
                Uniform::new("size", (panel.w as f32, panel.h as f32)),
                Uniform::new("radius", CORNER_RADIUS),
                // Glass-like for the demo: surface.1 at 0.40 so the blur stays
                // visible (Frost proper is 0.72), saturate x1.3, rim 12%.
                Uniform::new("tint_color", grey_a(0.15, PANEL_TINT)),
                Uniform::new("saturation", 1.3f32),
                Uniform::new("rim", 0.12f32),
            ],
        )?;
        Ok(())
    }

    fn draw_scene(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        state: &FloorState,
        drawn: &[DrawnWindow],
        size: (i32, i32),
        fx: Effects,
    ) -> Result<(), RenderError> {
        let full = Rectangle::<i32, Physical>::from_size(size.into());
        let sz_buf: Size<i32, Buffer> = size.into();
        frame.clear(Color32F::new(0.0, 0.0, 0.0, 1.0), &[full])?;
        frame.render_pixel_shader_to(
            &self.programs.background,
            Rectangle::from_size(sz_buf.to_f64()),
            full,
            sz_buf,
            None,
            1.0,
            &[
                Uniform::new("top", grey(0.27)),
                Uniform::new("bottom", grey(0.17)),
            ],
        )?;

        let top = state.windows.len().saturating_sub(1);
        for d in drawn {
            let w = &state.windows[d.index];
            let layout = w.layout();
            let focused = d.index == top;
            if fx.shadows {
                self.draw_shadow(frame, &layout)?;
            }
            if w.server_side {
                self.draw_titlebar(frame, state, d.index, &layout, focused, fx)?;
            }
            self.draw_client(frame, w, &layout, d, fx)?;
        }
        Ok(())
    }

    /// A small neutral dot: enough to see where the pointer is on a bare TTY.
    fn draw_cursor(&self, frame: &mut GlesFrame<'_, '_>, state: &FloorState, fx: Effects) -> Result<(), RenderError> {
        if !fx.cursor {
            return Ok(());
        }
        let (x, y) = state.pointer;
        let r = Rect::new(x as i32 - 1, y as i32 - 1, 14, 14);
        self.rounded_box(frame, r, [7.0; 4], [1.0, 1.0, 1.0, 0.95], [0.0, 0.0, 0.0, 0.8])
    }

    fn draw_shadow(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        layout: &FrameLayout,
    ) -> Result<(), RenderError> {
        let b = layout.shadow_bounds();
        let o = layout.outer();
        frame.render_pixel_shader_to(
            &self.programs.shadow,
            Rectangle::from_size(Size::<f64, Buffer>::from((f64::from(b.w), f64::from(b.h)))),
            px_rect(b),
            (b.w, b.h).into(),
            None,
            1.0,
            &[
                Uniform::new(
                    "box_rect",
                    (
                        (o.x - b.x) as f32,
                        (o.y - b.y) as f32,
                        o.w as f32,
                        o.h as f32,
                    ),
                ),
                Uniform::new("radius", CORNER_RADIUS),
                Uniform::new(
                    "key",
                    (SHADOW_OFFSET_Y as f32, SHADOW_BLUR, SHADOW_ALPHA, 0.0),
                ),
                Uniform::new(
                    "contact",
                    (CONTACT_OFFSET_Y as f32, CONTACT_BLUR, CONTACT_ALPHA, 0.0),
                ),
            ],
        )?;
        Ok(())
    }

    fn rounded_box(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        rect: Rect,
        radii: [f32; 4],
        fill: [f32; 4],
        hairline: [f32; 4],
    ) -> Result<(), RenderError> {
        frame.render_pixel_shader_to(
            &self.programs.rounded_box,
            Rectangle::from_size(Size::<f64, Buffer>::from((
                f64::from(rect.w),
                f64::from(rect.h),
            ))),
            px_rect(rect),
            (rect.w, rect.h).into(),
            None,
            1.0,
            &[
                Uniform::new("radii", radii),
                Uniform::new("fill", fill),
                Uniform::new("hairline", hairline),
            ],
        )?;
        Ok(())
    }

    fn draw_titlebar(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        state: &FloorState,
        index: usize,
        layout: &FrameLayout,
        focused: bool,
        fx: Effects,
    ) -> Result<(), RenderError> {
        let r = if fx.rounded { CORNER_RADIUS } else { 0.0 };
        // Neutral surfaces (D59): focused surface.3, unfocused surface.2.
        let fill = grey_a(if focused { 0.23 } else { 0.19 }, 1.0);
        self.rounded_box(frame, layout.outer(), [r; 4], fill, [1.0, 1.0, 1.0, 0.08])?;
        let hover = state.hover.filter(|(i, _)| *i == index).map(|(_, h)| h);
        for (rect, hit) in [
            (layout.close_button(), Hit::Close),
            (layout.maximize_button(), Hit::Maximize),
            (layout.minimize_button(), Hit::Minimize),
        ] {
            let l = if hover == Some(hit) {
                0.62
            } else if focused {
                0.42
            } else {
                0.30
            };
            let rad = BUTTON_SIZE as f32 / 2.0;
            self.rounded_box(frame, rect, [rad; 4], grey_a(l, 1.0), [0.0; 4])?;
        }
        Ok(())
    }

    fn draw_client(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        w: &FloorWindow,
        _layout: &FrameLayout,
        d: &DrawnWindow,
        fx: Effects,
    ) -> Result<(), RenderError> {
        let geo = w.window.geometry();
        // Round only the bottom corners under a titlebar; a client that
        // draws its own decorations is not masked at all.
        let masked = w.server_side && fx.rounded;
        for e in &d.surface {
            if masked && let Some(buf) = d.buffer_size {
                frame.override_default_tex_program(
                    self.programs.clip_surface.clone(),
                    vec![
                        Uniform::new("buf_size", (buf.0 as f32, buf.1 as f32)),
                        Uniform::new(
                            "geo",
                            (
                                geo.loc.x as f32,
                                geo.loc.y as f32,
                                geo.size.w as f32,
                                geo.size.h as f32,
                            ),
                        ),
                        Uniform::new("radii", [0.0, 0.0, CORNER_RADIUS, CORNER_RADIUS]),
                    ],
                );
            }
            let dst = e.geometry(1.0.into());
            e.draw(frame, e.src(), dst, &[Rectangle::from_size(dst.size)], &[])?;
            frame.clear_tex_program_override();
        }
        for e in &d.popups {
            let dst = e.geometry(1.0.into());
            e.draw(frame, e.src(), dst, &[Rectangle::from_size(dst.size)], &[])?;
        }
        Ok(())
    }
}

/// Padding around the panel that the blur samples from: the effective
/// reach of the chosen levels, rounded to a multiple of 4 so every level
/// has whole pixels.
fn padding() -> i32 {
    (((BLUR_RADIUS * 1.5) as i32 + 3) / 4) * 4
}

fn pass(
    renderer: &mut GlesRenderer,
    target: &mut GlesTexture,
    source: &GlesTexture,
    src: Rectangle<f64, Buffer>,
    program: &GlesTexProgram,
    texel: (f32, f32),
    offset: f32,
) -> Result<(), RenderError> {
    let size = target.size();
    let out: Size<i32, Physical> = (size.w, size.h).into();
    let mut fb = renderer.bind(target)?;
    let mut frame = renderer.render(&mut fb, out, Transform::Normal)?;
    let dst = Rectangle::from_size(out);
    frame.render_texture_from_to(
        source,
        src,
        dst,
        &full_damage(dst),
        &[],
        Transform::Normal,
        1.0,
        Some(program),
        &[
            Uniform::new("halfpixel", (texel.0 * 0.5, texel.1 * 0.5)),
            Uniform::new("offset", offset),
        ],
    )?;
    let _ = frame.finish()?;
    Ok(())
}

type SurfaceElement = WaylandSurfaceRenderElement<GlesRenderer>;

struct DrawnWindow {
    index: usize,
    surface: Vec<SurfaceElement>,
    popups: Vec<SurfaceElement>,
    buffer_size: Option<(i32, i32)>,
}

fn collect_window(
    renderer: &mut GlesRenderer,
    state: &FloorState,
    index: usize,
    w: &FloorWindow,
) -> DrawnWindow {
    let origin = state.surface_origin(index);
    let Some(surface) = w.surface() else {
        return DrawnWindow {
            index,
            surface: vec![],
            popups: vec![],
            buffer_size: None,
        };
    };
    let buffer_size = with_renderer_surface_state(&surface, |s| s.buffer_size())
        .flatten()
        .map(|s| (s.w, s.h));
    let tree =
        |renderer: &mut GlesRenderer, s: &WlSurface, at: (i32, i32)| -> Vec<SurfaceElement> {
            render_elements_from_surface_tree(renderer, s, at, 1.0, 1.0, Kind::Unspecified)
        };
    let mut surface_elems = tree(renderer, &surface, origin);
    // Smithay returns top-most first; draw bottom-most first.
    surface_elems.reverse();
    let geo_loc = w.window.geometry().loc;
    let mut popups = Vec::new();
    for (popup, loc) in PopupManager::popups_for_surface(&surface) {
        let off = geo_loc + loc - popup.geometry().loc;
        let mut v = tree(
            renderer,
            popup.wl_surface(),
            (origin.0 + off.x, origin.1 + off.y),
        );
        v.reverse();
        popups.extend(v);
    }
    DrawnWindow {
        index,
        surface: surface_elems,
        popups,
        buffer_size,
    }
}

/// Keep the unused-import lint honest for traits needed only by method
/// resolution in generic bounds.
#[allow(dead_code)]
fn _assert_bounds<R: Renderer + ImportAll>() {}
#[allow(dead_code)]
fn _surface_type() -> WindowSurfaceType {
    WindowSurfaceType::ALL
}
