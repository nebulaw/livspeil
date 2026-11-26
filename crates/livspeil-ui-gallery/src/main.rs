// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! `cargo run -p livspeil-ui-gallery [page]`: the component gallery in a
//! window (see `README.md`, "Try the gallery"). `--gpu-info` prints the GPU
//! stack and the chosen blur path instead (see `docs/ui-host.md`).

use livspeil_ui_core::{App, KEYMAP_FILE};
use livspeil_ui_gallery::{Chrome, Page, WINDOW_SIZE, mount_with};
use livspeil_ui_host_winit::{HostOptions, PaintRenderer, Titlebar, WidgetClipboard, run};
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_theme::{ConfigKind, ConfigWatcher, config_dir};

fn main() {
    if std::env::args().any(|a| a == "--gpu-info") {
        match livspeil_ui_host_winit::gpu_info() {
            Ok(report) => print!("{}", report.to_text()),
            Err(e) => {
                eprintln!("livspeil-ui-gallery --gpu-info: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let page = std::env::args()
        .nth(1)
        .and_then(|a| Page::ALL.into_iter().find(|p| p.slug() == a))
        .unwrap_or(Page::Buttons);
    let mut app = App::new();
    let mut renderer = PaintRenderer::for_theme(PaintBridge::new(FontSystem::new()), app.theme());
    renderer.install_measurer(&mut app);
    // Tell the Materials page when blur had to fall back to plain colours.
    renderer.on_blur_path(|path| {
        livspeil_ui_gallery::set_blur_note(match path {
            livspeil_ui_host_winit::BlurPath::Fallback(why) => Some(why.clone()),
            _ => None,
        });
    });
    // macOS: our titlebar under a transparent, full-size native one (the
    // traffic lights stay). Linux and others: system decorations, so X11
    // window managers and Wayland compositors behave natively.
    let chrome = Chrome::native();
    let mounted = mount_with(&mut app, FontSystem::new(), page, chrome);
    WidgetClipboard::install_system(&mounted.env);

    let env = mounted.env.clone();
    let mut options = HostOptions::default()
        .title("Livspeil Gallery")
        .size(f64::from(WINDOW_SIZE.0), f64::from(WINDOW_SIZE.1))
        .min_size(800.0, 560.0)
        .titlebar(match chrome {
            Chrome::Titlebar => Titlebar::Hidden,
            Chrome::System => Titlebar::Standard,
        })
        // Accessibility actions can move focus: let text fields know.
        .after_accessibility_action(move |app| env.sync_focus(app));
    // Hot reload `~/.config/livspeil/ui/{theme,motion,keymap}.toml` when the
    // directory exists; otherwise no timer runs.
    if let Some(dir) = config_dir().filter(|d| d.is_dir()) {
        let mut watcher = ConfigWatcher::new();
        watcher.watch_dir(&dir);
        watcher.watch(dir.join(KEYMAP_FILE), ConfigKind::Raw("keymap"));
        options = options.config(watcher);
    }
    if let Err(e) = run(app, renderer, options) {
        eprintln!("livspeil-ui-gallery: {e}");
        std::process::exit(1);
    }
}
