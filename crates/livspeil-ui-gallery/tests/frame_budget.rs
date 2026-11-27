// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Work budgets per frame for every [`Scenario`]
//! (`specs/livspeil-ui-frame-pipeline.md`, section 5). Counts, not times, so
//! they are the same on every machine. Lower a budget when a change earns
//! it; raise one only with a reason in the commit.

use livspeil_ui_core::{App, TestHost};
use livspeil_ui_gallery::{Scenario, mount};
use livspeil_ui_layout::Size;
use livspeil_ui_paint::{FontSystem as PaintFonts, PaintBridge};
use livspeil_ui_render::{Color, DisplayList, Frame, FrameOptions, encode_into};
use livspeil_ui_text::FontSystem;

const SIZE: Size = Size {
    width: 1274.0,
    height: 1428.0,
};
/// Interacting frames, and how many of the first are not measured.
const FRAMES: u32 = 90;
const SETTLE: u32 = 2;

/// The work of the measured frames.
#[derive(Debug, Default, Clone, Copy)]
struct Work {
    /// Frames in which some view rendered.
    rendered: u32,
    /// Frames in which a layout ran.
    layouts: u32,
    /// Most views rendered and elements reconciled in one frame.
    views: usize,
    reconciled: usize,
    /// Most instances and instance bytes in one frame.
    instances: usize,
    upload_bytes: usize,
    /// Frames that drew something else than the frame before.
    changed: u32,
}

fn measure(scenario: Scenario) -> Work {
    let mut app = App::new();
    let mut bridge = PaintBridge::new(PaintFonts::test());
    app.set_text_measurer(bridge.text.measurer());
    let mounted = mount(&mut app, FontSystem::test(), scenario.page());
    let mut host = TestHost::new(app, SIZE);
    let options = FrameOptions {
        width: 1274,
        height: 1428,
        scale: 1.0,
        clear: Color::BLACK,
    };
    let mut list = DisplayList::new();
    let mut gpu = Frame {
        options,
        clear: [0.0; 4],
        instances: Vec::new(),
        ops: Vec::new(),
    };
    let mut work = Work::default();
    let mut last = None;
    for frame in 0..Scenario::WARMUP + FRAMES {
        scenario.step(&mut host.app, &mounted, frame);
        let out = host.app.draw_frame(1.0 / 144.0);
        // The first interacting frames still settle (gesture starts).
        if frame < Scenario::WARMUP + SETTLE {
            last = Some(out.scene);
            continue;
        }
        list.items.clear();
        bridge
            .paint_scene(&out.scene, 1.0, &mut list)
            .expect("paint");
        encode_into(&list, options, &mut gpu).expect("encode");
        work.rendered += u32::from(out.stats.views_rendered > 0);
        work.layouts += u32::from(out.stats.layout);
        work.views = work.views.max(out.stats.views_rendered);
        work.reconciled = work.reconciled.max(out.stats.reconciled);
        work.instances = work.instances.max(gpu.instances.len());
        work.upload_bytes = work.upload_bytes.max(gpu.instance_byte_len());
        work.changed += u32::from(last.as_ref() != Some(&out.scene));
        last = Some(out.scene);
    }
    work
}

/// Today's work, the ceiling for tomorrow's, over 88 measured frames:
/// (frames that rendered, frames that ran layout, views in a frame,
/// elements reconciled in a frame, instances, bytes uploaded).
fn budget(scenario: Scenario) -> (u32, u32, usize, usize, usize, usize) {
    match scenario {
        Scenario::PanSample => (0, 0, 0, 0, 579, 185_280),
        Scenario::PanStress => (0, 0, 0, 0, 17_894, 5_726_080),
        Scenario::PanStressFit => (0, 0, 0, 0, 15_477, 4_952_640),
        Scenario::ZoomStress => (0, 0, 0, 0, 18_137, 5_803_840),
        Scenario::DragStress => (88, 88, 2, 2328, 18_089, 5_788_480),
        Scenario::ScrollList => (10, 10, 1, 388, 1777, 568_640),
    }
}

/// Culling at a float boundary may differ by a few items between platforms.
fn with_headroom(n: usize) -> usize {
    n + n / 50
}

#[test]
fn every_scenario_stays_within_its_budget() {
    let mut over = Vec::new();
    for s in Scenario::ALL {
        let work = measure(s);
        eprintln!("{}: {work:?}", s.slug());
        let (rendered, layouts, views, reconciled, instances, bytes) = budget(s);
        // A scenario that stops interacting would pass trivially.
        assert!(
            work.changed >= s.changing_frames() && work.instances > 0,
            "{s:?} did nothing: {work:?}"
        );
        if work.rendered > rendered
            || work.layouts > layouts
            || work.views > views
            || work.reconciled > with_headroom(reconciled)
            || work.instances > with_headroom(instances)
            || work.upload_bytes > with_headroom(bytes)
        {
            over.push(format!("{}: {work:?}", s.slug()));
        }
    }
    assert!(over.is_empty(), "over budget:\n{}", over.join("\n"));
}

#[test]
fn scenarios_have_distinct_names_that_parse_back() {
    for s in Scenario::ALL {
        assert_eq!(Scenario::from_slug(s.slug()), Some(s));
    }
    assert_eq!(Scenario::from_slug("nothing"), None);
}
