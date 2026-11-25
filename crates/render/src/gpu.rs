//! GPU-PATH MODULE — every item here touches a wgpu instance, surface,
//! device, or queue. The development host has no GPU adapter, so nothing in
//! this module is exercised by CI; correctness rests on review and on the
//! pure modules it delegates to (`batch`, `camera`, `atlas`, `instance`,
//! `tier`). Kept deliberately thin: all decisions are made before this module
//! is reached, all it does is upload, encode, and present.

use wgpu::util::DeviceExt;

use crate::atlas::AtlasDims;
use crate::batch::SpriteBatch;
use crate::camera::OrthoCamera;
use crate::instance::SpriteInstance;
use crate::tier::QualityTier;

/// Sprite shader: one instanced quad per [`SpriteInstance`], tinted and
/// alpha-cut. Field-for-field the WGSL mirror of [`SpriteInstance`] — any
/// layout change must touch both.
pub const SPRITE_SHADER: &str = r#"
struct CameraUniform {
    clip_matrix: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(0) @binding(1) var atlas_texture: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

struct Instance {
    @location(0) position: vec2<f32>,
    @location(1) scale: vec2<f32>,
    @location(2) rotation: f32,
    @location(3) uv_rect: vec4<f32>,
    @location(4) tint: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

// Corner offsets in unit space (y down, matching screen pixels) and the
// matching atlas corners; indexed by vertex_index, no vertex buffer.
const CORNERS = array<vec2<f32>, 4>(
    vec2<f32>(-0.5, -0.5),
    vec2<f32>(0.5, -0.5),
    vec2<f32>(-0.5, 0.5),
    vec2<f32>(0.5, 0.5),
);
const UV_CORNERS = array<vec2<f32>, 4>(
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 1.0),
);

@vertex
fn vs_main(
    @builtin(vertex_index) vertex_index: u32,
    instance: Instance,
) -> VertexOutput {
    let corner = CORNERS[vertex_index];
    let c = cos(instance.rotation);
    let s = sin(instance.rotation);
    let rotated = vec2<f32>(
        corner.x * c - corner.y * s,
        corner.x * s + corner.y * c,
    );
    let screen = instance.position + rotated * instance.scale;
    let uv_min = vec2<f32>(instance.uv_rect.x, instance.uv_rect.y);
    let uv_span = vec2<f32>(
        instance.uv_rect.z - instance.uv_rect.x,
        instance.uv_rect.w - instance.uv_rect.y,
    );
    return VertexOutput(
        camera.clip_matrix * vec4<f32>(screen, 0.0, 1.0),
        uv_min + UV_CORNERS[vertex_index] * uv_span,
        instance.tint,
    );
}

@fragment
fn fs_main(out: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(atlas_texture, atlas_sampler, out.uv) * out.tint;
    // HD look: a smooth alpha ramp shapes the edge coverage so sprite
    // silhouettes blend softly at any scale — the pixel-art hard cut is
    // retired. The ramp's upper edge mirrors `sampling::ALPHA_RAMP_HI`
    // (const strings cannot share the value).
    let coverage = smoothstep(0.0, 0.06, color.a);
    if (coverage <= 0.0) {
        discard;
    }
    return vec4<f32>(color.rgb, color.a * coverage);
}
"#;

/// Road shader: flat-color triangle-strip quads (one trapezoid per road
/// strip). Field-for-field the WGSL mirror of [`RoadVertex`]. Shared verbatim
/// by the offscreen frame proof and the browser `GpuRenderer` road pass.
pub const ROAD_SHADER: &str = r#"
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

/// One road-mesh vertex: screen-space position plus a flat color. Owned here
/// so the shared GPU path can draw the road; `runtime` composes meshes of
/// these and re-exports the type.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RoadVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

/// Grass-green backdrop the road rides on. Field-for-field the clear color
/// of the offscreen frame proof, so the browser frame and the proof PNG
/// compose identically.
const BACKDROP_CLEAR: wgpu::Color = wgpu::Color {
    r: 0.18,
    g: 0.52,
    b: 0.24,
    a: 1.0,
};

/// Uniform mirror of the WGSL `CameraUniform` struct.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniformGpu {
    clip_matrix: [[f32; 4]; 4],
}

