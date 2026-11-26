// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The embedded set: complete, parseable and visible at every size.

use std::collections::BTreeSet;
use std::path::Path;

use livspeil_ui_icons::{Command, IconId, IconRasterizer, VIEW_BOX, parse_svg};

#[test]
fn registry_matches_the_vendored_directory() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/boxicons");
    let on_disk: BTreeSet<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".svg").map(str::to_owned)
        })
        .collect();
    let names: Vec<&str> = IconId::all().map(IconId::name).collect();
    assert_eq!(names.len(), 49);
    // Sorted (name lookup is a binary search) and the same set.
    assert!(names.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(on_disk, names.iter().map(|n| (*n).to_owned()).collect());
    for id in IconId::all() {
        assert_eq!(IconId::from_name(id.name()), Some(id));
    }
    assert_eq!(IconId::from_name("bx-trash"), None);
    assert_eq!(IconId::from_name(""), None);
}

#[test]
fn every_icon_parses_inside_its_view_box() {
    for id in IconId::all() {
        let shapes = parse_svg(id.svg()).unwrap_or_else(|e| panic!("{}: {e}", id.name()));
        for command in shapes.iter().flatten() {
            let end = match command {
                Command::MoveTo(p) | Command::LineTo(p) => *p,
                Command::QuadTo(_, p) | Command::CurveTo(_, _, p) => *p,
                Command::Close => continue,
            };
            // A wrong relative or smooth command flies off the canvas.
            assert!(
                (-0.5..=VIEW_BOX + 0.5).contains(&end.x)
                    && (-0.5..=VIEW_BOX + 0.5).contains(&end.y),
                "{}: point {end:?} outside the view box",
                id.name()
            );
        }
    }
}

#[test]
fn every_icon_rasterizes_at_every_size_and_scale() {
    let mut r = IconRasterizer::new();
    for id in IconId::all() {
        for size in [14.0, 16.0, 20.0, 24.0] {
            for scale in [1.0, 2.0] {
                let px: f32 = size * scale;
                let mask = r.rasterize(id, px).unwrap();
                assert_eq!(mask.side, px as u32);
                assert_eq!(mask.data.len(), (mask.side * mask.side) as usize);
                let ink: u32 = mask.data.iter().map(|&a| u32::from(a)).sum();
                let covered = ink as f32 / (255.0 * px * px);
                assert!(
                    (0.05..0.95).contains(&covered),
                    "{} at {px}px covers {covered}",
                    id.name()
                );
                assert!(mask.data.iter().any(|&a| a > 200), "{}", id.name());
            }
        }
    }
}

#[test]
fn odd_sizes_are_rounded_up_and_clamped() {
    let mut r = IconRasterizer::new();
    let id = IconId::from_name("bx-copy").unwrap();
    assert_eq!(r.rasterize(id, 17.5).unwrap().side, 18);
    assert_eq!(r.rasterize(id, 0.0).unwrap().side, 1);
    assert_eq!(r.rasterize(id, f32::NAN).unwrap().side, 1);
    assert_eq!(r.rasterize(id, 1e9).unwrap().side, 512);
}

#[test]
fn rect_and_path_are_both_drawn() {
    // bx-copy is a rounded rect (8..22 x 2..16) and a path below and left.
    let mask = IconRasterizer::new()
        .rasterize(IconId::from_name("bx-copy").unwrap(), 24.0)
        .unwrap();
    let at = |x: u32, y: u32| mask.data[(y * mask.side + x) as usize];
    assert_eq!(at(15, 9), 255, "inside the rect");
    assert_eq!(at(4, 15), 255, "inside the path");
    assert_eq!(at(3, 3), 0, "empty top left");
    assert_eq!(at(21, 21), 0, "empty bottom right");
    assert!(at(8, 2) < 128, "the rect's corner is rounded");
}
