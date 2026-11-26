// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! GLSL ES 1.00 sources for the Floor's effects. The SDF, coverage and
//! shadow functions mirror `geometry.rs`.

/// Shared helpers, prepended to every shader.
const COMMON: &str = r#"
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

// radii: top-left, top-right, bottom-right, bottom-left (matches geometry.rs).
float sd_round_box(vec2 p, vec2 half_size, vec4 radii) {
    float r = p.x < 0.0 ? (p.y < 0.0 ? radii.x : radii.w)
                        : (p.y < 0.0 ? radii.y : radii.z);
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}
float coverage(float sdf) { return clamp(0.5 - sdf, 0.0, 1.0); }
float shadow_falloff(float d, float blur) {
    float sigma = max(blur * 0.5, 0.001);
    return 1.0 / (1.0 + exp(1.702 * d / sigma));
}
// Interleaved gradient noise: breaks up banding on dark gradients.
float dither(vec2 frag) {
    return fract(52.9829189 * fract(dot(frag, vec2(0.06711056, 0.00583715)))) - 0.5;
}
"#;

/// Pixel shader: two-layer soft shadow (elevation e3) of a rounded box.
/// `box_rect` is the outer rect relative to the drawn region.
pub fn shadow() -> String {
    format!(
        r#"{COMMON}
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
uniform vec4 box_rect;
uniform float radius;
uniform vec4 key;      // offset_y, blur, opacity, unused
uniform vec4 contact;  // offset_y, blur, opacity, unused
void main() {{
    vec2 p = v_coords * size;
    vec2 half_size = box_rect.zw * 0.5;
    vec2 c = box_rect.xy + half_size;
    float dk = sd_round_box(p - c - vec2(0.0, key.x), half_size, vec4(radius));
    float dc = sd_round_box(p - c - vec2(0.0, contact.x), half_size, vec4(radius));
    float a1 = key.z * shadow_falloff(dk, key.y);
    float a2 = contact.z * shadow_falloff(dc, contact.y);
    float a = (a1 + a2 * (1.0 - a1)) * alpha;
    gl_FragColor = vec4(0.0, 0.0, 0.0, a);
}}
"#
    )
}

/// Pixel shader: filled rounded box with a 1 px inner hairline.
pub fn rounded_box() -> String {
    format!(
        r#"{COMMON}
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
uniform vec4 radii;
uniform vec4 fill;    // straight (non-premultiplied) RGBA
uniform vec4 hairline;
void main() {{
    vec2 p = v_coords * size - size * 0.5;
    float d = sd_round_box(p, size * 0.5, radii);
    float cov = coverage(d);
    float inner = coverage(d + 1.0);
    vec4 c = mix(hairline, fill, inner);
    float a = c.a * cov * alpha;
    gl_FragColor = vec4(c.rgb * a, a);
}}
"#
    )
}

/// Pixel shader: neutral vertical gradient with dithering (the desktop
/// background of the spike).
pub fn background() -> String {
    format!(
        r#"{COMMON}
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
uniform vec3 top;
uniform vec3 bottom;
void main() {{
    vec3 c = mix(top, bottom, v_coords.y);
    c += dither(v_coords * size) / 255.0;
    gl_FragColor = vec4(c, 1.0) * alpha;
}}
"#
    )
}

/// Texture shader defines handled the way Smithay's default shader does.
const TEX_HEADER: &str = r#"#version 100
//_DEFINES_
#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif
"#;

fn tex_sampler() -> &'static str {
    r#"
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
vec4 sample_tex(vec2 uv) {
    vec4 color = texture2D(tex, uv);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    return color;
}
"#
}