/// Atlas bytes ready for upload, plus their dimensions. The `game` layer
/// decodes the `assets` packer's PNG into raw RGBA and hands it here, so this
/// crate never needs an image decoder.
#[derive(Debug, Clone)]
pub struct AtlasUpload {
    pub dims: AtlasDims,
    /// Row-major RGBA8, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl AtlasUpload {
    /// Validates the byte count up front — a short slice would otherwise
    /// surface as a GPU-side fault no host test can catch.
    pub fn from_rgba(dims: AtlasDims, rgba: Vec<u8>) -> Result<Self, RenderError> {
        let expected = dims.width as usize * dims.height as usize * 4;
        if rgba.len() != expected {
            return Err(RenderError::AtlasSize {
                expected,
                got: rgba.len(),
            });
        }
        Ok(Self { dims, rgba })
    }
}

/// Everything that can go wrong on the GPU path.
#[derive(Debug)]
pub enum RenderError {
    Surface(wgpu::SurfaceError),
    /// `request_adapter` found nothing usable (headless host, unsupported
    /// browser).
    NoAdapter,
    /// An adapter existed but no device could be created from it.
    NoDevice,
    SurfaceCreation(wgpu::CreateSurfaceError),
    AtlasSize {
        expected: usize,
        got: usize,
    },
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Surface(e) => write!(f, "surface error: {e}"),
            Self::NoAdapter => write!(f, "no compatible GPU adapter"),
            Self::NoDevice => write!(f, "device creation failed"),
            Self::SurfaceCreation(e) => write!(f, "surface creation failed: {e}"),
            Self::AtlasSize { expected, got } => {
                write!(
                    f,
                    "atlas RGBA payload mismatch: expected {expected} bytes, got {got}"
                )
            }
        }
    }
}

impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Surface(e) => Some(e),
            Self::SurfaceCreation(e) => Some(e),
            _ => None,
        }
    }
}

impl From<wgpu::SurfaceError> for RenderError {
    fn from(e: wgpu::SurfaceError) -> Self {
        Self::Surface(e)
    }
}

impl From<wgpu::CreateSurfaceError> for RenderError {
    fn from(e: wgpu::CreateSurfaceError) -> Self {
        Self::SurfaceCreation(e)
    }
}

/// Instance-buffer vertex attributes — the Rust-side half of the WGSL
/// `Instance` struct layout (52 bytes, see `instance.rs`).
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
    2 => Float32,
    3 => Float32x4,
    4 => Float32x4,
];

/// A warmed-up GPU renderer: surface, one sprite pipeline, one road pipeline,
/// atlas texture, and fixed-capacity instance/road-vertex buffers. Built once
/// at load (pipelines warm behind the loading screen per the blueprint), then
/// only [`GpuRenderer::draw`] runs per frame.
pub struct GpuRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    road_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    /// The road and sprite pipelines share one camera uniform buffer; both
    /// WGSL `CameraUniform` structs are field-identical.
    road_bind_group: wgpu::BindGroup,
    camera_buf: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    capacity: usize,
    road_vertex_buf: wgpu::Buffer,
    road_vertex_capacity: usize,
    /// Six u16 indices per quad, pattern-filled once per capacity growth —
    /// the index pattern never varies with the mesh contents.
    road_index_buf: wgpu::Buffer,
    road_index_quads: usize,
    /// Multisampled attachment the frame resolves from. Recreated in
    /// [`GpuRenderer::draw`] whenever the frame size or format drifts.
    msaa_texture: wgpu::Texture,
    msaa_view: wgpu::TextureView,
    msaa_key: (u32, u32, wgpu::TextureFormat),
    /// MSAA sample count for the attachment: the tier's choice (item 14).
    sample_count: u32,
    /// The tier the renderer was built for — the source of the sample count
    /// and backing scale above, and of the instance cap.
    #[allow(dead_code)]
    tier: QualityTier,
}

