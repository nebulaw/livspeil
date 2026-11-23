// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! `livspeil-ui-shot`: render named scenes to PNG (run via `cargo xtask shot`).

use std::path::PathBuf;
use std::process::ExitCode;

use livspeil_ui_shot::{
    Tolerance, compare, gpu_or_skip, render_scene_cpu, scenes, shots_dir, write_png,
};

const HELP: &str = "\
livspeil-ui-shot [SCENE...] [--gpu] [--out DIR] [--list]

Renders scenes with the CPU reference rasterizer to DIR/<scene>.png
(default target/shots). With --gpu, also renders on the GPU to
DIR/<scene>.gpu.png and reports the difference to the CPU image (skipped
with a message when there is no adapter). No SCENE renders all.
";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut names = Vec::new();
    let mut gpu = false;
    let mut out = shots_dir();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gpu" => gpu = true,
            "--out" | "-o" => out = PathBuf::from(args.next().ok_or("--out needs a directory")?),
            "--list" => {
                for s in scenes() {
                    println!("{:<14} {}", s.name, s.description);
                }
                return Ok(true);
            }
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(true);
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other}").into()),
            other => names.push(other.to_string()),
        }
    }
    let all = scenes();
    let selected: Vec<_> = if names.is_empty() {
        all
    } else {
        let mut v = Vec::new();
        for n in &names {
            v.push(
                all.iter()
                    .find(|s| s.name == n)
                    .copied()
                    .ok_or_else(|| format!("unknown scene '{n}' (see --list)"))?,
            );
        }
        v
    };
    let mut shooter = if gpu { gpu_or_skip()? } else { None };
    if let Some(s) = &shooter {
        eprintln!("GPU adapter: {}", s.describe());
    }
    let mut ok = true;
    for scene in &selected {
        let cpu = render_scene_cpu(scene)?;
        let path = out.join(format!("{}.png", scene.name));
        write_png(&path, &cpu)?;
        println!("{}", path.display());
        if let Some(s) = shooter.as_mut() {
            let img = s.render(scene)?;
            let gpath = out.join(format!("{}.gpu.png", scene.name));
            write_png(&gpath, &img)?;
            let tol: Tolerance = s.tolerance();
            let cmp = compare(&img, &cpu, tol);
            println!(
                "{}  (vs CPU: {cmp}{})",
                gpath.display(),
                if cmp.passed { "" } else { ", OVER TOLERANCE" }
            );
            if !cmp.passed {
                write_png(&out.join(format!("{}.gpu.diff.png", scene.name)), &cmp.diff)?;
                ok = false;
            }
        }
    }
    Ok(ok)
}
