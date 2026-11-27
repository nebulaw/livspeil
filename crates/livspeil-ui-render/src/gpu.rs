// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! wgpu backend.
//!
//! Three buffers per frame (see [`record`](crate::record)): the draw order
//! as a per-instance vertex attribute, and the record arena and property
//! table as storage buffers (two per stage; the downlevel limit is four).
//! The vertex shader covers each instance's record with two triangles, a
//! curve with two per segment; the fragment shader evaluates it. Ops map to render passes: consecutive `Draw` ops share
//! a pass; a `Backdrop` ends the pass, runs the dual-Kawase passes on the
//! target (downsampled into `Rgba16Float` pyramid levels, scissored to the
//! backdrop's region) and composites.

use std::collections::HashMap;
use std::sync::mpsc;

use crate::GpuTimer;
use crate::RenderError;
use crate::atlas::AtlasUpdate;
use crate::cpu::{PassParams, blur_passes};
use crate::encode::{Frame, MAX_BLUR_LEVELS, Op, level_size};
use crate::image::RgbaImage;
use crate::record::{PROPERTY_BYTES, SLOT_BYTES};
use crate::support::{BlurPath, BlurSupport, choose_blur_path};

/// Format of the offscreen targets made by [`GpuContext::render_to_image`].
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const LEVEL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const PASS_STRIDE: u64 = 256;

/// A device and queue without a window.
#[derive(Debug)]
pub struct GpuContext {
    /// The device.
    pub device: wgpu::Device,
    /// Its queue.
    pub queue: wgpu::Queue,
    /// Which adapter was picked.
    pub adapter_info: wgpu::AdapterInfo,
}

impl GpuContext {
    /// Picks an adapter (any backend; `WGPU_BACKEND` and
    /// `WGPU_ADAPTER_NAME` are honored), falling back to a software adapter.
    /// Returns [`RenderError::NoAdapter`] when there is none at all, which
    /// tests treat as "skip".
    pub fn headless() -> Result<GpuContext, RenderError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .or_else(|_| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: true,
                ..Default::default()
            }))
        })
        .map_err(|e| RenderError::NoAdapter(e.to_string()))?;
        let adapter_info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("livspeil-ui-render"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| RenderError::Device(e.to_string()))?;
        Ok(GpuContext {
            device,
            queue,
            adapter_info,
        })
    }

    /// Whether the adapter is a software rasterizer (llvmpipe, lavapipe,
    /// WARP, SwiftShader).
    pub fn is_software(&self) -> bool {
        self.adapter_info.device_type == wgpu::DeviceType::Cpu
    }

    /// Renders a frame into a new offscreen texture and reads it back.
    pub fn render_to_image(
        &self,
        renderer: &mut Renderer,
        frame: &Frame,
    ) -> Result<RgbaImage, RenderError> {
        let (w, h) = (frame.options.width, frame.options.height);
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen target"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: Renderer::TARGET_USAGE | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        renderer.render(&self.device, &self.queue, frame, &target)?;
        let img = self.read_texture(&target, w, h);
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(RenderError::Device(err.to_string()));
        }
        img
    }

    /// Reads back a 4-byte-per-pixel texture (needs `COPY_SRC`).
    pub fn read_texture(
        &self,
        target: &wgpu::Texture,
        w: u32,
        h: u32,
    ) -> Result<RgbaImage, RenderError> {
        let unpadded = w as usize * 4;
        let padded = unpadded.div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * h as usize) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback"),
            });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            // The receiver outlives the poll below; a send error cannot
            // happen and would surface as a recv error anyway.
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| RenderError::Readback(e.to_string()))?
            .map_err(|e| RenderError::Readback(e.to_string()))?;
        let mut img = RgbaImage::new(w, h);
        {
            let view = slice
                .get_mapped_range()
                .map_err(|e| RenderError::Readback(e.to_string()))?;
            for (row, out) in view.chunks(padded).zip(img.data.chunks_exact_mut(unpadded)) {
                out.copy_from_slice(&row[..unpadded]);
            }
        }
        buffer.unmap();
        Ok(img)
    }
}

