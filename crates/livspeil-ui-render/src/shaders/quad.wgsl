// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

// Instanced SDF quads, analytic shadows and backdrop composites.
// Mirrored by src/shade.rs (the CPU reference); keep both in step.

struct Instance {
    rect: vec4<f32>,
    xform: vec4<f32>,
    xform2: vec4<f32>,   // e, f, px_per_unit, kind (0 quad, 1 shadow, 2 backdrop)
    inv: vec4<f32>,
    inv2: vec4<f32>,     // e, f, opacity, dither
    radii: vec4<f32>,    // corner extents tl tr br bl
    exps: vec4<f32>,     // corner exponents
    clip: vec4<f32>,     // physical pixels x0 y0 x1 y1
    bounds: vec4<f32>,   // local x0 y0 x1 y1
    grad: vec4<f32>,
    fill: vec4<f32>,     // kind (0 none, 1 solid, 2 linear, 3 radial), n_stops
    stop_offsets: vec4<f32>,
    stop0: vec4<f32>,
    stop1: vec4<f32>,
    stop2: vec4<f32>,
    stop3: vec4<f32>,
    border_color: vec4<f32>,
    border: vec4<f32>,
    shadow: vec4<f32>,   // dx dy spread sigma
    shadow_color: vec4<f32>,
}

struct Globals {
    viewport: vec2<f32>,
    scale: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(1) @binding(0) var blur_tex: texture_2d<f32>;
@group(1) @binding(1) var blur_sampler: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) index: u32,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let inst = instances[ii];
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(0.0, 1.0), vec2(1.0, 0.0), vec2(1.0, 1.0),
    );
    let t = corners[vi];
    let local = mix(inst.bounds.xy, inst.bounds.zw, t);
    let screen = vec2(
        inst.xform.x * local.x + inst.xform.z * local.y + inst.xform2.x,
        inst.xform.y * local.x + inst.xform.w * local.y + inst.xform2.y,
    );
    let px = screen * globals.scale;
    let ndc = vec2(px.x / globals.viewport.x * 2.0 - 1.0, 1.0 - px.y / globals.viewport.y * 2.0);
    var out: VsOut;
    out.position = vec4(ndc, 0.0, 1.0);
    out.index = ii;
    return out;
}

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

