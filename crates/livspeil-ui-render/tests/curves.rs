// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The curve primitive in the CPU reference rasterizer (the GPU is held to
//! the same pictures in `gpu_vs_cpu.rs`).

use livspeil_ui_render::{
    Affine, Color, Curve, DisplayList, FrameOptions, Quad, Rect, RgbaImage, encode, render_cpu,
};

fn render(list: &DisplayList) -> RgbaImage {
    let options = FrameOptions {
        width: 64,
        height: 32,
        scale: 1.0,
        clear: Color::BLACK,
    };
    render_cpu(&encode(list, options).unwrap())
}

/// Equal up to one level of rounding (half coverage lands on `x.5`).
#[track_caller]
fn assert_levels(actual: &[u8], expected: &[u8], what: &str) {
    assert!(
        actual.len() == expected.len()
            && actual
                .iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(*b) <= 1),
        "{what}: {actual:?}, expected {expected:?}"
    );
}

fn one(curve: Curve) -> RgbaImage {
    let mut list = DisplayList::new();
    list.add(curve);
    render(&list)
}

/// A straight horizontal stroke along the pixel centers of row `y - 0.5`.
fn level(y: f32, width: f32) -> Curve {
    Curve::new(
        [[8.0, y], [24.0, y], [40.0, y], [56.0, y]],
        width,
        Color::WHITE,
    )
}

fn column(img: &RgbaImage, x: u32) -> Vec<u8> {
    (0..img.height)
        .map(|y| img.pixel(x, y).unwrap()[0])
        .collect()
}

fn max_delta(a: &RgbaImage, b: &RgbaImage) -> u8 {
    a.data
        .iter()
        .zip(&b.data)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

#[test]
fn a_stroke_is_antialiased_with_round_caps() {
    let img = one(level(10.5, 3.0));
    let c = column(&img, 30);
    assert_levels(&c[8..13], &[0, 255, 255, 255, 0], "on pixel centers");
    // A stroke between two pixel rows is half in each.
    let c = column(&one(level(10.0, 1.0)), 30);
    assert_levels(&c[8..12], &[0, 128, 128, 0], "between rows");
    // The cap reaches half the width past the end, and is round.
    assert!(img.pixel(7, 10).unwrap()[0] > 0);
    assert_eq!(img.pixel(5, 10).unwrap()[0], 0);
    assert!(img.pixel(6, 9).unwrap()[0] < img.pixel(7, 10).unwrap()[0]);
}

#[test]
fn strokes_thinner_than_a_pixel_fade_instead_of_vanishing() {
    for (width, value) in [(1.0, 255), (0.5, 128), (0.25, 64), (0.05, 13)] {
        let c = column(&one(level(10.5, width)), 30);
        assert_levels(&c[9..12], &[0, value, 0], &format!("width {width}"));
    }
}

#[test]
fn the_transform_applies_to_points_and_width() {
    let points = [[2.0, 3.0], [14.0, 3.0], [14.0, 12.0], [28.0, 12.0]];
    let transformed = one(Curve {
        transform: Affine::translate(4.0, 2.0).then_after(Affine::scale(2.0, 2.0)),
        ..Curve::new(points, 2.0, Color::WHITE)
    });
    let moved = points.map(|[x, y]| [x * 2.0 + 4.0, y * 2.0 + 2.0]);
    let plain = one(Curve::new(moved, 4.0, Color::WHITE));
    assert!(max_delta(&transformed, &plain) <= 1);
}

#[test]
fn clip_and_opacity() {
    let clip = Rect::new(20.0, 0.0, 10.0, 32.0);
    let img = one(Curve {
        clip: Some(clip),
        opacity: 0.5,
        ..level(10.5, 3.0)
    });
    for x in 0..64 {
        let expected = if (20..30).contains(&x) { 128 } else { 0 };
        assert_levels(
            &[img.pixel(x, 10).unwrap()[0]],
            &[expected],
            &format!("x {x}"),
        );
    }
}

#[test]
fn painter_order_holds_across_families() {
    let mut list = DisplayList::new();
    list.add(Quad::new(Rect::new(0.0, 0.0, 40.0, 32.0), Color::WHITE));
    list.add(Curve {
        color: Color::BLACK,
        ..level(10.5, 3.0)
    });
    list.add(Quad::new(
        Rect::new(30.0, 0.0, 34.0, 32.0),
        Color::oklab(0.6, 0.0, 0.0),
    ));
    let grey = render(&list).pixel(50, 0).unwrap()[0];
    let img = render(&list);
    // Curve over the first quad, under the second.
    assert_eq!(img.pixel(15, 10).unwrap()[0], 0);
    assert_eq!(img.pixel(15, 5).unwrap()[0], 255);
    assert_eq!(img.pixel(35, 10).unwrap()[0], grey);
    // A lower layer wins over insertion order.
    list.push(-1, level(10.5, 3.0));
    assert_eq!(render(&list).pixel(35, 10).unwrap()[0], grey);
}