/// Pipelines and per-target resources. Create once per device and target
/// format; reuse across frames.
#[derive(Debug)]
pub struct Renderer {
    format: wgpu::TextureFormat,
    primitive_pipeline: wgpu::RenderPipeline,
    backdrop_pipeline: wgpu::RenderPipeline,
    down_pipeline: wgpu::RenderPipeline,
    up_pipeline: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    pass_layout: wgpu::BindGroupLayout,
    blit_pipeline: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    blur_support: BlurSupport,
    offscreen: Option<wgpu::Texture>,
    /// Set while frames are being timed.
    timer: Option<GpuTimer>,
    sampler: wgpu::Sampler,
    globals: wgpu::Buffer,
    /// The draw order, the record arena and the property table.
    instances: FrameBuffer,
    records: FrameBuffer,
    properties: FrameBuffer,
    levels: Vec<wgpu::Texture>,
    levels_for: (u32, u32),
    atlas_layout: wgpu::BindGroupLayout,
    atlases: HashMap<u32, GpuAtlas>,
    empty_atlas: wgpu::BindGroup,
}

/// One R8 coverage atlas on the GPU.
#[derive(Debug)]
struct GpuAtlas {
    generation: u64,
    size: [u32; 2],
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

fn create_atlas(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    size: [u32; 2],
) -> (wgpu::Texture, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glyph atlas"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: ATLAS_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glyph atlas"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });
    (texture, bind_group)
}

/// A GPU buffer holding one of a frame's arrays; grows in powers of two and
/// is kept across frames.
#[derive(Debug)]
struct FrameBuffer {
    label: &'static str,
    usage: wgpu::BufferUsages,
    /// Smallest size: one element, so an empty frame can still be bound.
    min_size: u64,
    buffer: Option<wgpu::Buffer>,
}

impl FrameBuffer {
    fn new(label: &'static str, usage: wgpu::BufferUsages, min_size: usize) -> FrameBuffer {
        FrameBuffer {
            label,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            min_size: min_size as u64,
            buffer: None,
        }
    }

    /// Makes room for `bytes` and writes them.
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
    ) -> Result<&wgpu::Buffer, RenderError> {
        let needed = (bytes.len() as u64).max(self.min_size);
        let limits = device.limits();
        let mut limit = limits.max_buffer_size;
        if self.usage.contains(wgpu::BufferUsages::STORAGE) {
            limit = limit.min(limits.max_storage_buffer_binding_size);
        }
        if needed > limit {
            return Err(RenderError::Device(format!(
                "{}: {needed} bytes exceed the device's limit of {limit}",
                self.label
            )));
        }
        if self.buffer.as_ref().is_none_or(|b| b.size() < needed) {
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: needed.next_power_of_two().min(limit),
                usage: self.usage,
                mapped_at_creation: false,
            }));
        }
        let Some(buffer) = self.buffer.as_ref() else {
            return Err(RenderError::Device(format!("{} missing", self.label)));
        };
        if !bytes.is_empty() {
            queue.write_buffer(buffer, 0, bytes);
        }
        Ok(buffer)
    }
}