impl GpuRenderer {
    /// Brings up the surface, device, and pipeline for `tier`.
    ///
    /// `size` is the initial swap-chain size in pixels; the caller updates it
    /// via [`wgpu::Surface::configure`] on resize (or re-creates the
    /// renderer). `surface_target` is consumed and leaked to `'static` — the
    /// renderer owns the window binding for its lifetime.
    pub async fn new(
        instance: &wgpu::Instance,
        surface_target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: (u32, u32),
        atlas: &AtlasUpload,
        tier: QualityTier,
    ) -> Result<Self, RenderError> {
        let surface = instance.create_surface(surface_target)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| RenderError::NoAdapter)?;
        // WebGL2-floor limits keep one pipeline source valid across both
        // backend builds; the WebGPU build simply runs under-featured.
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("render.device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|_| RenderError::NoDevice)?;

        let (_, view) = upload_atlas(&device, &queue, atlas);
        // HD direction: linear filtering — the atlas is high-resolution
        // shaded art now, and nearest sampling would re-pixelate it under
        // scale (reverses the pixel-art choice; policy in `sampling`).
        let filter = crate::sampling::atlas_filter_mode();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("render.atlas.sampler"),
            mag_filter: filter,
            min_filter: filter,
            ..wgpu::SamplerDescriptor::default()
        });

        let camera_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("render.camera_uniform"),
            contents: bytemuck::bytes_of(&CameraUniformGpu {
                clip_matrix: [[0.0; 4]; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("render.bind_group_layout"),
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render.bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let capacity = tier.max_instances();
        // Item 14: the tier drives the backing size and the MSAA sample
        // count. Log both at boot so a tier regression is visible in the
        // console where the game runs.
        let sample_count = tier.msaa_sample_count();
        let backing = tier.scaled_size(size.0, size.1);
        log_boot(tier, size, backing, sample_count);
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render.instances"),
            // The whole batch is one `write_buffer` per frame; the buffer is
            // sized for the tier cap once and never reallocated.
            size: (capacity * std::mem::size_of::<SpriteInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render.sprite_shader"),
            source: wgpu::ShaderSource::Wgsl(SPRITE_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render.pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render.sprite_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SpriteInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &INSTANCE_ATTRIBUTES,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface.get_capabilities(&adapter).formats[0],
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            // Four corners per instance, one instance per sprite: the vertex
            // shader fans the quad out from `vertex_index`, so a strip is the
            // correct topology — a list would render only the first triangle.
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                ..wgpu::MultisampleState::default()
            },
            multiview: None,
            cache: None,
        });

        // Road pipeline: flat-color indexed triangle list over the same
        // camera uniform — one six-index quad per mesh strip, so separate
        // quads never bridge into one another as they would in one long
        // strip. Opaque — the road always covers its pixels fully.
        let road_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render.road_shader"),
            source: wgpu::ShaderSource::Wgsl(ROAD_SHADER.into()),
        });
        let road_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("render.road_bind_group_layout"),
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
        let road_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("render.road_bind_group"),
            layout: &road_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });
        let road_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render.road_pipeline_layout"),
            bind_group_layouts: &[&road_bind_group_layout],
            push_constant_ranges: &[],
        });
        let road_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("render.road_pipeline"),
            layout: Some(&road_pipeline_layout),
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
                    format: surface.get_capabilities(&adapter).formats[0],
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                ..wgpu::MultisampleState::default()
            },
            multiview: None,
            cache: None,
        });

        let capabilities = surface.get_capabilities(&adapter);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: capabilities.formats[0],
            // Item 14: render at the tier's resolution scale — the canvas
            // presents upscaled, trading softness for frame rate on the
            // weak tiers.
            width: backing.0.max(1),
            height: backing.1.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: capabilities.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Road vertices start empty; the first draw sizes the buffer to the
        // mesh it is handed. Strip count is constant per frame once composed
        // (draw-distance / step), so steady state never reallocates.
        let road_vertex_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render.road_vertices"),
            size: 0,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let road_index_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("render.road_indices"),
            size: 0,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let msaa_key = (config.width, config.height, config.format);
        let (msaa_texture, msaa_view) = create_msaa_target(&device, msaa_key, sample_count);

        Ok(Self {
            surface,
            device,
            queue,
            pipeline,
            road_pipeline,
            bind_group,
            road_bind_group,
            camera_buf,
            instance_buf,
            capacity,
            road_vertex_buf,
            road_vertex_capacity: 0,
            road_index_buf,
            road_index_quads: 0,
            msaa_texture,
            msaa_view,
            msaa_key,
            sample_count,
            tier,
        })
    }

    /// The MSAA sample count the frame attachment renders with: the tier's
    /// choice (item 14) — 1x on the low-end WebGL2 floor, 4x elsewhere.
    pub fn msaa_sample_count(&self) -> u32 {
        self.sample_count
    }

    /// The backing size the render surface was configured at: the tier's
    /// resolution scale applied to the canvas size (item 14).
    pub fn backing_size(&self) -> (u32, u32) {
        (self.msaa_key.0, self.msaa_key.1)
    }

    /// Uploads the road mesh, then draws the frame: one flat-color
    /// triangle-strip pass for the road over the grass backdrop, then one
    /// instanced quad per sprite on top, and presents. Per-frame work is
    /// exactly: three `write_buffer`s, one encoder, one pass, one present —
    /// no allocations in steady state.
    ///
    /// `road` may be empty — the sprite-only path — in which case the pass
    /// presents the backdrop and sprites alone.
    pub fn draw(
        &mut self,
        batch: &SpriteBatch,
        road: &[RoadVertex],
        camera: &OrthoCamera,
    ) -> Result<(), RenderError> {
        let instances = batch.instances();
        debug_assert!(instances.len() <= self.capacity);
        let count = instances.len() as u32;

        let frame = self.surface.get_current_texture()?;
        self.queue.write_buffer(
            &self.camera_buf,
            0,
            bytemuck::bytes_of(&CameraUniformGpu {
                clip_matrix: camera.clip_matrix(),
            }),
        );
        if !instances.is_empty() {
            self.queue
                .write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(instances));
        }
        if road.len() > self.road_vertex_capacity {
            self.road_vertex_capacity = road.len();
            self.road_vertex_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("render.road_vertices"),
                size: std::mem::size_of_val(road) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !road.is_empty() {
            self.queue
                .write_buffer(&self.road_vertex_buf, 0, bytemuck::cast_slice(road));
        }
        let quad_count = road.len() / 4;
        debug_assert_eq!(quad_count * 4, road.len(), "road mesh must be whole quads");
        if quad_count > self.road_index_quads {
            self.road_index_quads = quad_count;
            self.road_index_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("render.road_indices"),
                size: (quad_count * std::mem::size_of::<[u16; 6]>()) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            // The index pattern depends only on quad count, never on mesh
            // contents, so it is uploaded once per capacity growth.
            self.queue.write_buffer(
                &self.road_index_buf,
                0,
                bytemuck::cast_slice(&quad_index_pattern(quad_count)),
            );
        }

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let frame_key = (
            frame.texture.width(),
            frame.texture.height(),
            frame.texture.format(),
        );
        // At 1x MSAA there is no resolve step: wgpu rejects a resolve pair
        // whose source and destination are both single-sample, so the low-end
        // tier renders straight into the frame texture (item 14).
        let multisampled = self.sample_count > 1;
        if multisampled && frame_key != self.msaa_key {
            let (texture, msaa_view) =
                create_msaa_target(&self.device, frame_key, self.sample_count);
            self.msaa_texture = texture;
            self.msaa_view = msaa_view;
            self.msaa_key = frame_key;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render.frame_encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render.frame_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    // Multisampled tiers render into the MSAA buffer and
                    // resolve into the frame; the 1x tier targets the frame
                    // view directly and must keep its contents.
                    view: if multisampled { &self.msaa_view } else { &view },
                    resolve_target: if multisampled { Some(&view) } else { None },
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BACKDROP_CLEAR),
                        // Resolved into the frame already; the multisample
                        // buffer itself is dead on store.
                        store: if multisampled {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // Road first, sprites on top — painter's order, as in the proof.
            if !road.is_empty() {
                pass.set_pipeline(&self.road_pipeline);
                pass.set_bind_group(0, &self.road_bind_group, &[]);
                pass.set_vertex_buffer(0, self.road_vertex_buf.slice(..));
                pass.set_index_buffer(self.road_index_buf.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..(quad_count * 6) as u32, 0, 0..1);
            }
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.instance_buf.slice(..));
            if count > 0 {
                pass.draw(0..4, 0..count);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

/// Logs the boot-time tier facts — tier, canvas size, backing size, MSAA
/// sample count (item 14) — to the browser console on wasm and to stderr on
/// the native test hosts. Never fails bring-up.
fn log_boot(tier: QualityTier, canvas: (u32, u32), backing: (u32, u32), msaa: u32) {
    let line =
        format!("render boot: tier={tier:?} canvas={canvas:?} backing={backing:?} msaa={msaa}x");
    #[cfg(target_arch = "wasm32")]
    web_sys::console::log_1(&line.into());
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{line}");
}

/// Builds the draw pattern for a road mesh of `quads` flat-colored quads:
/// six u16 indices per quad — two triangles over the four vertices pushed per
/// strip (`near-left, near-right, far-left, far-right`). Pure, so the pattern
/// is testable on the host without a device.
fn quad_index_pattern(quads: usize) -> Vec<u16> {
    let mut indices = Vec::with_capacity(quads * 6);
    for q in 0..quads as u16 {
        let base = q * 4;
        indices.extend_from_slice(&[base, base + 1, base + 2, base + 2, base + 1, base + 3]);
    }
    indices
}

/// Creates the multisampled render attachment the frame pass draws into and
/// resolves from: same size and format as the frame, `samples` samples.
fn create_msaa_target(
    device: &wgpu::Device,
    key: (u32, u32, wgpu::TextureFormat),
    samples: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let (width, height, format) = key;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render.msaa_target"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Uploads atlas RGBA bytes into a texture binding pair. Row padding: the
/// packer emits tightly packed rows, so `width * 4` must be a multiple of 256
/// (WebGL2's `UNPACK_ALIGNMENT` floor) — the atlas packer's power-of-two
/// widths satisfy that; a narrower atlas would need a staging copy here.
fn upload_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    atlas: &AtlasUpload,
) -> (wgpu::Texture, wgpu::TextureView) {
    let size = wgpu::Extent3d {
        width: atlas.dims.width.max(1),
        height: atlas.dims.height.max(1),
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render.atlas_texture"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &atlas.rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(atlas.dims.width.max(1) * 4),
            rows_per_image: Some(atlas.dims.height.max(1)),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
mod tests {
    use super::quad_index_pattern;

    #[test]
    fn one_quad_covers_both_triangles_over_its_four_vertices() {
        assert_eq!(quad_index_pattern(1), vec![0, 1, 2, 2, 1, 3]);
    }

    #[test]
    fn each_quad_indexes_only_its_own_vertices() {
        // No index ever reaches past its own quad, so a rumble strip can
        // never bridge triangles across into the neighboring quad.
        let quads = 7;
        for (i, index) in quad_index_pattern(quads).into_iter().enumerate() {
            let quad = i / 6;
            let base = (quad * 4) as u16;
            assert!(
                index >= base && index < base + 4,
                "quad {quad} indexed {index}"
            );
        }
    }

    #[test]
    fn empty_mesh_yields_no_indices() {
        assert!(quad_index_pattern(0).is_empty());
    }
}
