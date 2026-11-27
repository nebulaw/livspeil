// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

// Every primitive family, in one instanced pipeline: sprites (glyphs and
// icons), SDF quads, analytic shadows, cubic Bezier curves, and the
// backdrop composite. The per-record math is mirrored by src/shade.rs (the
// CPU reference); keep both in step.
//
// Data (src/record.rs): each instance is one u32, a family tag and the slot
// of its record in `records`; records refer to `properties` by index for
// transform, clip and opacity. Uses two storage buffers per stage, within
// the downlevel limit of four.

const OFFSET_BITS: u32 = 29u;
const OFFSET_MASK: u32 = 0x1fffffffu;
const FAMILY_SPRITE: u32 = 0u;
const FAMILY_QUAD: u32 = 1u;
const FAMILY_SHADOW: u32 = 2u;
const FAMILY_CURVE: u32 = 3u;
const FAMILY_BACKDROP: u32 = 4u;

const FILL_SOLID: u32 = 1u;
const FILL_LINEAR: u32 = 2u;

const MAX_CURVE_SEGMENTS: u32 = 64u;
// Physical pixels past a stroke's edge within which a segment's box draws.
const CLAIM_MARGIN_PX: f32 = 0.75;
const INV_TEXT_GAMMA: f32 = 1.0 / 1.8;

struct SpriteRecord {
    rect: vec4<f32>,
    color: vec4<f32>,
    texels: vec2<u32>,    // x | y << 16, w | h << 16
    prop: u32,
    lightness: f32,
}

struct QuadRecord {
    rect: vec4<f32>,
    ext: vec4<f32>,       // corner extents tl tr br bl
    exps: vec4<f32>,      // corner exponents
    fill: vec4<f32>,
    border: vec4<f32>,
    prop: u32,
    flags: u32,           // fill kind | stop count << 8
    border_width: f32,
    effect: u32,          // slot of the GradientRecord
}

struct GradientRecord {
    geometry: vec4<f32>,
    offsets: vec4<f32>,
    stop0: vec4<f32>,     // OKLab + alpha
    stop1: vec4<f32>,
    stop2: vec4<f32>,
    stop3: vec4<f32>,
}

struct ShadowRecord {
    rect: vec4<f32>,
    ext: vec4<f32>,
    exps: vec4<f32>,
    shadow: vec4<f32>,    // dx dy spread sigma
    color: vec4<f32>,
    prop: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
}

struct CurveRecord {
    p01: vec4<f32>,
    p23: vec4<f32>,
    color: vec4<f32>,
    prop: u32,
    half_width: f32,
    segments: u32,
    window: u32,          // segments to search on either side
}

struct BackdropRecord {
    rect: vec4<f32>,
    ext: vec4<f32>,
    exps: vec4<f32>,
    tint: vec4<f32>,
    rim: vec4<f32>,
    params: vec4<f32>,    // highlight width, highlight kind, saturation, grain
    prop: u32,
    has_blur: u32,
    pad0: u32,
    pad1: u32,
}

struct Property {
    linear: vec4<f32>,    // a b c d
    inverse: vec4<f32>,
    offset: vec4<f32>,    // e f of the transform, then of the inverse
    clip: vec4<f32>,      // physical pixels x0 y0 x1 y1
    ppu: f32,             // physical pixels per local unit
    opacity: f32,
    pad0: f32,
    pad1: f32,
}

