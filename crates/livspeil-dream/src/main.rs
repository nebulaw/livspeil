// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil-dream <room> [--select=NAME] [--play]`: open a room, see its
//! graph and watch it run.

use std::path::PathBuf;
use std::process::ExitCode;

use livspeil_dream::{Session, app};
use livspeil_ui_core::App;
use livspeil_ui_host_winit::{HostOptions, PaintRenderer, WidgetClipboard, run};
use livspeil_ui_paint::{FontSystem as PaintFonts, PaintBridge};
use livspeil_ui_text::FontSystem;
use livspeil_ui_widgets::Env;

const USAGE: &str = "usage: livspeil-dream <room> [--select=NAME] [--play]";

/// Warnings and errors from the host and renderer, on stderr.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("{} {}: {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

struct Args {
    room: PathBuf,
    select: Option<String>,
    play: bool,
}

fn args(list: impl Iterator<Item = String>) -> Result<Args, String> {
    let (mut room, mut select, mut play) = (None, None, false);
    for arg in list {
        if let Some(name) = arg.strip_prefix("--select=") {
            select = Some(name.to_owned());
        } else if arg == "--play" {
            play = true;
        } else if arg.starts_with('-') || room.is_some() {
            return Err(format!("unexpected argument {arg}"));
        } else {
            room = Some(PathBuf::from(arg));
        }
    }
    Ok(Args {
        room: room.ok_or("no room given")?,
        select,
        play,
    })
}

fn main() -> ExitCode {
    // Fails only if a logger is already installed, which is fine too.
    if log::set_logger(&StderrLog).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    let args = match args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("livspeil-dream: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut session = match Session::open(&args.room) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("livspeil-dream: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(name) = &args.select {
        let Some(i) = session.find(name) else {
            eprintln!("livspeil-dream: no node called {name}");
            return ExitCode::FAILURE;
        };
        session.select(Some(i));
    }
    let title = format!("{} — Livspeil Dream", session.name());
    let mut app = App::new();
    let renderer = PaintRenderer::for_theme(PaintBridge::new(PaintFonts::new()), app.theme());
    renderer.install_measurer(&mut app);
    let env = Env::new(&mut app, FontSystem::new());
    WidgetClipboard::install_system(&env);
    let mounted = app::mount(&mut app, &env, session);
    if args.play {
        app::set_playing(mounted.editor, &mut app, true);
    }
    let focus = env.clone();
    let options = HostOptions::default()
        .title(title)
        .size(1280.0, 800.0)
        .min_size(800.0, 500.0)
        // Accessibility actions can move focus: let text fields know.
        .after_accessibility_action(move |app| focus.sync_focus(app))
        .on_close_requested(move |app| mounted.may_close(app));
    match run(app, renderer, options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("livspeil-dream: {e}");
            ExitCode::FAILURE
        }
    }
}
