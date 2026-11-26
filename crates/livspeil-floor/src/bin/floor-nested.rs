// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Floor inside a window: `cargo run -p livspeil-floor --bin floor-nested`.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The Floor is Linux only.");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
const HELP: &str = "\
floor-nested: the Livspeil Floor in a window (spike)

USAGE: floor-nested [OPTIONS]

  --size WxH           window size (default 1280x800)
  --spawn CMD          start CMD as a client (repeatable), e.g. --spawn foot
  --panel X,Y          place the Frost panel (default 120,150)
  --no-panel           no Frost panel
  --no-blur            panel without backdrop blur (plain tint)
  --no-shadow          no window shadows
  --no-round           square corners, no clip mask
  --bench N            render N frames continuously, print frame times, exit
  --screenshot PATH    spawn two test clients, save a PNG once mapped, exit
  -h, --help           this text

Quit with Ctrl+Alt+Q or by closing the window. RUST_LOG=info for GL details.
";

#[cfg(target_os = "linux")]
fn parse() -> Result<livspeil_floor::nested::Options, String> {
    use livspeil_floor::{geometry::Rect, nested::Options};

    let mut o = Options::default();
    let mut args = std::env::args().skip(1);
    let mut test_client = false;
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v.split_once('x').ok_or("--size expects WxH")?;
                o.size = (
                    w.parse().map_err(|_| "bad width")?,
                    h.parse().map_err(|_| "bad height")?,
                );
            }
            "--spawn" => o.spawn.push(value("--spawn")?),
            "--panel" => {
                let v = value("--panel")?;
                let (x, y) = v.split_once(',').ok_or("--panel expects X,Y")?;
                // Multiples of 4 keep every blur level on whole pixels.
                let x: i32 = x.parse().map_err(|_| "bad x")?;
                let y: i32 = y.parse().map_err(|_| "bad y")?;
                o.panel = Some(Rect::new(x, y, 360, 96));
            }
            "--no-panel" => o.panel = None,
            "--no-blur" => o.fx.blur = false,
            "--no-shadow" => o.fx.shadows = false,
            "--no-round" => o.fx.rounded = false,
            "--bench" => {
                o.bench = Some(value("--bench")?.parse().map_err(|_| "bad frame count")?);
                test_client = true;
            }
            "--screenshot" => {
                o.screenshot = Some(value("--screenshot")?.into());
                test_client = true;
            }
            other => return Err(format!("unknown option {other} (try --help)")),
        }
    }
    if test_client && o.spawn.is_empty() {
        // Use the fixture client next to this binary.
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let client = exe.with_file_name("floor-test-client");
        let client = client.display();
        o.spawn.push(format!("{client} 'Test window A'"));
        o.spawn.push(format!("{client} 'Test window B'"));
    }
    if o.screenshot.is_some() {
        o.expect_windows = o.spawn.len();
    }
    Ok(o)
}

#[cfg(target_os = "linux")]
fn main() {
    use tracing_subscriber::EnvFilter;

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();
    let opts = match parse() {
        Ok(o) => o,
        Err(msg) => {
            eprintln!("floor-nested: {msg}");
            std::process::exit(2);
        }
    };
    if let Err(err) = livspeil_floor::nested::run(opts) {
        eprintln!("floor-nested: {err}");
        std::process::exit(1);
    }
}
