// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The primitive shader on the backends this machine may not have: it
//! validates without optional capabilities, translates to GLSL ES 3.10 and
//! to the Metal shading language, and stays within the downlevel limits the
//! hosts request. Translation is not execution; the GPU tests run it.

use naga::back::{glsl, msl};
use naga::valid::{Capabilities, ValidationFlags, Validator};

fn module() -> (naga::Module, naga::valid::ModuleInfo) {
    let module = naga::front::wgsl::parse_str(include_str!("../src/shaders/primitives.wgsl"))
        .expect("the shader parses");
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .expect("the shader validates");
    (module, info)
}

#[test]
fn within_the_downlevel_limits() {
    let (module, _) = module();
    let limits = wgpu::Limits::downlevel_defaults();
    let count = |space: fn(naga::AddressSpace) -> bool| {
        module
            .global_variables
            .iter()
            .filter(|(_, v)| space(v.space))
            .count() as u32
    };
    // Even if every buffer were used by one stage.
    let storage = count(|s| matches!(s, naga::AddressSpace::Storage { .. }));
    assert_eq!(storage, 2);
    assert!(storage <= limits.max_storage_buffers_per_shader_stage);
    let uniform = count(|s| s == naga::AddressSpace::Uniform);
    assert!(uniform <= limits.max_uniform_buffers_per_shader_stage);
}

#[test]
fn translates_to_gles_3_1() {
    let (module, info) = module();
    let options = glsl::Options {
        version: glsl::Version::Embedded {
            version: 310,
            is_webgl: false,
        },
        ..glsl::Options::default()
    };
    for entry in &module.entry_points {
        let pipeline = glsl::PipelineOptions {
            shader_stage: entry.stage,
            entry_point: entry.name.clone(),
            multiview: None,
        };
        let mut out = String::new();
        glsl::Writer::new(
            &mut out,
            &module,
            &info,
            &options,
            &pipeline,
            naga::proc::BoundsCheckPolicies::default(),
        )
        .and_then(|mut w| w.write())
        .unwrap_or_else(|e| panic!("{}: {e}", entry.name));
        assert!(out.starts_with("#version 310 es"), "{}", entry.name);
    }
}

#[test]
fn translates_to_metal() {
    let (module, info) = module();
    let options = msl::Options {
        fake_missing_bindings: true,
        ..msl::Options::default()
    };
    let (source, translated) =
        msl::write_string(&module, &info, &options, &msl::PipelineOptions::default())
            .expect("translates");
    assert!(!source.is_empty());
    for name in translated.entry_point_names {
        name.expect("every entry point translates");
    }
}
