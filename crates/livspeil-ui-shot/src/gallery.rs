// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Gallery scenes: every page of `livspeil-ui-gallery` (the same library
//! views as the app) at 1x and 2x in dark Simple, plus the Theme page in
//! light Pro. Bundled fonts only (deterministic).

use livspeil_ui_core::{App, PaintCmd, Scene as CoreScene, TestHost};
use livspeil_ui_gallery::{Page, WINDOW_SIZE, mount};
use livspeil_ui_layout::Size;
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_render::Color;
use livspeil_ui_theme::{Density, Scheme, Theme};

use crate::ShotError;
use crate::scenes::{Scene, Shot, surface};
use crate::ui::finish;

/// The settled scene of `page` with `theme`, measured with `bridge`.
pub fn settle_page(bridge: &PaintBridge, page: Page, theme: Theme) -> Result<CoreScene, ShotError> {
    let mut app = App::new();
    app.set_text_measurer(bridge.text.measurer());
    app.set_theme(theme);
    mount(&mut app, FontSystem::test(), page);
    let mut host = TestHost::new(app, Size::new(WINDOW_SIZE.0, WINDOW_SIZE.1));
    host.run_until_idle(300);
    host.scene()
        .cloned()
        .ok_or_else(|| ShotError::Scene("the gallery drew no frame".into()))
}

/// What `livspeil-ui-host-winit`'s `PaintRenderer` does to every scene
/// (`widen_text_runs`): one more pixel of width per text run, so labels
/// whose layout box was rounded below their measured width do not wrap.
fn widen_text_runs(scene: &mut CoreScene) {
    for c in &mut scene.commands {
        if let PaintCmd::Text(t) = c
            && t.rect.width > 0.0
        {
            t.rect.width += 1.0;
        }
    }
}

fn shoot(page: Page, theme: Theme, scale: f32) -> Result<Shot, ShotError> {
    let bridge = PaintBridge::new(FontSystem::test());
    let mut scene = settle_page(&bridge, page, theme)?;
    widen_text_runs(&mut scene);
    finish(bridge, &scene, scale, |_, _| Ok(()))
}

fn page_shot<const P: usize>(scale: f32) -> Result<Shot, ShotError> {
    shoot(Page::ALL[P], Theme::default(), scale)
}

fn theme_light_pro(scale: f32) -> Result<Shot, ShotError> {
    let theme = Theme {
        scheme: Scheme::Light,
        density: Density::Pro,
        ..Theme::default()
    };
    shoot(Page::Theme, theme, scale)
}

fn dark() -> Color {
    surface(0)
}

/// Scene names `gallery-<slug>-1x` / `-2x`, in page order.
const NAMES: [[&str; 2]; 10] = [
    ["gallery-buttons-1x", "gallery-buttons-2x"],
    ["gallery-toggles-1x", "gallery-toggles-2x"],
    ["gallery-radios-1x", "gallery-radios-2x"],
    ["gallery-sliders-1x", "gallery-sliders-2x"],
    ["gallery-text-fields-1x", "gallery-text-fields-2x"],
    ["gallery-select-1x", "gallery-select-2x"],
    ["gallery-materials-1x", "gallery-materials-2x"],
    ["gallery-typography-1x", "gallery-typography-2x"],
    ["gallery-motion-1x", "gallery-motion-2x"],
    ["gallery-theme-1x", "gallery-theme-2x"],
];

const BUILDS: [fn(f32) -> Result<Shot, ShotError>; 10] = [
    page_shot::<0>,
    page_shot::<1>,
    page_shot::<2>,
    page_shot::<3>,
    page_shot::<4>,
    page_shot::<5>,
    page_shot::<6>,
    page_shot::<7>,
    page_shot::<8>,
    page_shot::<9>,
];

/// The gallery scenes.
pub fn scenes() -> Vec<Scene> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (width, height) = (WINDOW_SIZE.0 as u32, WINDOW_SIZE.1 as u32);
    let mut v = Vec::new();
    for (i, names) in NAMES.iter().enumerate() {
        for (j, scale) in [1.0, 2.0].into_iter().enumerate() {
            v.push(Scene {
                name: names[j],
                description: "a gallery page (livspeil-ui-gallery), dark Simple",
                width,
                height,
                scale,
                build: BUILDS[i],
                clear: dark,
            });
        }
    }
    v.push(Scene {
        name: "gallery-theme-light-pro",
        description: "the gallery's Theme page in light Pro",
        width,
        height,
        scale: 1.0,
        build: theme_light_pro,
        clear: dark,
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_pages() {
        for (page, names) in Page::ALL.iter().zip(NAMES) {
            assert_eq!(names[0], format!("gallery-{}-1x", page.slug()));
            assert_eq!(names[1], format!("gallery-{}-2x", page.slug()));
        }
    }
}
