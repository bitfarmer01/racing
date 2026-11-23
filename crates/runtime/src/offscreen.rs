//! Offscreen one-frame render — the GPU-path proof.
//!
//! Renders one composed [`GameFrame`] into an offscreen wgpu texture (no
//! window, no surface), reads the pixels back, and encodes them to a PNG.
//! Like `render::gpu`, everything here touches a device and is exercised by
//! the `frame-proof` binary on capable hosts, never by tests.
//!
//! Adapter strategy: a fallback (software) adapter is requested first — the
//! proof exists precisely for GPU-less hosts. If none answers, any adapter
//! is requested second. Both failing is the explicit `NoAdapter` outcome,
//! never faked.

use std::future::Future;
use std::path::Path;

use render::{AtlasDims, SpriteInstance, SPRITE_SHADER};
use wgpu::util::DeviceExt;

use crate::frame::{GameFrame, RoadVertex};

/// Offscreen target format; matches the atlas texture's encoding so sprite
/// colors round-trip the way the `render` crate's pipeline expects.
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Everything the proof render needs: the composed frame and the packed
/// atlas pixels.
pub struct ProofScene<'a> {
    pub frame: &'a GameFrame,
    /// Row-major RGBA8 atlas pixels, `width * height * 4` bytes.
    pub atlas_rgba: &'a [u8],
    pub atlas_dims: AtlasDims,
}

/// A successful proof: the PNG's pixel dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderedProof {
    pub width: u32,
    pub height: u32,
}

/// Everything that can go wrong on the proof path. GPU-phase failures reuse
/// [`render::RenderError`] so the wording matches the render crate.
#[derive(Debug)]
pub enum ProofError {
    Gpu(render::RenderError),
    /// The readback buffer's callback reported a mapping failure.
    Readback(String),
    Image(image::ImageError),
    Io(std::io::Error),
}

impl std::fmt::Display for ProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gpu(e) => write!(f, "gpu: {e}"),
            Self::Readback(detail) => write!(f, "pixel readback failed: {detail}"),
            Self::Image(e) => write!(f, "png encode/save failed: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for ProofError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Gpu(e) => e.source(),
            Self::Image(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Readback(_) => None,
        }
    }
}

impl From<render::RenderError> for ProofError {
    fn from(e: render::RenderError) -> Self {
        Self::Gpu(e)
    }
}

impl From<image::ImageError> for ProofError {
    fn from(e: image::ImageError) -> Self {
        Self::Image(e)
    }
}

impl From<std::io::Error> for ProofError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Minimal `block_on` for the two wgpu setup futures, without a runtime
/// dependency: spin-poll with a no-op waker. wgpu's native setup completes
/// from internal service threads, so each re-poll observes the ready state;
/// this burns a little CPU for microseconds of setup and nothing else.
fn block_on<T>(fut: impl Future<Output = T>) -> T {
    let mut fut = std::pin::pin!(fut);
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    loop {
        if let std::task::Poll::Ready(value) = fut.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::yield_now();
    }
}

fn request_adapter(
    instance: &wgpu::Instance,
    force_fallback: bool,
) -> impl Future<Output = Result<wgpu::Adapter, wgpu::RequestAdapterError>> + '_ {
    instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        force_fallback_adapter: force_fallback,
    })
}

/// Uniform mirror of the WGSL `CameraUniform` struct in [`SPRITE_SHADER`]
/// (and the road shader below, which shares the layout).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    clip_matrix: [[f32; 4]; 4],
}

const ROAD_SHADER: &str = r#"
struct CameraUniform {
    clip_matrix: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;

struct VSOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) color: vec4<f32>) -> VSOut {
    return VSOut(camera.clip_matrix * vec4<f32>(position, 0.0, 1.0), color);
}

@fragment
fn fs_main(out: VSOut) -> @location(0) vec4<f32> {
    return out.color;
}
"#;