/// Texture shader: clip a client surface to a rounded rectangle given in
/// buffer pixels (the window geometry), anti-aliased.
pub fn clip_surface() -> String {
    format!(
        r#"{TEX_HEADER}{COMMON}{}
uniform vec2 buf_size;
uniform vec4 geo;    // x, y, w, h of the window geometry in buffer pixels
uniform vec4 radii;
void main() {{
    vec2 p = v_coords * buf_size - (geo.xy + geo.zw * 0.5);
    float cov = coverage(sd_round_box(p, geo.zw * 0.5, radii));
    gl_FragColor = sample_tex(v_coords) * (alpha * cov);
}}
"#,
        tex_sampler()
    )
}

/// Dual-Kawase downsample (5 taps).
pub fn kawase_down() -> String {
    format!(
        r#"{TEX_HEADER}{COMMON}{}
uniform vec2 halfpixel;
uniform float offset;
void main() {{
    vec2 o = halfpixel * offset;
    vec4 sum = sample_tex(v_coords) * 4.0;
    sum += sample_tex(v_coords - o);
    sum += sample_tex(v_coords + o);
    sum += sample_tex(v_coords + vec2(o.x, -o.y));
    sum += sample_tex(v_coords - vec2(o.x, -o.y));
    gl_FragColor = sum / 8.0 * alpha;
}}
"#,
        tex_sampler()
    )
}

/// Dual-Kawase upsample (8 taps).
pub fn kawase_up() -> String {
    format!(
        r#"{TEX_HEADER}{COMMON}{}
uniform vec2 halfpixel;
uniform float offset;
void main() {{
    vec2 o = halfpixel * offset;
    vec4 sum = sample_tex(v_coords + vec2(-o.x * 2.0, 0.0));
    sum += sample_tex(v_coords + vec2(-o.x, o.y)) * 2.0;
    sum += sample_tex(v_coords + vec2(0.0, o.y * 2.0));
    sum += sample_tex(v_coords + vec2(o.x, o.y)) * 2.0;
    sum += sample_tex(v_coords + vec2(o.x * 2.0, 0.0));
    sum += sample_tex(v_coords + vec2(o.x, -o.y)) * 2.0;
    sum += sample_tex(v_coords + vec2(0.0, -o.y * 2.0));
    sum += sample_tex(v_coords + vec2(-o.x, -o.y)) * 2.0;
    gl_FragColor = sum / 12.0 * alpha;
}}
"#,
        tex_sampler()
    )
}

/// "Frost" panel: the (already blurred, panel sized) texture, saturated,
/// tinted with a neutral surface colour, with a top rim highlight, clipped
/// to a rounded rectangle. Matches the Frost role of the UI spec.
pub fn frost() -> String {
    format!(
        r#"{TEX_HEADER}{COMMON}{}
uniform vec2 size;
uniform float radius;
uniform vec4 tint_color;  // rgb + opacity of the tint over the blur
uniform float saturation; // 1.3 in the spec
uniform float rim;        // top highlight strength (white), e.g. 0.08
void main() {{
    vec2 p = v_coords * size;
    float d = sd_round_box(p - size * 0.5, size * 0.5, vec4(radius));
    float cov = coverage(d);
    vec3 c = sample_tex(v_coords).rgb;
    float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = mix(vec3(luma), c, saturation);
    c = mix(c, tint_color.rgb, tint_color.a);
    // Rim: a 1 px lighter edge that fades over the first 2 px inside.
    float edge = clamp(1.0 - (-d) / 2.0, 0.0, 1.0) * (1.0 - smoothstep(0.0, 14.0, p.y));
    c += vec3(rim * edge);
    c += dither(v_coords * size) / 255.0;
    gl_FragColor = vec4(c, 1.0) * (alpha * cov);
}}
"#,
        tex_sampler()
    )
}

/// Plain translucent panel for the no-blur path: tint only.
pub fn tint_panel() -> String {
    format!(
        r#"{COMMON}
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
uniform float radius;
uniform vec4 tint_color;
void main() {{
    float cov = coverage(sd_round_box(v_coords * size - size * 0.5, size * 0.5, vec4(radius)));
    float a = tint_color.a * cov * alpha;
    gl_FragColor = vec4(tint_color.rgb * a, a);
}}
"#
    )
}
