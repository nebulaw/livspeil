// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Window-frame geometry and the CPU reference of the Floor's effect math.
//!
//! Everything here is plain `f32`/`i32` and platform independent, so it is
//! unit-tested on every OS. The GLSL in `render::shaders` (Linux only) is a
//! line-by-line port of [`rounded_box_sdf`], [`coverage`] and
//! [`shadow_falloff`]; keep them in sync.

/// Titlebar height in logical pixels.
pub const TITLEBAR_HEIGHT: i32 = 32;
/// Outer corner radius of a framed window (logical pixels).
pub const CORNER_RADIUS: f32 = 12.0;
/// Diameter of a titlebar button.
pub const BUTTON_SIZE: i32 = 14;
/// Gap between titlebar buttons and to the titlebar edge.
pub const BUTTON_GAP: i32 = 10;

/// An axis-aligned rectangle in logical pixels (`x`, `y` is the top-left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    /// Whether the point lies inside (right and bottom edges exclusive).
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= f64::from(self.x)
            && py >= f64::from(self.y)
            && px < f64::from(self.x + self.w)
            && py < f64::from(self.y + self.h)
    }

    /// Grow by `by` on every side.
    pub fn inflate(&self, by: i32) -> Rect {
        Rect::new(self.x - by, self.y - by, self.w + 2 * by, self.h + 2 * by)
    }

    /// Centre of the rectangle.
    pub fn center(&self) -> (f32, f32) {
        (
            self.x as f32 + self.w as f32 * 0.5,
            self.y as f32 + self.h as f32 * 0.5,
        )
    }

    /// Intersection with `other`, or `None` when they do not overlap.
    pub fn intersect(&self, other: Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// What a point on a framed window hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Outside the window.
    None,
    /// The client area (events go to the client surface).
    Client,
    /// Empty titlebar: drag to move.
    Titlebar,
    Close,
    Maximize,
    Minimize,
}

/// Layout of one server-decorated window.
///
/// `origin` is the top-left of the *outer* frame (titlebar included);
/// `client` is the size of the client's window geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameLayout {
    pub origin: (i32, i32),
    pub client: (i32, i32),
    /// Titlebar height: [`TITLEBAR_HEIGHT`], or 0 for clients that draw
    /// their own decorations.
    pub titlebar_h: i32,
}

impl FrameLayout {
    pub fn new(origin: (i32, i32), client: (i32, i32)) -> Self {
        FrameLayout {
            origin,
            client,
            titlebar_h: TITLEBAR_HEIGHT,
        }
    }

    /// A frame without a titlebar (client-side decorations).
    pub fn undecorated(origin: (i32, i32), client: (i32, i32)) -> Self {
        FrameLayout {
            origin,
            client,
            titlebar_h: 0,
        }
    }

    /// Outer rectangle: titlebar plus client.
    pub fn outer(&self) -> Rect {
        Rect::new(
            self.origin.0,
            self.origin.1,
            self.client.0,
            self.client.1 + self.titlebar_h,
        )
    }

    pub fn titlebar(&self) -> Rect {
        Rect::new(self.origin.0, self.origin.1, self.client.0, self.titlebar_h)
    }

    /// Where the client's window geometry is placed.
    pub fn client_rect(&self) -> Rect {
        Rect::new(
            self.origin.0,
            self.origin.1 + self.titlebar_h,
            self.client.0,
            self.client.1,
        )
    }

    /// Button rectangles, right-aligned: close is the rightmost.
    pub fn close_button(&self) -> Rect {
        self.button(0)
    }
    pub fn maximize_button(&self) -> Rect {
        self.button(1)
    }
    pub fn minimize_button(&self) -> Rect {
        self.button(2)
    }

    fn button(&self, index_from_right: i32) -> Rect {
        let x = self.origin.0 + self.client.0 - (BUTTON_GAP + BUTTON_SIZE) * (index_from_right + 1);
        let y = self.origin.1 + (TITLEBAR_HEIGHT - BUTTON_SIZE) / 2;
        Rect::new(x, y, BUTTON_SIZE, BUTTON_SIZE)
    }

