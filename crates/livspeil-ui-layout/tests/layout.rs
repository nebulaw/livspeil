// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Taffy snapshots, relayout caching, dock splitters and canvas culling.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use livspeil_testkit::assert_golden;
use livspeil_ui_layout::style::{
    AlignItems, Dimension, Display, FlexDirection, JustifyContent, LengthPercentage, fr, length,
};
use livspeil_ui_layout::{
    Canvas, CanvasIndex, Dock, DockEdge, DockSide, LayoutNode, LayoutTree, Measure, Rect, Size,
    Style, Vec2, taffy,
};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn dump(tree: &LayoutTree, nodes: &[(&str, usize, LayoutNode)]) -> String {
    let mut out = String::new();
    for (name, depth, node) in nodes {
        let r = tree.rect(*node).unwrap();
        writeln!(out, "{}{name} {r}", "  ".repeat(*depth)).unwrap();
    }
    out
}

fn sized(w: f32, h: f32) -> Style {
    Style {
        size: taffy::Size {
            width: Dimension::length(w),
            height: Dimension::length(h),
        },
        ..Style::default()
    }
}

fn text(chars: u32) -> Measure {
    Measure::TextPlaceholder {
        chars,
        font_size: 13.0,
        line_height: 16.0,
    }
}

#[test]
fn flex_snapshot() {
    let mut t = LayoutTree::new();
    let pad = |v: f32| taffy::Rect {
        left: LengthPercentage::length(v),
        right: LengthPercentage::length(v),
        top: LengthPercentage::length(v),
        bottom: LengthPercentage::length(v),
    };
    let sidebar = t.create(sized(200.0, 0.0), Measure::None).unwrap();
    let sidebar_style = Style {
        size: taffy::Size {
            width: Dimension::length(200.0),
            height: Dimension::auto(),
        },
        flex_direction: FlexDirection::Column,
        padding: pad(8.0),
        gap: taffy::Size {
            width: length(0.0),
            height: length(4.0),
        },
        ..Style::default()
    };
    t.set_style(sidebar, &sidebar_style).unwrap();
    let rows: Vec<LayoutNode> = (0..3)
        .map(|_| t.create(sized(0.0, 32.0), Measure::None).unwrap())
        .collect();
    for r in &rows {
        t.set_style(
            *r,
            &Style {
                size: taffy::Size {
                    width: Dimension::auto(),
                    height: Dimension::length(32.0),
                },
                ..Style::default()
            },
        )
        .unwrap();
    }
    t.set_children(sidebar, &rows).unwrap();

    let label = t.create(Style::default(), text(12)).unwrap();
    let button = t
        .create(
            Style {
                padding: pad(6.0),
                align_items: AlignItems::CENTER,
                ..Style::default()
            },
            Measure::None,
        )
        .unwrap();
    t.set_children(button, &[label]).unwrap();
    let content = t
        .create(
            Style {
                flex_grow: 1.0,
                justify_content: JustifyContent::CENTER,
                align_items: AlignItems::CENTER,
                ..Style::default()
            },
            Measure::None,
        )
        .unwrap();
    t.set_children(content, &[button]).unwrap();
    let root = t
        .create(
            Style {
                display: Display::Flex,
                size: taffy::Size {
                    width: Dimension::percent(1.0),
                    height: Dimension::percent(1.0),
                },
                ..Style::default()
            },
            Measure::None,
        )
        .unwrap();
    t.set_children(root, &[sidebar, content]).unwrap();
    t.compute(root, Size::new(800.0, 600.0)).unwrap();

    let mut nodes = vec![("root", 0, root), ("sidebar", 1, sidebar)];
    for r in &rows {
        nodes.push(("row", 2, *r));
    }
    nodes.extend([("content", 1, content), ("button", 2, button), ("label", 3, label)]);
    assert_golden(&golden_dir(), "flex", &dump(&t, &nodes));
}

