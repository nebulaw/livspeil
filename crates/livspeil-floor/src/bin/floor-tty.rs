// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Floor on bare metal (DRM/KMS). Build with
//! `cargo build -p livspeil-floor --features tty --bin floor-tty`, run from
//! a text TTY (Ctrl+Alt+F3), not from inside another compositor.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The Floor is Linux only.");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
const HELP: &str = "\
floor-tty: the Livspeil Floor on DRM/KMS (spike, untested on hardware)

USAGE: floor-tty [OPTIONS]      (run from a text TTY)

  --list               print GPU, connectors and modes, then exit (safe)
  --timeout SECS       exit by itself after SECS (default 120; 0 = never)
  --spawn CMD          start CMD as a client (repeatable)
  --no-blur --no-shadow --no-round --no-panel   disable effects
  --log PATH           log file (default /tmp/floor-tty.log)
  -h, --help           this text

Ctrl+Alt+Q quits, Ctrl+Alt+F1..F12 switch VT. RUST_LOG=debug for detail.
";

#[cfg(target_os = "linux")]
fn main() {
    use std::{fs::File, sync::Mutex, time::Duration};

    use livspeil_floor::tty::{Options, run};
    use tracing_subscriber::EnvFilter;

    let mut o = Options::default();
    let mut log_path = String::from("/tmp/floor-tty.log");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("floor-tty: {name} needs a value");
                std::process::exit(2);
            })
        };
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return;
            }
            "--list" => o.list_only = true,
            "--timeout" => {
                let secs: u64 = value("--timeout").parse().unwrap_or_else(|_| {
                    eprintln!("floor-tty: bad --timeout");
                    std::process::exit(2);
                });
                o.timeout = (secs > 0).then(|| Duration::from_secs(secs));
            }
            "--spawn" => o.spawn.push(value("--spawn")),
            "--log" => log_path = value("--log"),
            "--no-blur" => o.fx.blur = false,
            "--no-shadow" => o.fx.shadows = false,
            "--no-round" => o.fx.rounded = false,
            "--no-panel" => o.panel = None,
            other => {
                eprintln!("floor-tty: unknown option {other} (try --help)");
                std::process::exit(2);
            }
        }
    }

    // On a bare TTY stderr is the console the Floor is drawing over, so log
    // to a file the owner can send back.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    match File::create(&log_path) {
        Ok(file) => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .init(),
        Err(e) => {
            eprintln!("floor-tty: cannot create log {log_path}: {e}");
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
    if let Err(e) = run(o) {
        eprintln!("floor-tty: {e}");
        std::process::exit(1);
    }
}