struct Globals {
    viewport: vec2<f32>,
    scale: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> records: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read> properties: array<Property>;
@group(1) @binding(0) var blur_tex: texture_2d<f32>;
@group(1) @binding(1) var blur_sampler: sampler;
// R8 coverage atlas for sprites; read with textureLoad.
@group(2) @binding(0) var glyph_atlas: texture_2d<f32>;

fn slot_f(at: u32) -> vec4<f32> {
    return bitcast<vec4<f32>>(records[at]);
}

fn load_sprite(at: u32) -> SpriteRecord {
    let t = records[at + 2u];
    return SpriteRecord(slot_f(at), slot_f(at + 1u), t.xy, t.z, bitcast<f32>(t.w));
}

fn load_quad(at: u32) -> QuadRecord {
    let t = records[at + 5u];
    return QuadRecord(
        slot_f(at), slot_f(at + 1u), slot_f(at + 2u), slot_f(at + 3u), slot_f(at + 4u),
        t.x, t.y, bitcast<f32>(t.z), t.w,
    );
}

fn load_gradient(at: u32) -> GradientRecord {
    return GradientRecord(
        slot_f(at), slot_f(at + 1u), slot_f(at + 2u), slot_f(at + 3u), slot_f(at + 4u),
        slot_f(at + 5u),
    );
}

fn load_shadow(at: u32) -> ShadowRecord {
    let t = records[at + 5u];
    return ShadowRecord(
        slot_f(at), slot_f(at + 1u), slot_f(at + 2u), slot_f(at + 3u), slot_f(at + 4u),
        t.x, t.y, t.z, t.w,
    );
}

fn load_curve(at: u32) -> CurveRecord {
    let t = records[at + 3u];
    return CurveRecord(
        slot_f(at), slot_f(at + 1u), slot_f(at + 2u),
        t.x, bitcast<f32>(t.y), t.z, t.w,
    );
}

fn load_backdrop(at: u32) -> BackdropRecord {
    let t = records[at + 6u];
    return BackdropRecord(
        slot_f(at), slot_f(at + 1u), slot_f(at + 2u), slot_f(at + 3u), slot_f(at + 4u),
        slot_f(at + 5u), t.x, t.y, t.z, t.w,
    );
}

// ---- geometry ----

// x0 y0 x1 y1 of (x, y, w, h) grown by d on every side.
fn grown(rect: vec4<f32>, d: f32) -> vec4<f32> {
    let lo = rect.xy - vec2(d);
    return vec4(lo, lo + (rect.zw + vec2(2.0 * d)));
}

fn shape_bounds(rect: vec4<f32>, prop: Property) -> vec4<f32> {
    return grown(rect, 1.0 / prop.ppu);
}

fn sprite_bounds(sprite: SpriteRecord, prop: Property) -> vec4<f32> {
    var d = 1.0 / prop.ppu;
    if all(prop.linear == vec4(1.0, 0.0, 0.0, 1.0)) {
        d = 0.0;
    }
    return grown(sprite.rect, d);
}

fn shadow_bounds(sh: ShadowRecord, prop: Property) -> vec4<f32> {
    let sigma = sh.shadow.w;
    let spread = max(sh.shadow.z, -0.5 * min(sh.rect.z, sh.rect.w));
    let shape = vec4(
        sh.rect.xy + sh.shadow.xy - vec2(spread),
        sh.rect.zw + vec2(2.0 * spread),
    );
    return grown(shape, 3.0 * sigma + 1.0 / prop.ppu);
}

// Local to clip space.
fn project(local: vec2<f32>, prop: Property) -> vec4<f32> {
    let screen = vec2(
        prop.linear.x * local.x + prop.linear.z * local.y + prop.offset.x,
        prop.linear.y * local.x + prop.linear.w * local.y + prop.offset.y,
    );
    let px = screen * globals.scale;
    return vec4(px.x / globals.viewport.x * 2.0 - 1.0, 1.0 - px.y / globals.viewport.y * 2.0, 0.0, 1.0);
}

// The point of a curve at parameter t.
fn curve_point(c: CurveRecord, t: f32) -> vec2<f32> {
    let u = 1.0 - t;
    return u * u * u * c.p01.xy + 3.0 * u * u * t * c.p01.zw
        + 3.0 * u * t * t * c.p23.xy + t * t * t * c.p23.zw;
}

fn segment_distance(q: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let aq = q - a;
    let len2 = ab.x * ab.x + ab.y * ab.y;
    var t = 0.0;
    if len2 > 0.0 {
        t = clamp((aq.x * ab.x + aq.y * ab.y) / len2, 0.0, 1.0);
    }
    let d = aq - ab * t;
    return sqrt(d.x * d.x + d.y * d.y);
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) instance: u32,
    // The curve segment whose box this is.
    @location(1) @interpolate(flat) segment: u32,
}