fn local_pos(inst: Instance, px: vec2<f32>) -> vec2<f32> {
    let s = px / globals.scale;
    return vec2(
        inst.inv.x * s.x + inst.inv.z * s.y + inst.inv2.x,
        inst.inv.y * s.x + inst.inv.w * s.y + inst.inv2.y,
    );
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

fn shade_shadow(inst: Instance, p: vec2<f32>, ppu: f32) -> vec4<f32> {
    let spread = inst.shadow.z;
    let sigma = inst.shadow.w;
    let half = max(inst.rect.zw * 0.5 + vec2(spread), vec2(0.0));
    let center = inst.rect.xy + inst.rect.zw * 0.5 + inst.shadow.xy;
    let rel = p - center;
    let i = corner_index(rel);
    let r = min(max(circle_equivalent(inst.radii[i], inst.exps[i]) + spread, 0.0), min(half.x, half.y));
    var a = box_shadow(rel, half, sigma, r);
    a *= 1.0 - coverage(sd_shape(p, inst.rect, inst.radii, inst.exps), ppu);
    return inst.shadow_color * a;
}

fn stop_color(inst: Instance, i: u32) -> vec4<f32> {
    var c: vec4<f32>;
    switch i {
        case 0u: { c = inst.stop0; }
        case 1u: { c = inst.stop1; }
        case 2u: { c = inst.stop2; }
        default: { c = inst.stop3; }
    }
    return vec4(c.xyz * c.w, c.w);
}

fn gradient(inst: Instance, p: vec2<f32>) -> vec4<f32> {
    let n = u32(inst.fill.y);
    let g = inst.grad;
    var t: f32;
    if inst.fill.x == 2.0 {
        let d = g.zw - g.xy;
        let len2 = max(d.x * d.x + d.y * d.y, 1e-12);
        t = ((p.x - g.x) * d.x + (p.y - g.y) * d.y) / len2;
    } else {
        let u = (p - g.xy) / g.zw;
        t = sqrt(u.x * u.x + u.y * u.y);
    }
    let offs = inst.stop_offsets;
    var lab = stop_color(inst, 0u);
    if n > 1u && t > offs[0] {
        lab = stop_color(inst, n - 1u);
        for (var i = 0u; i < n - 1u; i++) {
            if t < offs[i + 1u] {
                let f = clamp((t - offs[i]) / max(offs[i + 1u] - offs[i], 1e-6), 0.0, 1.0);
                lab = mix(stop_color(inst, i), stop_color(inst, i + 1u), f);
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

fn shade_quad(inst: Instance, p: vec2<f32>, ppu: f32) -> vec4<f32> {
    let d = sd_shape(p, inst.rect, inst.radii, inst.exps);
    let shape = coverage(d, ppu);
    let fk = inst.fill.x;
    var fill = vec4(0.0);
    if fk == 1.0 {
        fill = inst.stop0;
    } else if fk >= 2.0 {
        fill = gradient(inst, p);
    }
    let bw = inst.border.x;
    if bw > 0.0 {
        let bcov = shape - coverage(d + bw, ppu);
        let b = inst.border_color;
        let k = shape - bcov * b.w;
        return b * bcov + fill * k;
    }
    return fill * shape;
}

@fragment
fn fs_quad(in: VsOut) -> @location(0) vec4<f32> {
    let inst = instances[in.index];
    let px = in.position.xy;
    let p = local_pos(inst, px);
    let ppu = inst.xform2.z;
    let kind = inst.xform2.w;
    var c: vec4<f32>;
    if kind == 1.0 {
        c = shade_shadow(inst, p, ppu);
    } else {
        c = shade_quad(inst, p, ppu);
    }
    let dither = inst.inv2.w;
    if inst.fill.x >= 2.0 && kind == 0.0 {
        let n = bayer(px) * dither / 255.0 * c.w;
        c = vec4(c.xyz + vec3(n), c.w);
    } else if kind == 1.0 {
        let n = bayer(px) * dither / 255.0 * min(c.w, 1.0);
        c = vec4(c.xyz + vec3(n), c.w);
    }
    return c * (inst.inv2.z * clip_coverage(px, inst.clip));
}

fn saturate_css(c: vec3<f32>, s: f32) -> vec3<f32> {
    let r = (0.213 + 0.787 * s) * c.x + (0.715 - 0.715 * s) * c.y + (0.072 - 0.072 * s) * c.z;
    let g = (0.213 - 0.213 * s) * c.x + (0.715 + 0.285 * s) * c.y + (0.072 - 0.072 * s) * c.z;
    let b = (0.213 - 0.213 * s) * c.x + (0.715 - 0.715 * s) * c.y + (0.072 + 0.928 * s) * c.z;
    return clamp(vec3(r, g, b), vec3(0.0), vec3(1.0));
}

@fragment
fn fs_backdrop(in: VsOut) -> @location(0) vec4<f32> {
    let inst = instances[in.index];
    let px = in.position.xy;
    let p = local_pos(inst, px);
    let ppu = inst.xform2.z;
    let d = sd_shape(p, inst.rect, inst.radii, inst.exps);
    let shape = coverage(d, ppu);
    let tint = inst.stop0;
    let hw = inst.border.x;
    let hk = inst.border.y;
    let sat = inst.border.z;
    let grain = inst.border.w;
    // Sample unconditionally (uniform control flow), use only with blur.
    let blurred = textureSampleLevel(blur_tex, blur_sampler, px * inst.grad.xy, 0.0).xyz;
    var base = tint;
    if inst.grad.z > 0.0 {
        let c = saturate_css(blurred, sat);
        base = vec4(c * (1.0 - tint.w) + tint.xyz, 1.0);
    }
    var rim = 0.0;
    if hw > 0.0 {
        rim = shape - coverage(d + hw, ppu);
        if hk > 0.0 {
            let fade = max(max(inst.radii.x, inst.radii.y), 4.0);
            rim *= clamp(1.0 - (p.y - inst.rect.y) / fade, 0.0, 1.0);
        }
    }
    let h = inst.border_color;
    var c = h * rim + base * (shape - rim * h.w);
    let n = (bayer(px) * inst.inv2.w / 255.0 + noise(px) * grain) * c.w;
    c = vec4(c.xyz + vec3(n), c.w);
    return c * (inst.inv2.z * clip_coverage(px, inst.clip));
}
