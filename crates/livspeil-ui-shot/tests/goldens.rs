// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Golden-image tests. Goldens come from the CPU reference rasterizer;
//! refresh with `UPDATE_GOLDEN=1 cargo test -p livspeil-ui-shot` and look
//! at the PNGs before committing. GPU output is checked against the same
//! goldens when an adapter exists (skipped with a message otherwise).

use livspeil_ui_shot::{
    GoldenOutcome, Tolerance, check_golden, gpu_or_skip, render_scene_cpu, scenes, shots_dir,
    update_requested,
};

fn report(name: &str, tag: &str, outcome: GoldenOutcome) -> Option<String> {
    match outcome {
        GoldenOutcome::Matched(c) => {
            eprintln!("{name} [{tag}]: ok ({c})");
            None
        }
        GoldenOutcome::Updated => {
            eprintln!("{name} [{tag}]: golden updated");
            None
        }
        GoldenOutcome::Missing(p) => Some(format!(
            "{name} [{tag}]: no golden at {} (run with UPDATE_GOLDEN=1)",
            p.display()
        )),
        GoldenOutcome::Mismatch {
            comparison,
            actual,
            diff,
        } => Some(format!(
            "{name} [{tag}]: {comparison}; actual {}, diff {}",
            actual.display(),
            diff.display()
        )),
    }
}

#[test]
fn cpu_matches_goldens() {
    let out = shots_dir().join("diff");
    let update = update_requested();
    let mut failures = Vec::new();
    for scene in scenes() {
        let img = render_scene_cpu(&scene).unwrap();
        let outcome = check_golden(scene.name, "cpu", &img, Tolerance::CPU, update, &out).unwrap();
        failures.extend(report(scene.name, "cpu", outcome));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn gpu_matches_goldens() {
    let Some(mut gpu) = gpu_or_skip().unwrap() else {
        return;
    };
    eprintln!("GPU adapter: {}", gpu.describe());
    let out = shots_dir().join("diff");
    let tol = gpu.tolerance();
    let mut failures = Vec::new();
    for scene in scenes() {
        let img = gpu.render(&scene).unwrap();
        // Goldens are never written from the GPU.
        let outcome = check_golden(scene.name, "gpu", &img, tol, false, &out).unwrap();
        if matches!(outcome, GoldenOutcome::Missing(_)) && update_requested() {
            continue;
        }
        failures.extend(report(scene.name, "gpu", outcome));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