// A quad of two triangles per instance, covering the record's bounds; a
// curve is a strip of such quads, one box per segment.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @location(0) instance: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let t = corners[vi % 6u];
    let at = instance & OFFSET_MASK;
    var out: VsOut;
    out.instance = instance;
    out.segment = vi / 6u;
    var bounds = vec4(0.0);
    var prop_index = 0u;
    switch instance >> OFFSET_BITS {
        case FAMILY_SPRITE: {
            let r = load_sprite(at);
            prop_index = r.prop;
            bounds = sprite_bounds(r, properties[r.prop]);
        }
        case FAMILY_QUAD: {
            let r = load_quad(at);
            prop_index = r.prop;
            bounds = shape_bounds(r.rect, properties[r.prop]);
        }
        case FAMILY_SHADOW: {
            let r = load_shadow(at);
            prop_index = r.prop;
            bounds = shadow_bounds(r, properties[r.prop]);
        }
        case FAMILY_BACKDROP: {
            let r = load_backdrop(at);
            prop_index = r.prop;
            bounds = shape_bounds(r.rect, properties[r.prop]);
        }
        case FAMILY_CURVE: {
            let c = load_curve(at);
            // A draw has the vertices of its longest curve; the rest of a
            // shorter one collapses to a point.
            if out.segment >= c.segments {
                out.position = vec4(0.0, 0.0, 0.0, 1.0);
                return out;
            }
            let prop = properties[c.prop];
            let n = f32(c.segments);
            let a = curve_point(c, f32(out.segment) / n);
            let b = curve_point(c, f32(out.segment + 1u) / n);
            let ab = b - a;
            let len = length(ab);
            var dir = vec2(1.0, 0.0);
            if len > 1e-6 {
                dir = ab / len;
            }
            let reach = c.half_width + 1.0 / prop.ppu;
            let along = mix(a - dir * reach, b + dir * reach, t.x);
            out.position = project(along + vec2(-dir.y, dir.x) * mix(-reach, reach, t.y), prop);
            return out;
        }
        default: {}
    }
    out.position = project(mix(bounds.xy, bounds.zw, t), properties[prop_index]);
    return out;
}

// ---- per-pixel math ----

fn bayer(px: vec2<f32>) -> f32 {
    var m = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    let x = u32(max(px.x, 0.0)) & 3u;
    let y = u32(max(px.y, 0.0)) & 3u;
    return (m[y * 4u + x] + 0.5) / 16.0 - 0.5;
}

fn noise(px: vec2<f32>) -> f32 {
    let x = u32(max(px.x, 0.0));
    let y = u32(max(px.y, 0.0));
    var h = (x * 0x8da6b343u) ^ (y * 0xd8163841u);
    h = h ^ (h >> 16u);
    h = h * 0x7feb352du;
    h = h ^ (h >> 15u);
    h = h * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h >> 8u) / 16777216.0 - 0.5;
}

fn corner_index(q: vec2<f32>) -> u32 {
    if q.x < 0.0 {
        if q.y < 0.0 { return 0u; }
        return 3u;
    }
    if q.y < 0.0 { return 1u; }
    return 2u;
}

fn sd_shape(p: vec2<f32>, rect: vec4<f32>, ext: vec4<f32>, exps: vec4<f32>) -> f32 {
    let half = rect.zw * 0.5;
    let rel = p - (rect.xy + half);
    let i = corner_index(rel);
    let r = ext[i];
    let n = exps[i];
    let q = abs(rel) - half + vec2(r);
    if r > 0.0 && q.x > 0.0 && q.y > 0.0 {
        let u = max(q / r, vec2(1e-6));
        let un = pow(u, vec2(n));
        let norm = pow(un.x + un.y, 1.0 / n);
        let g = pow(u, vec2(n - 1.0));
        let grad = sqrt(g.x * g.x + g.y * g.y) / pow(norm, n - 1.0);
        return r * (norm - 1.0) / max(grad, 1e-6);
    }
    let m = max(q, vec2(0.0));
    return min(max(q.x, q.y), 0.0) + sqrt(m.x * m.x + m.y * m.y) - r;
}

fn coverage(d: f32, ppu: f32) -> f32 {
    return clamp(0.5 - d * ppu, 0.0, 1.0);
}

fn clip_coverage(px: vec2<f32>, clip: vec4<f32>) -> f32 {
    return clamp(px.x - clip.x + 0.5, 0.0, 1.0)
        * clamp(clip.z - px.x + 0.5, 0.0, 1.0)
        * clamp(px.y - clip.y + 0.5, 0.0, 1.0)
        * clamp(clip.w - px.y + 0.5, 0.0, 1.0);
}

fn erf(x: f32) -> f32 {
    let s = sign(x);
    let a = abs(x);
    let t = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    let t2 = t * t;
    return s - s / (t2 * t2);
}