/// Renders one composed frame offscreen and writes it as a PNG to `out_path`.
/// Returns the PNG's pixel dimensions on success.
///
/// # Errors
/// `ProofError::Gpu(render::RenderError::NoAdapter)` when no adapter — not
/// even a software one — answers on this host. Nothing is written in that
/// case; the caller reports the failure honestly.
pub fn render_proof(scene: &ProofScene<'_>, out_path: &Path) -> Result<RenderedProof, ProofError> {
    let frame = scene.frame;
    let width = frame.camera.width as u32;
    let height = frame.camera.height as u32;

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    // Software adapter first: this proof must run on GPU-less hosts.
    let adapter = match block_on(request_adapter(&instance, true)) {
        Ok(adapter) => adapter,
        // No software backend either: take any adapter that exists.
        Err(_) => block_on(request_adapter(&instance, false))
            .map_err(|_| render::RenderError::NoAdapter)?,
    };
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("runtime.proof.device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .map_err(|_| render::RenderError::NoDevice)?;

    // Atlas texture — mirrors the render crate's upload (tightly packed RGBA8).
    let atlas_size = wgpu::Extent3d {
        width: scene.atlas_dims.width.max(1),
        height: scene.atlas_dims.height.max(1),
        depth_or_array_layers: 1,
    };
    let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("runtime.proof.atlas"),
        size: atlas_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &atlas_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        scene.atlas_rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(scene.atlas_dims.width.max(1) * 4),
            rows_per_image: Some(scene.atlas_dims.height.max(1)),
        },
        atlas_size,
    );
    let atlas_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("runtime.proof.sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..wgpu::SamplerDescriptor::default()
    });

    let camera_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("runtime.proof.camera"),
        contents: bytemuck::bytes_of(&CameraUniform {
            clip_matrix: frame.camera.clip_matrix(),
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    // Two bind-group layouts: flat-color road (uniform only) and textured
    // sprites (uniform + texture + sampler).
    let road_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("runtime.proof.road_bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let road_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("runtime.proof.road_bind"),
        layout: &road_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buf.as_entire_binding(),
        }],
    });

    let sprite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("runtime.proof.sprite_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let sprite_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("runtime.proof.sprite_bind"),
        layout: &sprite_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let road_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("runtime.proof.road_shader"),
        source: wgpu::ShaderSource::Wgsl(ROAD_SHADER.into()),
    });
    let sprite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("runtime.proof.sprite_shader"),
        source: wgpu::ShaderSource::Wgsl(SPRITE_SHADER.into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("runtime.proof.layout"),
        bind_group_layouts: &[&road_layout],
        push_constant_ranges: &[],
    });
    let road_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("runtime.proof.road_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &road_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<RoadVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: &road_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("runtime.proof.sprite_layout"),
        bind_group_layouts: &[&sprite_layout],
        push_constant_ranges: &[],
    });
    let sprite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("runtime.proof.sprite_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &sprite_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<SpriteInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x2,
                    1 => Float32x2,
                    2 => Float32,
                    3 => Float32x4,
                    4 => Float32x4
                ],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: &sprite_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    let road_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("runtime.proof.road_vertices"),
        contents: bytemuck::cast_slice(&frame.road.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let instances: &[SpriteInstance] = frame.batch.instances();
    let instance_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("runtime.proof.instances"),
        contents: bytemuck::cast_slice(instances),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("runtime.proof.target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("runtime.proof.encoder"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("runtime.proof.pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                // Grass-green sky-to-ground backdrop the road rides on.
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.18,
                        g: 0.52,
                        b: 0.24,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        // Road first, sprites on top.
        pass.set_pipeline(&road_pipeline);
        pass.set_bind_group(0, &road_bind, &[]);
        pass.set_vertex_buffer(0, road_buf.slice(..));
        pass.draw(0..frame.road.vertices.len() as u32, 0..1);
        pass.set_pipeline(&sprite_pipeline);
        pass.set_bind_group(0, &sprite_bind, &[]);
        pass.set_vertex_buffer(0, instance_buf.slice(..));
        pass.draw(0..4, 0..instances.len() as u32);
    }

    // Readback: rows are padded to the 256-byte COPY alignment, so the buffer
    // gets 256-aligned rows and the padding is stripped before PNG encode.
    let src_bytes_per_row = width * 4;
    let dst_bytes_per_row = src_bytes_per_row.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("runtime.proof.readback"),
        size: u64::from(dst_bytes_per_row * height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dst_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let (sender, receiver) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    device
        .poll(wgpu::PollType::Wait)
        .map_err(|e| ProofError::Readback(e.to_string()))?;
    match receiver.try_recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(ProofError::Readback(e.to_string())),
        Err(_) => return Err(ProofError::Readback("mapping never completed".into())),
    }

    let src_row = dst_bytes_per_row as usize;
    let row_len = src_bytes_per_row as usize;
    let mapped = readback.get_mapped_range(..);
    let mut rgba = Vec::with_capacity(row_len * height as usize);
    for row in 0..height as usize {
        rgba.extend_from_slice(&mapped[row * src_row..row * src_row + row_len]);
    }
    drop(mapped);
    readback.unmap();

    let image = image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| {
        ProofError::Readback(format!("readback payload is not {width}x{height}x4 bytes"))
    })?;

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    image.save(out_path)?;
    Ok(RenderedProof { width, height })
}