    /// Hit-test a point given in the same coordinate space as `origin`.
    pub fn hit_test(&self, px: f64, py: f64) -> Hit {
        let outer = self.outer();
        if !outer.contains(px, py) {
            return Hit::None;
        }
        // The rounded corners are part of the window for input purposes;
        // a corner pixel that is fully transparent still belongs to it.
        if self.client_rect().contains(px, py) {
            return Hit::Client;
        }
        if self.titlebar_h == 0 {
            return Hit::None;
        }
        // Buttons get a slightly larger target than they are drawn.
        for (rect, hit) in [
            (self.close_button(), Hit::Close),
            (self.maximize_button(), Hit::Maximize),
            (self.minimize_button(), Hit::Minimize),
        ] {
            if rect.inflate(BUTTON_GAP / 2).contains(px, py) {
                return hit;
            }
        }
        Hit::Titlebar
    }

    /// Pixel margin that must be drawn around the outer rectangle so the
    /// shadow (e3: `0 12 32` + `0 2 6`) is not clipped.
    pub fn shadow_bounds(&self) -> Rect {
        let o = self.outer();
        Rect::new(
            o.x - SHADOW_MARGIN,
            o.y - SHADOW_MARGIN + SHADOW_OFFSET_Y,
            o.w + 2 * SHADOW_MARGIN,
            o.h + 2 * SHADOW_MARGIN,
        )
    }
}

/// Key shadow layer of elevation e3: offset y, blur radius, opacity.
pub const SHADOW_OFFSET_Y: i32 = 12;
pub const SHADOW_BLUR: f32 = 32.0;
pub const SHADOW_ALPHA: f32 = 0.35;
/// Margin around the outer rect that contains the whole key shadow
/// (blur radius plus offset, rounded up).
pub const SHADOW_MARGIN: i32 = 48;

/// Signed distance from `p` to a rounded box centred at `c` with half-size
/// `half`; `radii` are `[top-left, top-right, bottom-right, bottom-left]`.
/// Negative inside. Y grows downwards.
pub fn rounded_box_sdf(p: (f32, f32), c: (f32, f32), half: (f32, f32), radii: [f32; 4]) -> f32 {
    let q0 = (p.0 - c.0, p.1 - c.1);
    let r = match (q0.0 >= 0.0, q0.1 >= 0.0) {
        (false, false) => radii[0],
        (true, false) => radii[1],
        (true, true) => radii[2],
        (false, true) => radii[3],
    };
    let q = (q0.0.abs() - half.0 + r, q0.1.abs() - half.1 + r);
    let outside = (q.0.max(0.0).powi(2) + q.1.max(0.0).powi(2)).sqrt();
    outside + q.0.max(q.1).min(0.0) - r
}

/// Anti-aliased coverage for a signed distance (1 px wide ramp).
pub fn coverage(sdf: f32) -> f32 {
    (0.5 - sdf).clamp(0.0, 1.0)
}

/// Fraction of a Gaussian-blurred edge that remains at signed distance `d`
/// (negative inside) for blur radius `blur` (CSS convention: sigma =
/// blur / 2). Uses the logistic approximation of the normal CDF,
/// `Phi(x) ~ 1 / (1 + exp(-1.702 x))`, which is within 0.01 of the exact
/// value and cheap in a fragment shader.
pub fn shadow_falloff(d: f32, blur: f32) -> f32 {
    let sigma = (blur * 0.5).max(1e-3);
    1.0 / (1.0 + (1.702 * d / sigma).exp())
}

/// Shadow opacity of a rounded box at point `p` (reference for the shader).
pub fn shadow_alpha(p: (f32, f32), outer: Rect, radius: f32) -> f32 {
    let (cx, cy) = outer.center();
    let c = (cx, cy + SHADOW_OFFSET_Y as f32);
    let d = rounded_box_sdf(
        p,
        c,
        (outer.w as f32 * 0.5, outer.h as f32 * 0.5),
        [radius; 4],
    );
    SHADOW_ALPHA * shadow_falloff(d, SHADOW_BLUR)
}