fn gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (2.5066283 * sigma);
}

fn shadow_x(x: f32, y: f32, sigma: f32, r: f32, half: vec2<f32>) -> f32 {
    let delta = min(half.y - r - abs(y), 0.0);
    let curved = half.x - r + sqrt(max(r * r - delta * delta, 0.0));
    let k = 0.70710678 / sigma;
    let lo = 0.5 + 0.5 * erf((x - curved) * k);
    let hi = 0.5 + 0.5 * erf((x + curved) * k);
    return hi - lo;
}

fn box_shadow(p: vec2<f32>, half: vec2<f32>, sigma: f32, r: f32) -> f32 {
    let low = p.y - half.y;
    let high = p.y + half.y;
    let start = clamp(-3.0 * sigma, low, high);
    let end = clamp(3.0 * sigma, low, high);
    let step = (end - start) * 0.25;
    var y = start + step * 0.5;
    var v = 0.0;
    for (var i = 0; i < 4; i++) {
        v += shadow_x(p.x, p.y - y, sigma, r, half) * gaussian(y, sigma) * step;
        y += step;
    }
    return v;
}

fn circle_equivalent(ext: f32, n: f32) -> f32 {
    return ext * (1.0 - exp2(-1.0 / n)) / 0.2928932;
}

fn linear_to_srgb(x: f32) -> f32 {
    let c = clamp(x, 0.0, 1.0);
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn oklab_to_srgb(lab: vec3<f32>) -> vec3<f32> {
    let l_ = lab.x + 0.39633778 * lab.y + 0.21580376 * lab.z;
    let m_ = lab.x - 0.105561346 * lab.y - 0.06385417 * lab.z;
    let s_ = lab.x - 0.08948418 * lab.y - 1.2914855 * lab.z;
    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;
    let r = 4.0767417 * l3 - 3.3077116 * m3 + 0.23096994 * s3;
    let g = -1.268438 * l3 + 2.6097574 * m3 - 0.34131938 * s3;
    let b = -0.0041960863 * l3 - 0.7034186 * m3 + 1.7076147 * s3;
    return vec3(linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b));
}

fn local_pos(prop: Property, px: vec2<f32>) -> vec2<f32> {
    let s = px / globals.scale;
    return vec2(
        prop.inverse.x * s.x + prop.inverse.z * s.y + prop.offset.z,
        prop.inverse.y * s.x + prop.inverse.w * s.y + prop.offset.w,
    );
}

// Opacity times the clip's coverage.
fn visibility(prop: Property, px: vec2<f32>) -> f32 {
    return prop.opacity * clip_coverage(px, prop.clip);
}

fn shade_shadow(sh: ShadowRecord, px: vec2<f32>) -> vec4<f32> {
    let prop = properties[sh.prop];
    let p = local_pos(prop, px);
    let spread = sh.shadow.z;
    let sigma = sh.shadow.w;
    let half = max(sh.rect.zw * 0.5 + vec2(spread), vec2(0.0));
    let center = sh.rect.xy + sh.rect.zw * 0.5 + sh.shadow.xy;
    let rel = p - center;
    let i = corner_index(rel);
    let r = min(max(circle_equivalent(sh.ext[i], sh.exps[i]) + spread, 0.0), min(half.x, half.y));
    var a = box_shadow(rel, half, sigma, r);
    a *= 1.0 - coverage(sd_shape(p, sh.rect, sh.ext, sh.exps), prop.ppu);
    var c = sh.color * a;
    let n = bayer(px) / 255.0 * min(c.w, 1.0);
    c = vec4(c.xyz + vec3(n), c.w);
    return c * visibility(prop, px);
}

fn stop_color(g: GradientRecord, i: u32) -> vec4<f32> {
    var c: vec4<f32>;
    switch i {
        case 0u: { c = g.stop0; }
        case 1u: { c = g.stop1; }
        case 2u: { c = g.stop2; }
        default: { c = g.stop3; }
    }
    return vec4(c.xyz * c.w, c.w);
}