fn create_level(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("blur level"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: LEVEL_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

impl Renderer {
    /// Usages a target texture needs: render attachment, plus texture
    /// binding so backdrops can blur it.
    pub const TARGET_USAGE: wgpu::TextureUsages =
        wgpu::TextureUsages::RENDER_ATTACHMENT.union(wgpu::TextureUsages::TEXTURE_BINDING);

    /// Creates the pipelines for targets of `format` (a non-sRGB 8-bit
    /// format: the shaders output gamma-encoded sRGB and blend in it).
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Renderer {
        let primitive_src = include_str!("shaders/primitives.wgsl");
        let blur_src = include_str!("shaders/blur.wgsl");
        let primitive_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("primitives.wgsl"),
            source: wgpu::ShaderSource::Wgsl(primitive_src.into()),
        });
        let blur_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur.wgsl"),
            source: wgpu::ShaderSource::Wgsl(blur_src.into()),
        });
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });
        let vis = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let buffer_entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                buffer_entry(0, wgpu::BufferBindingType::Uniform),
                buffer_entry(1, storage),
                buffer_entry(2, storage),
            ],
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("backdrop texture"),
            entries: &[tex_entry(0), sampler_entry(1)],
        });
        let pass_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur pass"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(1),
                sampler_entry(2),
            ],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glyph atlas"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let premul = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = |label: &str,
                        layouts: &[Option<&wgpu::BindGroupLayout>],
                        module: &wgpu::ShaderModule,
                        vs: &str,
                        fs: &str,
                        fmt: wgpu::TextureFormat,
                        blend: Option<wgpu::BlendState>,
                        buffers: &[Option<wgpu::VertexBufferLayout<'_>>]| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: fmt,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // One instance reference per instance (see `record::InstanceRef`).
        let instance_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 4,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Uint32],
        })];
        let primitive_pipeline = pipeline(
            "primitives",
            &[Some(&globals_layout), None, Some(&atlas_layout)],
            &primitive_module,
            "vs_main",
            "fs_main",
            format,
            Some(premul),
            &instance_buffers,
        );
        // Everything blur needs is built inside a validation scope: a
        // device that rejects it must not take the whole UI down (D63).
        let blur_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let backdrop_pipeline = pipeline(
            "backdrop",
            &[Some(&globals_layout), Some(&texture_layout)],
            &primitive_module,
            "vs_main",
            "fs_backdrop",
            format,
            Some(premul),
            &instance_buffers,
        );
        let down_pipeline = pipeline(
            "kawase down",
            &[Some(&pass_layout)],
            &blur_module,
            "vs_full",
            "fs_down",
            LEVEL_FORMAT,
            None,
            &[],
        );
        let up_pipeline = pipeline(
            "kawase up",
            &[Some(&pass_layout)],
            &blur_module,
            "vs_full",
            "fs_up",
            LEVEL_FORMAT,
            None,
            &[],
        );
        let blit_pipeline = pipeline(
            "blit",
            &[Some(&blit_layout)],
            &blit_module,
            "vs_blit",
            "fs_blit",
            format,
            None,
            &[],
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Zero-initialized: glyphs whose atlas is missing draw nothing.
        let (_, empty_atlas) = create_atlas(device, &atlas_layout, [1, 1]);
        // Probe the level format the way a blur pass uses it: a render
        // attachment that a filtering sampler reads.
        let probe = create_level(device, 1, 1);
        let probe_view = probe.create_view(&wgpu::TextureViewDescriptor::default());
        let _probe_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur probe"),
            layout: &texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&probe_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let blur_support = match pollster::block_on(blur_scope.pop()) {
            None => BlurSupport::Supported,
            Some(e) => BlurSupport::Fallback(format!(
                "blur pipelines or the {LEVEL_FORMAT:?} blur level format were rejected: {e}"
            )),
        };
        Renderer {
            blit_pipeline,
            blit_layout,
            blur_support,
            offscreen: None,
            timer: None,
            format,
            primitive_pipeline,
            backdrop_pipeline,
            down_pipeline,
            up_pipeline,
            globals_layout,
            texture_layout,
            pass_layout,
            sampler,
            globals,
            instances: FrameBuffer::new("instances", wgpu::BufferUsages::VERTEX, 4),
            records: FrameBuffer::new("records", wgpu::BufferUsages::STORAGE, SLOT_BYTES),
            properties: FrameBuffer::new("properties", wgpu::BufferUsages::STORAGE, PROPERTY_BYTES),
            levels: Vec::new(),
            levels_for: (0, 0),
            atlas_layout,
            atlases: HashMap::new(),
            empty_atlas,
        }
    }

    /// Applies an atlas update (see [`AtlasUpdate`] for the protocol):
    /// reallocates the texture on a new generation or size, then writes
    /// the region. Call before [`Renderer::render`] for the frame that
    /// uses the new glyphs.
    pub fn update_atlas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        update: &AtlasUpdate,
    ) -> Result<(), RenderError> {
        update.validate()?;
        let version = self
            .atlases
            .get(&update.atlas)
            .map(|a| (a.generation, a.size));
        if update.reallocates(version) {
            let (texture, bind_group) = create_atlas(device, &self.atlas_layout, update.size);
            self.atlases.insert(
                update.atlas,
                GpuAtlas {
                    generation: update.generation,
                    size: update.size,
                    texture,
                    bind_group,
                },
            );
        }
        let Some(atlas) = self.atlases.get(&update.atlas) else {
            return Err(RenderError::Atlas("atlas missing after allocation"));
        };
        let [x, y, w, h] = update.region;
        if w == 0 || h == 0 {
            return Ok(());
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &atlas.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &update.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Applies several atlas updates in order.
    pub fn update_atlases<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        updates: impl IntoIterator<Item = &'a AtlasUpdate>,
    ) -> Result<(), RenderError> {
        for u in updates {
            self.update_atlas(device, queue, u)?;
        }
        Ok(())
    }

    fn ensure_levels(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if self.levels_for == (w, h) && !self.levels.is_empty() {
            return;
        }
        self.levels = (1..=MAX_BLUR_LEVELS)
            .map(|l| create_level(device, level_size(w, l), level_size(h, l)))
            .collect();
        self.levels_for = (w, h);
    }

    /// Whether this renderer can blur backdrops on its device (found when
    /// it was created). When it is `Fallback`, draw the frame from
    /// [`DisplayList::without_backdrops`](crate::DisplayList::without_backdrops).
    pub fn blur_support(&self) -> &BlurSupport {
        &self.blur_support
    }

    /// Turns blur off as if the device had rejected it (tests, and the
    /// `LIVSPEIL_NO_BLUR` style overrides of hosts).
    pub fn force_blur_fallback(&mut self, reason: impl Into<String>) {
        self.blur_support = BlurSupport::Fallback(reason.into());
    }

    /// How a frame with backdrops would reach a target of this renderer's
    /// format and the given `usage` (see [`choose_blur_path`]).
    pub fn blur_path(&self, usage: wgpu::TextureUsages) -> BlurPath {
        choose_blur_path(&self.blur_support, self.format, usage)
    }

    /// Time the GPU work of later frames, or stop. Does nothing on a device
    /// without [`GPU_TIMER_FEATURES`](crate::GPU_TIMER_FEATURES).
    pub fn set_timing(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, on: bool) {
        match (on, self.timer.is_some()) {
            (true, false) => self.timer = GpuTimer::new(device, queue),
            (false, true) => self.timer = None,
            _ => {}
        }
    }

    /// The GPU time of a recent timed frame.
    #[must_use]
    pub fn gpu_time(&self) -> Option<std::time::Duration> {
        self.timer.as_ref().and_then(GpuTimer::latest)
    }

    /// Records and submits a frame into `target` (which must have
    /// [`wgpu::TextureUsages::RENDER_ATTACHMENT`], the renderer's format
    /// and the frame's size).
    ///
    /// A frame with backdrops draws straight into `target` when it also
    /// has `TEXTURE_BINDING` ([`Renderer::TARGET_USAGE`]); otherwise (Metal
    /// surface textures) into an internal texture that is then blitted
    /// into `target`. With [`BlurSupport::Fallback`] such a frame is
    /// refused: draw it from `DisplayList::without_backdrops` instead.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame,
        target: &wgpu::Texture,
    ) -> Result<(), RenderError> {
        let (w, h) = (frame.options.width, frame.options.height);
        if target.format() != self.format {
            return Err(RenderError::Target("format differs from the renderer's"));
        }
        if target.width() != w || target.height() != h {
            return Err(RenderError::Target("size differs from the frame's"));
        }
        if !frame.ops.iter().any(|op| matches!(op, Op::Backdrop { .. })) {
            return self.render_direct(device, queue, frame, target);
        }
        match self.blur_path(target.usage()) {
            BlurPath::Direct => self.render_direct(device, queue, frame, target),
            BlurPath::Fallback(reason) => Err(RenderError::BlurUnavailable(reason)),
            BlurPath::Offscreen => {
                let off = match self.offscreen.take() {
                    Some(t) if t.width() == w && t.height() == h => t,
                    _ => device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("offscreen frame"),
                        size: wgpu::Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: self.format,
                        usage: Renderer::TARGET_USAGE,
                        view_formats: &[],
                    }),
                };
                let result = self.render_direct(device, queue, frame, &off);
                if result.is_ok() {
                    self.blit(device, queue, &off, target);
                }
                self.offscreen = Some(off);
                result
            }
        }
    }

    /// Copies `src` into `dst` (same size and format) with a render pass.
    fn blit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &wgpu::Texture,
        dst: &wgpu::Texture,
    ) {
        let src_view = src.create_view(&wgpu::TextureViewDescriptor::default());
        let dst_view = dst.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &self.blit_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&src_view),
            }],
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("blit"),
        });
        {
            let mut pass = target_pass(
                &mut enc,
                &dst_view,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                "blit",
            );
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([enc.finish()]);
    }

    fn render_direct(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Frame,
        target: &wgpu::Texture,
    ) -> Result<(), RenderError> {
        let (w, h) = (frame.options.width, frame.options.height);
        let has_backdrops = frame.ops.iter().any(|op| matches!(op, Op::Backdrop { .. }));
        if has_backdrops
            && !target
                .usage()
                .contains(wgpu::TextureUsages::TEXTURE_BINDING)
        {
            return Err(RenderError::Target("backdrops need TEXTURE_BINDING usage"));
        }

        // Upload.
        let mut globals = Vec::with_capacity(16);
        for v in [w as f32, h as f32, frame.options.scale, 0.0] {
            globals.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&self.globals, 0, &globals);
        let instances =
            self.instances
                .upload(device, queue, bytemuck::cast_slice(&frame.instances))?;
        let records = self
            .records
            .upload(device, queue, bytemuck::cast_slice(&frame.records))?;
        let properties =
            self.properties
                .upload(device, queue, bytemuck::cast_slice(&frame.properties))?;
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &self.globals_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: records.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: properties.as_entire_binding(),
                },
            ],
        });
        let instances = instances.clone();

        // Blur passes and their uniforms.
        let mut passes: Vec<Vec<(bool, u32, PassParams)>> = Vec::new();
        for op in &frame.ops {
            if let Op::Backdrop {
                blur: Some(plan), ..
            } = op
            {
                passes.push(blur_passes(w, h, *plan));
            }
        }
        let total: usize = passes.iter().map(Vec::len).sum();
        let pass_buffer = if total > 0 {
            let mut data = vec![0u8; total * PASS_STRIDE as usize];
            for (i, (_, _, p)) in passes.iter().flatten().enumerate() {
                let vals = [
                    p.src_uv_scale[0],
                    p.src_uv_scale[1],
                    p.half_texel[0],
                    p.half_texel[1],
                    p.dst_scale,
                    0.0,
                    0.0,
                    0.0,
                ];
                for (k, v) in vals.iter().enumerate() {
                    let at = i * PASS_STRIDE as usize + k * 4;
                    data[at..at + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("blur passes"),
                size: data.len() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, &data);
            Some(buf)
        } else {
            None
        };
        if has_backdrops {
            self.ensure_levels(device, w, h);
        }

        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let level_views: Vec<wgpu::TextureView> = self
            .levels
            .iter()
            .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect();
        let level1_bg = level_views.first().map(|v| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("level 1"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(v),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        });

        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame"),
        });
        if let Some(timer) = &mut self.timer {
            timer.start(device, &mut enc);
        }
        let c = frame.clear;
        let mut load = wgpu::LoadOp::Clear(wgpu::Color {
            r: f64::from(c[0]),
            g: f64::from(c[1]),
            b: f64::from(c[2]),
            a: f64::from(c[3]),
        });
        let mut pass_index = 0usize;
        let mut blur_index = 0usize;
        let mut i = 0;
        let ops = &frame.ops;
        if ops.is_empty() || matches!(ops.first(), Some(Op::Backdrop { .. })) {
            let _ = target_pass(&mut enc, &target_view, load, "clear");
            load = wgpu::LoadOp::Load;
        }
        while i < ops.len() {
            match &ops[i] {
                Op::Draw { .. } => {
                    let mut pass = target_pass(&mut enc, &target_view, load, "primitives");
                    load = wgpu::LoadOp::Load;
                    pass.set_pipeline(&self.primitive_pipeline);
                    pass.set_bind_group(0, &globals_bg, &[]);
                    pass.set_vertex_buffer(0, instances.slice(..));
                    while let Some(Op::Draw {
                        start,
                        end,
                        atlas,
                        vertices,
                    }) = ops.get(i)
                    {
                        let atlas_bg = atlas
                            .and_then(|a| self.atlases.get(&a))
                            .map_or(&self.empty_atlas, |a| &a.bind_group);
                        pass.set_bind_group(2, atlas_bg, &[]);
                        pass.draw(0..*vertices, *start..*end);
                        i += 1;
                    }
                }
                Op::Backdrop { instance, blur } => {
                    if blur.is_some() {
                        let (Some(buf), Some(plan_passes)) = (&pass_buffer, passes.get(blur_index))
                        else {
                            return Err(RenderError::Device("blur passes missing".into()));
                        };
                        blur_index += 1;
                        for (down, dst, p) in plan_passes {
                            let src_view = if *down && *dst == 1 {
                                &target_view
                            } else {
                                let src = if *down { dst - 1 } else { dst + 1 };
                                level_view(&level_views, src)?
                            };
                            let dst_view = level_view(&level_views, *dst)?;
                            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("blur pass"),
                                layout: &self.pass_layout,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: wgpu::BindingResource::Buffer(
                                            wgpu::BufferBinding {
                                                buffer: buf,
                                                offset: pass_index as u64 * PASS_STRIDE,
                                                size: wgpu::BufferSize::new(32),
                                            },
                                        ),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::TextureView(src_view),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                                    },
                                ],
                            });
                            pass_index += 1;
                            let level_load = if *down {
                                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                            } else {
                                wgpu::LoadOp::Load
                            };
                            let mut pass = target_pass(&mut enc, dst_view, level_load, "kawase");
                            let [x0, y0, x1, y1] = p.region;
                            if x1 > x0 && y1 > y0 {
                                pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                                pass.set_pipeline(if *down {
                                    &self.down_pipeline
                                } else {
                                    &self.up_pipeline
                                });
                                pass.set_bind_group(0, &bg, &[]);
                                pass.draw(0..3, 0..1);
                            }
                        }
                    }
                    let Some(level1_bg) = &level1_bg else {
                        return Err(RenderError::Device("blur levels missing".into()));
                    };
                    let mut pass = target_pass(&mut enc, &target_view, load, "backdrop");
                    load = wgpu::LoadOp::Load;
                    pass.set_pipeline(&self.backdrop_pipeline);
                    pass.set_bind_group(0, &globals_bg, &[]);
                    pass.set_bind_group(1, level1_bg, &[]);
                    pass.set_vertex_buffer(0, instances.slice(..));
                    pass.draw(0..6, *instance..*instance + 1);
                    i += 1;
                }
            }
        }
        if let Some(timer) = &mut self.timer {
            timer.stop(&mut enc);
        }
        queue.submit([enc.finish()]);
        if let Some(timer) = &mut self.timer {
            timer.submitted();
        }
        Ok(())
    }
}

fn level_view(views: &[wgpu::TextureView], level: u32) -> Result<&wgpu::TextureView, RenderError> {
    views
        .get(level as usize - 1)
        .ok_or_else(|| RenderError::Device(format!("blur level {level} missing")))
}

fn target_pass<'e>(
    enc: &'e mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
    label: &str,
) -> wgpu::RenderPass<'e> {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}
