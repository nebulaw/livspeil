// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! `cargo run -p livspeil-ui-gallery [page] [--hud] [--present-mode=MODE]`:
//! the component gallery in a window (see `README.md`, "Try the gallery").
//! `--gpu-info` prints the GPU stack and the chosen blur path instead (see
//! `docs/ui-host.md`).
//!
//! The frame HUD (F3 shows or hides it, F4 dumps the frame log) is always
//! available; `--hud` or `LIVSPEIL_HUD=1` starts with it visible.

use livspeil_ui_core::{App, KEYMAP_FILE};
use livspeil_ui_gallery::{Chrome, Page, WINDOW_SIZE, mount_with};
use livspeil_ui_host_winit::frame_stats::default_log_path;
use livspeil_ui_host_winit::{
    HostOptions, HudOptions, PRESENT_MODE_ENV, PaintRenderer, SurfaceOptions, Titlebar,
    WidgetClipboard, parse_present_mode, run,
};
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let page = args
        .iter()
        .find_map(|a| Page::ALL.into_iter().find(|p| p.slug() == *a))
        .unwrap_or(Page::Buttons);
    let hud_visible = args.iter().any(|a| a == "--hud")
        || std::env::var("LIVSPEIL_HUD").is_ok_and(|v| !matches!(v.as_str(), "" | "0" | "false"));
    // `--present-mode=` beats `LIVSPEIL_PRESENT_MODE`.
    let present_flag = args
        .iter()
        .find_map(|a| a.strip_prefix("--present-mode="))
        .map(str::to_owned);
    if let Some(m) = &present_flag
        && parse_present_mode(m).is_none()
    {
        eprintln!(
            "livspeil-ui-gallery: --present-mode={m}: expected fifo, mailbox, immediate or auto"
        );
        std::process::exit(2);
    }
    let surface = SurfaceOptions::default().with_env(|k| match (k, &present_flag) {
        (PRESENT_MODE_ENV, Some(m)) => Some(m.clone()),
        _ => std::env::var(k).ok(),
    });
    let log_path = default_log_path("gallery-frames.log");
    match &log_path {
        Some(p) => eprintln!(
            "livspeil-ui-gallery: frame log: {} (F3 HUD, F4 dump)",
            p.display()
        ),
        None => eprintln!("livspeil-ui-gallery: no state directory, no frame log (F3 HUD)"),
    }
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
        .frame_hud(HudOptions {
            visible: hud_visible,
            log_path,
            ..HudOptions::default()
        })
        .titlebar(match chrome {
            Chrome::Titlebar => Titlebar::Hidden,
            Chrome::System => Titlebar::Standard,
        })
        // Accessibility actions can move focus: let text fields know.
        .after_accessibility_action(move |app| env.sync_focus(app));
    options.surface = surface;
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