fn gradient(g: GradientRecord, flags: u32, p: vec2<f32>) -> vec4<f32> {
    let n = min(flags >> 8u, 4u);
    let geo = g.geometry;
    var t: f32;
    if (flags & 0xffu) == FILL_LINEAR {
        let d = geo.zw - geo.xy;
        let len2 = max(d.x * d.x + d.y * d.y, 1e-12);
        t = ((p.x - geo.x) * d.x + (p.y - geo.y) * d.y) / len2;
    } else {
        let u = (p - geo.xy) / geo.zw;
        t = sqrt(u.x * u.x + u.y * u.y);
    }
    let offs = g.offsets;
    var lab = stop_color(g, 0u);
    if n > 1u && t > offs[0] {
        lab = stop_color(g, n - 1u);
        for (var i = 0u; i < n - 1u; i++) {
            if t < offs[i + 1u] {
                let f = clamp((t - offs[i]) / max(offs[i + 1u] - offs[i], 1e-6), 0.0, 1.0);
                lab = mix(stop_color(g, i), stop_color(g, i + 1u), f);
                break;
            }
        }
    }
    let alpha = lab.w;
    if alpha <= 0.0 {
        return vec4(0.0);
    }
    let rgb = oklab_to_srgb(lab.xyz / alpha);
    return vec4(rgb * alpha, alpha);
}

fn shade_quad(q: QuadRecord, px: vec2<f32>) -> vec4<f32> {
    let prop = properties[q.prop];
    let p = local_pos(prop, px);
    let ppu = prop.ppu;
    let d = sd_shape(p, q.rect, q.ext, q.exps);
    let shape = coverage(d, ppu);
    let kind = q.flags & 0xffu;
    var fill = vec4(0.0);
    if kind == FILL_SOLID {
        fill = q.fill;
    } else if kind >= FILL_LINEAR {
        fill = gradient(load_gradient(q.effect), q.flags, p);
    }
    var c = fill * shape;
    let bw = q.border_width;
    if bw > 0.0 {
        let bcov = shape - coverage(d + bw, ppu);
        let b = q.border;
        let k = shape - bcov * b.w;
        c = b * bcov + fill * k;
    }
    if kind >= FILL_LINEAR {
        let n = bayer(px) / 255.0 * c.w;
        c = vec4(c.xyz + vec3(n), c.w);
    }
    return c * visibility(prop, px);
}

// The stroke covering this pixel, if this segment's box is the one that
// draws it: the segment nearest to the pixel, the lowest index among equals.
// Every box over a pixel computes the same nearest segment, so it is drawn
// once.
fn shade_curve(c: CurveRecord, segment: u32, px: vec2<f32>) -> vec4<f32> {
    let prop = properties[c.prop];
    let p = local_pos(prop, px);
    let segments = clamp(c.segments, 1u, MAX_CURVE_SEGMENTS);
    let n = f32(segments);
    // Beyond the stroke's reach of its own segment a box has nothing to
    // draw: either nothing is covered here or a nearer segment's box draws
    // it. Saves the search for the corners of the boxes.
    let own = segment_distance(p, curve_point(c, f32(segment) / n), curve_point(c, f32(segment + 1u) / n));
    if own >= c.half_width + CLAIM_MARGIN_PX / prop.ppu {
        discard;
    }
    // Only segments within the window can be as near as that
    // (curve::search_window).
    let window = min(c.window, segments);
    let first = segment - min(segment, window);
    let end = min(segment + window + 1u, segments);
    var best = 1e30;
    var owner = 0u;
    var a = curve_point(c, f32(first) / n);
    for (var k = first; k < end; k++) {
        let b = curve_point(c, f32(k + 1u) / n);
        let d = segment_distance(p, a, b);
        if d < best {
            best = d;
            owner = k;
        }
        a = b;
    }
    if owner != segment {
        discard;
    }
    let cov = coverage(best - c.half_width, prop.ppu);
    return c.color * (cov * visibility(prop, px));
}

fn glyph_alpha(a: f32, l: f32, inv_g: f32) -> f32 {
    let light = pow(a, inv_g);
    let dark = 1.0 - pow(1.0 - a, inv_g);
    return dark + (light - dark) * l;
}

fn atlas_tap(x: f32, y: f32, t: vec4<f32>) -> f32 {
    if x < t.x || y < t.y || x >= t.x + t.z || y >= t.y + t.w {
        return 0.0;
    }
    let size = vec2<i32>(textureDimensions(glyph_atlas));
    let c = vec2<i32>(i32(x), i32(y));
    if c.x >= size.x || c.y >= size.y {
        return 0.0;
    }
    return textureLoad(glyph_atlas, c, 0).x;
}