#[test]
fn grid_snapshot() {
    let mut t = LayoutTree::new();
    let cells: Vec<LayoutNode> = (0..5)
        .map(|_| t.create(Style::default(), Measure::Fixed(Size::new(10.0, 20.0))).unwrap())
        .collect();
    let grid = t
        .create(
            Style {
                display: Display::Grid,
                size: taffy::Size {
                    width: Dimension::length(300.0),
                    height: Dimension::auto(),
                },
                grid_template_columns: vec![
                    taffy::GridTemplateComponent::Single(length(60.0)),
                    taffy::GridTemplateComponent::Single(fr(1.0)),
                    taffy::GridTemplateComponent::Single(fr(2.0)),
                ],
                gap: taffy::Size {
                    width: length(6.0),
                    height: length(4.0),
                },
                ..Style::default()
            },
            Measure::None,
        )
        .unwrap();
    t.set_children(grid, &cells).unwrap();
    t.compute(grid, Size::new(300.0, 300.0)).unwrap();
    let mut nodes = vec![("grid", 0, grid)];
    nodes.extend(cells.iter().map(|c| ("cell", 1, *c)));
    assert_golden(&golden_dir(), "grid", &dump(&t, &nodes));
}

#[test]
fn only_dirty_subtrees_relayout() {
    let mut t = LayoutTree::new();
    // Two fixed-width columns, each with text leaves.
    let mk_col = |t: &mut LayoutTree| {
        let leaves: Vec<LayoutNode> = (0..4).map(|i| t.create(Style::default(), text(5 + i)).unwrap()).collect();
        let col = t.create(col_style(), Measure::None).unwrap();
        t.set_children(col, &leaves).unwrap();
        (col, leaves)
    };
    let (a, a_leaves) = mk_col(&mut t);
    let (b, _b_leaves) = mk_col(&mut t);
    let root = t.create(Style::default(), Measure::None).unwrap();
    t.set_children(root, &[a, b]).unwrap();
    t.compute(root, Size::new(800.0, 600.0)).unwrap();
    let first = t.stats().measure_calls;
    assert!(first > 0);

    // Nothing changed: recompute measures nothing.
    t.compute(root, Size::new(800.0, 600.0)).unwrap();
    assert_eq!(t.stats().measure_calls, first);

    // Writing an identical style does not dirty anything.
    assert!(!t.set_style(a, &col_style()).unwrap());
    assert!(!t.set_children(a, &a_leaves).unwrap());
    assert!(!t.is_dirty(root).unwrap());

    // Change one leaf in column A: column B's leaves are not measured again.
    assert!(t.set_measure(a_leaves[0], text(40)).unwrap());
    assert!(t.is_dirty(a).unwrap());
    assert!(!t.is_dirty(b).unwrap());
    let before = t.stats().measure_calls;
    t.compute(root, Size::new(800.0, 600.0)).unwrap();
    let after = t.stats().measure_calls;
    assert!(after > before, "dirty leaf must be measured");
    let per_column_full = first / 2;
    assert!(
        after - before < per_column_full + 1,
        "relayout measured {} times; a full column costs {per_column_full}",
        after - before
    );
    // The changed text wraps to two lines.
    assert_eq!(t.rect(a_leaves[0]).unwrap().height, 32.0);
}

fn col_style() -> Style {
    Style {
        flex_direction: FlexDirection::Column,
        size: taffy::Size {
            width: Dimension::length(200.0),
            height: Dimension::length(400.0),
        },
        ..Style::default()
    }
}

