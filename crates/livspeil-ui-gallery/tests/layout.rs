// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Every gallery page at the default, a narrow and a tall window: text
//! gets a box as tall as it is painted, cards keep their padding and
//! nothing runs off the window's right edge.

use livspeil_ui_core::{App, ElementId, PaintCmd, TestHost};
use livspeil_ui_gallery::{Page, SIDEBAR_WIDTH, WINDOW_SIZE, mount};
use livspeil_ui_layout::{Size, Vec2};
use livspeil_ui_paint::PaintBridge;
use livspeil_ui_text::{FontSystem, LayoutOptions, TextLayout, TextStyle};

const SIZES: [(f32, f32); 3] = [WINDOW_SIZE, (700.0, 500.0), (1274.0, 1428.0)];
const CARD_PADDING: f32 = 16.0;
/// Elements that clip or pan their children on purpose.
const CLIPS: [&str; 5] = ["tabs:", "frame", "preview", "material-stage", "scroll"];

fn host(page: Page, (w, h): (f32, f32)) -> TestHost {
    let mut app = App::new();
    let bridge = PaintBridge::new(livspeil_ui_paint::FontSystem::test());
    app.set_text_measurer(bridge.text.measurer());
    mount(&mut app, FontSystem::test(), page);
    let mut host = TestHost::new(app, Size::new(w, h));
    host.run_until_idle(300);
    host
}

/// Problems with text boxes: painted (wrapped as the renderer wraps it, with
/// the host's one pixel of slack) taller than laid out.
fn text_problems(h: &TestHost, fonts: &mut FontSystem) -> Vec<String> {
    let mut out = Vec::new();
    for c in &h.scene().expect("a frame").commands {
        let PaintCmd::Text(t) = c else { continue };
        if t.rect.width <= 0.0 || t.text.trim().is_empty() {
            continue;
        }
        let mut style = TextStyle::sized(t.font.size).with_weight(t.font.weight);
        for f in &t.font.features {
            style = style.with_feature(livspeil_ui_text::FontFeature {
                tag: f.tag,
                value: f.value,
            });
        }
        let natural = TextLayout::new(fonts, &t.text, &style, LayoutOptions::default()).width();
        let width = t.rect.width + 1.0;
        if natural <= width + 0.5 {
            continue;
        }
        let painted = TextLayout::new(fonts, &t.text, &style, LayoutOptions::wrap(width)).height();
        if painted > t.rect.height + 1.0 {
            out.push(format!(
                "text {:?} needs {painted} px in a {} px box",
                &*t.text, t.rect
            ));
        }
    }
    out
}

/// Pages that do not scroll keep every line of text inside the window.
fn clipped_text(h: &TestHost, page: Page) -> Vec<String> {
    let mut out = Vec::new();
    if page.scrolls() || matches!(page, Page::Scroll | Page::NodeCanvas | Page::Dock) {
        return out;
    }
    let bottom = h.app.viewport().height;
    for c in &h.scene().expect("a frame").commands {
        if let PaintCmd::Text(t) = c
            && t.rect.bottom() > bottom + 0.5
            && t.rect.x > SIDEBAR_WIDTH
        {
            out.push(format!("text {:?} is cut off at {}", &*t.text, t.rect));
        }
    }
    out
}

fn card_problems(h: &TestHost, page: Page) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack: Vec<(ElementId, Option<ElementId>)> =
        vec![(h.app.find("gallery").expect("gallery"), None)];
    while let Some((id, card)) = stack.pop() {
        let label = h.app.label(id).unwrap_or_default();
        let is_card = label.starts_with("card:");
        if let (Some(card), Some(b)) = (card, h.app.bounds(id)) {
            let c = h.app.bounds(card).expect("card bounds");
            let (r, bottom) = (c.right() - CARD_PADDING, c.bottom() - CARD_PADDING);
            if b.width > 0.0 && (b.right() > r + 0.6 || b.bottom() > bottom + 0.6) {
                out.push(format!(
                    "{label} {b} leaves the padding of {} {c}",
                    h.app.label(card).unwrap_or_default()
                ));
                continue;
            }
        }
        if CLIPS.iter().any(|p| label.starts_with(p)) {
            continue;
        }
        let card = if is_card { Some(id) } else { card };
        if is_card {
            let b = h.app.bounds(id).expect("card bounds");
            let w = h.app.viewport().width;
            let page_scrolls = page.scrolls();
            if !page_scrolls && b.bottom() > h.app.viewport().height + 0.5 {
                out.push(format!("{label} {b} runs off the bottom"));
            }
            if b.right() > w + 0.5 {
                out.push(format!("{label} {b} runs off a {w} px window"));
            }
        }
        stack.extend(h.app.children_of(id).into_iter().map(|c| (c, card)));
    }
    out
}

#[test]
fn short_windows_scroll_the_page_and_popups_follow() {
    let mut h = host(Page::Select, (700.0, 360.0));
    let select = h.app.find("select:Easing").expect("the select");
    let tip = h.app.find("button:Another").expect("a button below");
    assert!(
        h.app.bounds(tip).unwrap().bottom() > 360.0,
        "starts below the window"
    );
    let before = h.app.visual_bounds(select).unwrap();
    // Wheel the page up a little.
    h.scroll(450.0, 300.0, Vec2::new(0.0, -40.0));
    h.run_until_idle(300);
    let after = h.app.visual_bounds(select).unwrap();
    assert!(after.y < before.y - 20.0, "the page scrolled: {after}");
    // The popup opens next to the select where it is now.
    let c = after.center();
    h.click(c.x, c.y);
    h.run_until_idle(300);
    let option = h.app.find("option:Linear").expect("the popup is open");
    let o = h.app.bounds(option).unwrap();
    assert!(
        (o.y - after.bottom()).abs() < 16.0,
        "popup {o} right under the select {after}"
    );
}

#[test]
fn switching_pages_starts_at_the_top() {
    let mut h = host(Page::Select, (700.0, 360.0));
    let y = |h: &TestHost, n: &str| h.app.visual_bounds(h.app.find(n).unwrap()).unwrap().y;
    let top = y(&h, "select:Easing");
    h.scroll(450.0, 300.0, Vec2::new(0.0, -60.0));
    h.run_until_idle(300);
    assert!(y(&h, "select:Easing") < top);
    h.key("cmd-1");
    h.run_until_idle(300);
    h.key("cmd-6");
    h.run_until_idle(300);
    assert!((y(&h, "select:Easing") - top).abs() < 0.6);
}

#[test]
fn pages_lay_out_at_small_default_and_tall_sizes() {
    let mut fonts = FontSystem::test();
    let mut problems = Vec::new();
    for size in SIZES {
        for page in Page::ALL {
            let h = host(page, size);
            for p in text_problems(&h, &mut fonts)
                .into_iter()
                .chain(card_problems(&h, page))
                .chain(clipped_text(&h, page))
            {
                problems.push(format!("{page:?} at {size:?}: {p}"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