fn shade_sprite(s: SpriteRecord, px: vec2<f32>) -> vec4<f32> {
    let prop = properties[s.prop];
    let p = local_pos(prop, px);
    let t = vec4<f32>(vec4<u32>(
        s.texels.x & 0xffffu, s.texels.x >> 16u, s.texels.y & 0xffffu, s.texels.y >> 16u,
    ));
    let tx = t.x + (p.x - s.rect.x) / s.rect.z * t.z - 0.5;
    let ty = t.y + (p.y - s.rect.y) / s.rect.w * t.w - 0.5;
    let x0 = floor(tx);
    let y0 = floor(ty);
    let fx = tx - x0;
    let fy = ty - y0;
    let a00 = atlas_tap(x0, y0, t);
    let a10 = atlas_tap(x0 + 1.0, y0, t);
    let a01 = atlas_tap(x0, y0 + 1.0, t);
    let a11 = atlas_tap(x0 + 1.0, y0 + 1.0, t);
    let top = a00 + (a10 - a00) * fx;
    let bottom = a01 + (a11 - a01) * fx;
    let cov = clamp(top + (bottom - top) * fy, 0.0, 1.0);
    let a = glyph_alpha(cov, s.lightness, INV_TEXT_GAMMA) * visibility(prop, px);
    return s.color * a;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let at = in.instance & OFFSET_MASK;
    let px = in.position.xy;
    switch in.instance >> OFFSET_BITS {
        case FAMILY_SPRITE: { return shade_sprite(load_sprite(at), px); }
        case FAMILY_QUAD: { return shade_quad(load_quad(at), px); }
        case FAMILY_SHADOW: { return shade_shadow(load_shadow(at), px); }
        case FAMILY_CURVE: { return shade_curve(load_curve(at), in.segment, px); }
        default: { return vec4(0.0); }
    }
}

fn saturate_css(c: vec3<f32>, s: f32) -> vec3<f32> {
    let r = (0.213 + 0.787 * s) * c.x + (0.715 - 0.715 * s) * c.y + (0.072 - 0.072 * s) * c.z;
    let g = (0.213 - 0.213 * s) * c.x + (0.715 + 0.285 * s) * c.y + (0.072 - 0.072 * s) * c.z;
    let b = (0.213 - 0.213 * s) * c.x + (0.715 - 0.715 * s) * c.y + (0.072 + 0.928 * s) * c.z;
    return clamp(vec3(r, g, b), vec3(0.0), vec3(1.0));
}

@fragment
fn fs_backdrop(in: VsOut) -> @location(0) vec4<f32> {
    let b = load_backdrop(in.instance & OFFSET_MASK);
    let prop = properties[b.prop];
    let px = in.position.xy;
    let p = local_pos(prop, px);
    let ppu = prop.ppu;
    let d = sd_shape(p, b.rect, b.ext, b.exps);
    let shape = coverage(d, ppu);
    let tint = b.tint;
    let hw = b.params.x;
    let hk = b.params.y;
    let sat = b.params.z;
    let grain = b.params.w;
    // Texture coordinates of the half-resolution blur level.
    let half_size = max(ceil(globals.viewport * 0.5), vec2(1.0));
    let uv_scale = vec2(1.0) / (2.0 * half_size);
    // Sample unconditionally (uniform control flow), use only with blur.
    let blurred = textureSampleLevel(blur_tex, blur_sampler, px * uv_scale, 0.0).xyz;
    var base = tint;
    if b.has_blur != 0u {
        let c = saturate_css(blurred, sat);
        base = vec4(c * (1.0 - tint.w) + tint.xyz, 1.0);
    }
    var rim = 0.0;
    if hw > 0.0 {
        rim = shape - coverage(d + hw, ppu);
        if hk > 0.0 {
            let fade = max(max(b.ext.x, b.ext.y), 4.0);
            rim *= clamp(1.0 - (p.y - b.rect.y) / fade, 0.0, 1.0);
        }
    }
    let h = b.rim;
    var c = h * rim + base * (shape - rim * h.w);
    let n = (bayer(px) / 255.0 + noise(px) * grain) * c.w;
    c = vec4(c.xyz + vec3(n), c.w);
    return c * visibility(prop, px);
}
