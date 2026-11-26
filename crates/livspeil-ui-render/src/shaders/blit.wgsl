// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

// Copies an offscreen frame texture 1:1 into a render-only target (a
// surface texture that cannot be sampled, e.g. Metal drawables). No
// filtering, no blending: texels are copied bit for bit.

@group(0) @binding(0) var src: texture_2d<f32>;

@vertex
fn vs_blit(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    return vec4(x, y, 0.0, 1.0);
}

@fragment
fn fs_blit(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(src, vec2<i32>(pos.xy), 0);
}