#[test]
fn dock_layout_and_splitter_drags() {
    let mut dock = Dock {
        left: Some(DockSide::new(240.0, 160.0)),
        right: Some(DockSide::new(300.0, 200.0)),
        top: None,
        bottom: Some(DockSide::new(180.0, 100.0)),
        ..Dock::default()
    };
    let bounds = Rect::new(0.0, 0.0, 1200.0, 800.0);
    let mut out = String::new();
    let mut snap = |label: &str, dock: &Dock| {
        let l = dock.layout(bounds);
        writeln!(out, "# {label}").unwrap();
        for (edge, r) in &l.panels {
            writeln!(out, "{edge:?} {r}").unwrap();
        }
        writeln!(out, "center {}", l.center).unwrap();
        for s in &l.splitters {
            writeln!(out, "splitter {:?} {} hit {}", s.edge, s.rect, s.hit_rect).unwrap();
        }
    };
    snap("initial", &dock);

    // Hit-testing the left splitter, including its slop.
    assert_eq!(dock.splitter_at(bounds, Vec2::new(242.0, 400.0)), Some(DockEdge::Left));
    assert_eq!(dock.splitter_at(bounds, Vec2::new(236.0, 400.0)), Some(DockEdge::Left));
    assert_eq!(dock.splitter_at(bounds, Vec2::new(250.0, 400.0)), None);
    assert_eq!(dock.splitter_at(bounds, Vec2::new(600.0, 621.0)), Some(DockEdge::Bottom));

    let size = bounds.size();
    let drag = dock.begin_drag(DockEdge::Left).unwrap();
    assert_eq!(dock.drag(drag, 60.0, size), 300.0);
    // Past the minimum: clamped.
    assert_eq!(dock.drag(drag, -500.0, size), 160.0);
    // Coming back re-engages at the pointer.
    assert_eq!(dock.drag(drag, 10.0, size), 250.0);
    // Past the maximum: 1200 - 300 (right) - 120 (center min).
    assert_eq!(dock.drag(drag, 2000.0, size), 780.0);
    snap("left dragged to max", &dock);

    let drag = dock.begin_drag(DockEdge::Right).unwrap();
    // Right grows when dragged left; left already took the room.
    assert_eq!(dock.drag(drag, -100.0, size), 300.0);
    let drag_l = dock.begin_drag(DockEdge::Left).unwrap();
    dock.drag(drag_l, -400.0, size);
    assert_eq!(dock.drag(drag, -100.0, size), 400.0);

    let drag = dock.begin_drag(DockEdge::Bottom).unwrap();
    assert_eq!(dock.drag(drag, -50.0, size), 230.0);
    assert_eq!(dock.drag(drag, 500.0, size), 100.0);
    snap("right and bottom dragged", &dock);

    // Hidden panels take no space and have no splitter.
    dock.left.as_mut().unwrap().visible = false;
    assert!(dock.begin_drag(DockEdge::Left).is_none());
    snap("left hidden", &dock);

    // A window too small for the stored sizes is clamped at layout.
    let small = dock.layout(Rect::new(0.0, 0.0, 400.0, 200.0));
    assert!(small.center.width >= 0.0);
    assert_eq!(small.panel(DockEdge::Right).unwrap().width, 280.0);

    assert_golden(&golden_dir(), "dock", &out);
}

#[test]
fn canvas_zoom_and_culling() {
    let mut canvas = Canvas::default();
    let viewport = Size::new(800.0, 600.0);
    let items: Vec<Rect> = (0..20)
        .flat_map(|y| (0..20).map(move |x| Rect::new(x as f32 * 100.0, y as f32 * 100.0, 50.0, 50.0)))
        .collect();

    let visible = canvas.cull(viewport, 0.0, &items);
    assert_eq!(visible.len(), 8 * 6);

    // Zoom in 2x around the view center: the canvas point there stays put.
    let anchor = Vec2::new(400.0, 300.0);
    let before = canvas.to_canvas(anchor);
    canvas.zoom_at(anchor, 2.0);
    assert!((canvas.to_canvas(anchor) - before).length() < 1e-4);
    assert_eq!(canvas.visible_rect(viewport), Rect::new(200.0, 150.0, 400.0, 300.0));
    let visible = canvas.cull(viewport, 0.0, &items);
    assert_eq!(visible.len(), 4 * 3);
    // Round-trip through the transform.
    let t = canvas.transform();
    assert_eq!(t.apply(Vec2::new(200.0, 150.0)), Vec2::ZERO);

    // Pan far away: nothing visible; margin brings neighbours in.
    canvas.pan_by(Vec2::new(5000.0, 0.0));
    assert!(canvas.cull(viewport, 0.0, &items).is_empty());

    // Zoom is clamped.
    canvas.zoom_at(anchor, 1e6);
    assert_eq!(canvas.zoom, canvas.max_zoom);

    // The spatial index agrees with the linear cull.
    let mut index = CanvasIndex::new(256.0);
    for r in &items {
        index.insert(*r);
    }
    let mut c = Canvas::default();
    for (pan, zoom) in [((0.0, 0.0), 1.0), ((-730.0, -410.0), 0.5), ((120.0, 40.0), 3.0)] {
        c.pan = Vec2::new(pan.0, pan.1);
        c.zoom = zoom;
        let linear: Vec<u32> = c
            .cull(viewport, 16.0, &items)
            .into_iter()
            .map(|i| u32::try_from(i).unwrap())
            .collect();
        assert_eq!(index.visible(&c, viewport, 16.0), linear);
    }
}