/// sRGB (0..=255) of a neutral (chroma 0) OKLCH lightness, as used by the
/// `surface.0..3` roles (D59). For chroma 0, OKLab `L` maps to linear
/// luminance `L^3`.
pub fn neutral_srgb(l: f32) -> u8 {
    let lin = l.clamp(0.0, 1.0).powi(3);
    let s = if lin <= 0.003_130_8 {
        12.92 * lin
    } else {
        1.055 * lin.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Dual-Kawase pass plan for a backdrop blur of `radius` logical pixels:
/// number of down/up levels. Level `n` samples at `1/2^n` resolution; the
/// effective radius of `n` levels with a 1-texel offset is roughly
/// `2^(n+1)` pixels per side, so pick the smallest `n` that reaches it.
pub fn kawase_levels(radius: f32) -> u32 {
    let mut n = 1;
    while ((1u32 << (n + 1)) as f32) < radius && n < 5 {
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_rects_tile_the_outer_rect() {
        let l = FrameLayout::new((100, 50), (640, 400));
        assert_eq!(l.outer(), Rect::new(100, 50, 640, 432));
        assert_eq!(l.titlebar(), Rect::new(100, 50, 640, 32));
        assert_eq!(l.client_rect(), Rect::new(100, 82, 640, 400));
    }

    #[test]
    fn hit_test_regions() {
        let l = FrameLayout::new((0, 0), (300, 200));
        assert_eq!(l.hit_test(-1.0, 10.0), Hit::None);
        assert_eq!(l.hit_test(150.0, 100.0), Hit::Client);
        assert_eq!(l.hit_test(20.0, 5.0), Hit::Titlebar);
        let c = l.close_button();
        assert_eq!(
            l.hit_test(f64::from(c.x + 3), f64::from(c.y + 3)),
            Hit::Close
        );
        let m = l.maximize_button();
        assert_eq!(
            l.hit_test(f64::from(m.x + 3), f64::from(m.y + 3)),
            Hit::Maximize
        );
        let n = l.minimize_button();
        assert_eq!(
            l.hit_test(f64::from(n.x + 3), f64::from(n.y + 3)),
            Hit::Minimize
        );
        // Bottom edge exclusive.
        assert_eq!(l.hit_test(10.0, 232.0), Hit::None);
        assert_eq!(l.hit_test(10.0, 231.5), Hit::Client);
    }

    #[test]
    fn undecorated_frame_has_no_titlebar() {
        let l = FrameLayout::undecorated((10, 10), (100, 50));
        assert_eq!(l.outer(), l.client_rect());
        assert_eq!(l.hit_test(20.0, 12.0), Hit::Client);
        assert_eq!(l.hit_test(9.0, 12.0), Hit::None);
    }

    #[test]
    fn buttons_do_not_overlap_and_fit() {
        let l = FrameLayout::new((0, 0), (120, 80));
        let (c, m, n) = (l.close_button(), l.maximize_button(), l.minimize_button());
        assert!(c.x > m.x + m.w && m.x > n.x + n.w);
        assert!(n.x >= 0, "three buttons fit a 120 px titlebar");
    }

    #[test]
    fn sdf_signs_and_corner() {
        let c = (50.0, 50.0);
        let half = (50.0, 50.0);
        assert!(rounded_box_sdf((50.0, 50.0), c, half, [12.0; 4]) < -49.0);
        assert!((rounded_box_sdf((100.0, 50.0), c, half, [12.0; 4])).abs() < 1e-4);
        // The sharp corner point is outside a rounded box by r*(sqrt2-1).
        let d = rounded_box_sdf((100.0, 100.0), c, half, [12.0; 4]);
        assert!((d - 12.0 * (2f32.sqrt() - 1.0)).abs() < 1e-3, "{d}");
        // With a zero radius on that corner it is exactly on the edge.
        let d = rounded_box_sdf((100.0, 100.0), c, half, [0.0, 0.0, 0.0, 12.0]);
        assert!(d.abs() < 1e-4, "{d}");
    }

    #[test]
    fn coverage_ramp() {
        assert_eq!(coverage(-2.0), 1.0);
        assert_eq!(coverage(2.0), 0.0);
        assert!((coverage(0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn shadow_falloff_matches_normal_cdf() {
        // Exact: 1 - Phi(d / sigma). Logistic approximation error < 0.01.
        fn phi(x: f64) -> f64 {
            // Abramowitz-Stegun 7.1.26 erf.
            let t = 1.0 / (1.0 + 0.327_591_1 * (x / 2f64.sqrt()).abs());
            let y = 1.0
                - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t
                    - 0.284_496_736)
                    * t
                    + 0.254_829_592)
                    * t
                    * (-(x * x) / 2.0).exp();
            if x >= 0.0 {
                0.5 * (1.0 + y)
            } else {
                0.5 * (1.0 - y)
            }
        }
        let blur = 32.0;
        let sigma = f64::from(blur) / 2.0;
        for i in -40..=40 {
            let d = i as f32;
            let exact = 1.0 - phi(f64::from(d) / sigma);
            let got = f64::from(shadow_falloff(d, blur));
            assert!((exact - got).abs() < 0.01, "d={d}: {exact} vs {got}");
        }
        assert!((shadow_falloff(0.0, blur) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn shadow_is_offset_downwards_and_fades() {
        let outer = Rect::new(100, 100, 300, 200);
        let (cx, _) = outer.center();
        let above = shadow_alpha((cx, 100.0 - 20.0), outer, CORNER_RADIUS);
        let below = shadow_alpha((cx, 300.0 + 20.0), outer, CORNER_RADIUS);
        assert!(below > above * 2.0, "{below} vs {above}");
        // Beyond the margin the shadow is negligible.
        let far = shadow_alpha(
            (cx, 300.0 + SHADOW_MARGIN as f32 + SHADOW_OFFSET_Y as f32),
            outer,
            CORNER_RADIUS,
        );
        assert!(far < 0.02, "{far}");
        // Never exceeds the configured opacity.
        assert!(shadow_alpha((cx, 200.0), outer, CORNER_RADIUS) <= SHADOW_ALPHA);
    }

    #[test]
    fn shadow_bounds_contain_visible_shadow() {
        let l = FrameLayout::new((10, 10), (200, 100));
        let b = l.shadow_bounds();
        let o = l.outer();
        assert!(b.x < o.x && b.y < o.y && b.x + b.w > o.x + o.w && b.y + b.h > o.y + o.h);
    }

    #[test]
    fn neutral_surfaces_match_spec_lightness() {
        // surface.0 L=0.10, surface.1 L=0.15 ... (D59): neutral, monotonic.
        let v: Vec<u8> = [0.10, 0.15, 0.19, 0.23]
            .iter()
            .map(|l| neutral_srgb(*l))
            .collect();
        assert!(v.windows(2).all(|w| w[0] < w[1]), "{v:?}");
        assert_eq!(neutral_srgb(1.0), 255);
        assert_eq!(neutral_srgb(0.0), 0);
    }

    #[test]
    fn kawase_levels_grow_with_radius() {
        assert_eq!(kawase_levels(4.0), 1);
        assert!(kawase_levels(24.0) >= 3);
        assert!(kawase_levels(1000.0) <= 5);
    }

    #[test]
    fn rect_intersect() {
        let a = Rect::new(0, 0, 10, 10);
        assert_eq!(
            a.intersect(Rect::new(5, 5, 10, 10)),
            Some(Rect::new(5, 5, 5, 5))
        );
        assert_eq!(a.intersect(Rect::new(10, 0, 5, 5)), None);
    }
}
