// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

// Dual-Kawase blur passes (Bjorge, "Bandwidth-efficient rendering",
// SIGGRAPH 2015). Mirrored by the blur in src/cpu.rs.

struct Pass {
    // uv = screen_px * src_uv_scale
    src_uv_scale: vec2<f32>,
    // half a source texel in uv, times the offset
    half_texel: vec2<f32>,
    // screen pixels per destination texel (2^level)
    dst_scale: f32,
    _pad0: f32,
    _pad1: vec2<f32>,
}

@group(0) @binding(0) var<uniform> pass_: Pass;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    return vec4(x, y, 0.0, 1.0);
}

fn tap(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src, samp, uv, 0.0);
}

@fragment
fn fs_down(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy * pass_.dst_scale * pass_.src_uv_scale;
    let h = pass_.half_texel;
    var sum = tap(uv) * 4.0;
    sum += tap(uv - h);
    sum += tap(uv + h);
    sum += tap(uv + vec2(h.x, -h.y));
    sum += tap(uv - vec2(h.x, -h.y));
    return sum / 8.0;
}

@fragment
fn fs_up(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy * pass_.dst_scale * pass_.src_uv_scale;
    let h = pass_.half_texel;
    var sum = tap(uv + vec2(-h.x * 2.0, 0.0));
    sum += tap(uv + vec2(-h.x, h.y)) * 2.0;
    sum += tap(uv + vec2(0.0, h.y * 2.0));
    sum += tap(uv + vec2(h.x, h.y)) * 2.0;
    sum += tap(uv + vec2(h.x * 2.0, 0.0));
    sum += tap(uv + vec2(h.x, -h.y)) * 2.0;
    sum += tap(uv + vec2(0.0, -h.y * 2.0));
    sum += tap(uv + vec2(-h.x, -h.y)) * 2.0;
    return sum / 12.0;
}
