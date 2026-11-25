//! GPU side: pipelines, render targets and the per-frame pass sequence.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use game::theme::Theme;
use game::track::{Track, RUNOFF};

use crate::camera::{aabb_in_planes, Camera, Frustum};
use crate::mesh::Vertex;
use crate::style::{palette, track_style};
use crate::world::{Chunk, PropInstance, PropModel, WorldGeometry};

const COMMON: &str = include_str!("shaders/common.wgsl");
const WORLD: &str = include_str!("shaders/world.wgsl");
const BIKE: &str = include_str!("shaders/bike.wgsl");
const SKY: &str = include_str!("shaders/sky.wgsl");
const PARTICLE: &str = include_str!("shaders/particle.wgsl");
const POST: &str = include_str!("shaders/post.wgsl");

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const LDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BLOOM_LEVELS: usize = 5;
const SHADOW_HALF: f32 = 75.0;
const TERRAIN_DRAW: f32 = 6000.0;

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub shadow_size: u32,
    /// Internal resolution as a fraction of the output (dynamic resolution).
    pub render_scale: f32,
    pub bloom: bool,
    /// Ink outlines drawn from the depth buffer.
    pub ink: bool,
    /// Full-screen impact frames on crashes and hits (off for reduced motion).
    pub flashes: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shadow_size: 2048,
            render_scale: 1.0,
            bloom: true,
            ink: true,
            flashes: true,
        }
    }
}

/// One bike + rider to draw.
#[derive(Debug, Clone, Copy)]
pub struct BikeDraw {
    pub model: Mat4,
    pub paint: [f32; 3],
    pub leathers: [f32; 3],
    /// 0 solid .. 1 fully ghosted.
    pub ghost: f32,
    pub brake: f32,
    pub front_spin: f32,
    pub rear_spin: f32,
    pub steer: f32,
    /// Punch swing, -1 (left arm) .. 1 (right arm).
    pub punch: f32,
    /// Thrown-rider offset in bike space, and tumble angle.
    pub rider_offset: Vec3,
    pub rider_spin: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleKind {
    /// Alpha-blended, lit (smoke, dust, spray).
    Smoke,
    /// Additive, emissive (sparks, flames).
    Glow,
}

#[derive(Debug, Clone, Copy)]
pub struct Particle {
    pub kind: ParticleKind,
    pub pos: Vec3,
    pub size: f32,
    /// Linear colour and alpha (glow colours are HDR).
    pub color: [f32; 4],
    pub vel: Vec3,
    /// Streak length in seconds of velocity (0 = round).
    pub stretch: f32,
}

pub struct FrameInput<'a> {
    pub camera: Camera,
    pub time: f32,
    pub bikes: &'a [BikeDraw],
    pub particles: &'a [Particle],
    /// 0..5 red lights lit, 6 = green.
    pub start_lights: f32,
    /// 0..1, drives the speed post effects.
    pub boost: f32,
    /// Centre of the sharp-shadow region (usually the player).
    pub focus: Vec3,
    /// 0..1 impact-frame strength (a crash or a landed hit just happened).
    pub impact: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    shadow_mat: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    ground_bounce: [f32; 4],
    water_color: [f32; 4],
    grass: [f32; 4],
    grass_dry: [f32; 4],
    rock: [f32; 4],
    sand: [f32; 4],
    params: [f32; 4],
    params2: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PostUniform {
    texel: [f32; 4],
    params: [f32; 4],
    grade: [f32; 4],
    extra: [f32; 4],
    /// Ink rgb (linear), w: outline strength.
    ink: [f32; 4],
    /// Impact, speed lines, grain, camera near plane.
    fx: [f32; 4],
    /// Split-tone shadow tint, w: unused.
    shadow_tint: [f32; 4],
    /// Split-tone highlight tint, w: unused.
    high_tint: [f32; 4],
    /// Impact-frame hit tone (linear), w: unused.
    accent: [f32; 4],
    /// Impact-frame paper tone (linear), w: unused.
    paper: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct BikeInstance {
    m: [[f32; 4]; 4],
    paint: [f32; 4],
    paint2: [f32; 4],
    anim: [f32; 4],
    rider: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct ParticleInstance {
    pos: [f32; 4],
    color: [f32; 4],
    vel: [f32; 4],
}

fn v4(c: [f32; 3], w: f32) -> [f32; 4] {
    [c[0], c[1], c[2], w]
}

struct LoadedWorld {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    terrain: Vec<Chunk>,
    road: Vec<Chunk>,
    water: Option<Chunk>,
    props: Vec<PropModel>,
    bike: Chunk,
    theme: Theme,
    length: f32,
    half_width: f32,
    lamp_spacing: f32,
}

struct Targets {
    iw: u32,
    ih: u32,
    hdr: wgpu::TextureView,
    depth: wgpu::TextureView,
    ldr: wgpu::TextureView,
    down: Vec<(wgpu::TextureView, u32, u32)>,
    up: Vec<wgpu::TextureView>,
    /// prefilter, down x(L-1), up x(L-1), composite, fxaa
    post_groups: Vec<wgpu::BindGroup>,
}

struct GrowBuffer {
    buf: wgpu::Buffer,
    cap: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl GrowBuffer {
    fn new(
        device: &wgpu::Device,
        label: &'static str,
        cap: u64,
        usage: wgpu::BufferUsages,
    ) -> Self {
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: cap,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buf,
            cap,
            usage,
            label,
        }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if data.len() as u64 > self.cap {
            let cap = (data.len() as u64).next_power_of_two();
            *self = Self::new(device, self.label, cap, self.usage);
        }
        queue.write_buffer(&self.buf, 0, data);
    }
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    out_format: wgpu::TextureFormat,
    out_srgb: bool,
    hdr_format: wgpu::TextureFormat,
    hdr_scale: f32,
    width: u32,
    height: u32,
    pub settings: Settings,

    globals: wgpu::Buffer,
    shadow_globals: wgpu::Buffer,
    scene_bg: wgpu::BindGroup,
    shadow_bg: wgpu::BindGroup,
    shadow_view: wgpu::TextureView,

    world_pipe: wgpu::RenderPipeline,
    world_shadow_pipe: wgpu::RenderPipeline,
    prop_pipe: wgpu::RenderPipeline,
    prop_shadow_pipe: wgpu::RenderPipeline,
    bike_pipe: wgpu::RenderPipeline,
    bike_shadow_pipe: wgpu::RenderPipeline,
    sky_pipe: wgpu::RenderPipeline,
    particle_alpha_pipe: wgpu::RenderPipeline,
    particle_add_pipe: wgpu::RenderPipeline,
    prefilter_pipe: wgpu::RenderPipeline,
    down_pipe: wgpu::RenderPipeline,
    up_pipe: wgpu::RenderPipeline,
    composite_pipe: wgpu::RenderPipeline,
    fxaa_pipe: wgpu::RenderPipeline,

    post_bgl: wgpu::BindGroupLayout,
    composite_bgl: wgpu::BindGroupLayout,
    post_sampler: wgpu::Sampler,
    post_uniforms: Vec<wgpu::Buffer>,
    targets: Targets,

    world: Option<LoadedWorld>,
    prop_buf: GrowBuffer,
    bike_buf: GrowBuffer,
    particle_buf: GrowBuffer,
    quad_vb: wgpu::Buffer,

    scratch_props: Vec<PropInstance>,
    scratch_particles: Vec<ParticleInstance>,
    prop_draws: Vec<PropDraw>,
    scratch_far: Vec<PropInstance>,
    pub stats: FrameStats,
}

/// One prop model's instance ranges in the per-frame buffer: (byte offset, count).
struct PropDraw {
    model: usize,
    near: (u64, u32),
    far: (u64, u32),
    shadow: (u64, u32),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FrameStats {
    pub draws: u32,
    /// Triangles submitted to the main (camera) pass.
    pub triangles: u64,
    /// Of those, triangles from prop instances (trees, rocks, stands...).
    pub prop_triangles: u64,
    /// Triangles submitted to the sun shadow pass.
    pub shadow_triangles: u64,
    pub props: u32,
}

fn module(device: &wgpu::Device, label: &str, src: &str, with_common: bool) -> wgpu::ShaderModule {
    let full = if with_common {
        format!("{COMMON}\n{src}")
    } else {
        src.to_string()
    };
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(full.into()),
    })
}

struct PipeDesc<'a> {
    label: &'a str,
    layout: &'a wgpu::PipelineLayout,
    module: &'a wgpu::ShaderModule,
    vs: &'a str,
    fs: &'a str,
    buffers: &'a [wgpu::VertexBufferLayout<'a>],
    color: Option<(wgpu::TextureFormat, Option<wgpu::BlendState>)>,
    depth: Option<(bool, wgpu::CompareFunction)>,
    shadow_bias: bool,
}

fn pipeline(device: &wgpu::Device, d: PipeDesc) -> wgpu::RenderPipeline {
    let targets: Vec<Option<wgpu::ColorTargetState>> = d
        .color
        .map(|(format, blend)| {
            vec![Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })]
        })
        .unwrap_or_default();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(d.label),
        layout: Some(d.layout),
        vertex: wgpu::VertexState {
            module: d.module,
            entry_point: Some(d.vs),
            compilation_options: Default::default(),
            buffers: d.buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: d.depth.map(|(write, compare)| wgpu::DepthStencilState {
            format: DEPTH,
            depth_write_enabled: write,
            depth_compare: compare,
            stencil: Default::default(),
            bias: if d.shadow_bias {
                wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.5,
                    clamp: 0.0,
                }
            } else {
                Default::default()
            },
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: d.module,
            entry_point: Some(d.fs),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview: None,
        cache: None,
    })
}

const PROP_ATTRS: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![5 => Float32x4, 6 => Float32x4];
const BIKE_ATTRS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
    5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4,
    9 => Float32x4, 10 => Float32x4, 11 => Float32x4, 12 => Float32x4,
];
const PARTICLE_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![1 => Float32x4, 2 => Float32x4, 3 => Float32x4];
const CORNER_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];

fn instance_layout(
    stride: usize,
    attrs: &'static [wgpu::VertexAttribute],
) -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: stride as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: attrs,
    }
}

impl Renderer {
    pub fn new(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        out_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        settings: Settings,
    ) -> Self {
        let f16 = adapter.get_texture_format_features(wgpu::TextureFormat::Rgba16Float);
        let hdr_ok = f16
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            && f16
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE);
        let (hdr_format, hdr_scale) = if hdr_ok {
            (wgpu::TextureFormat::Rgba16Float, 1.0)
        } else {
            (wgpu::TextureFormat::Rgba8Unorm, 0.125)
        };

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                uniform_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let shadow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow"),
            entries: &[uniform_entry],
        });
        let size = settings.shadow_size;
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_tex.create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: &scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });
        let shadow_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow"),
            layout: &shadow_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_globals.as_entire_binding(),
            }],
        });

        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[&scene_bgl],
            push_constant_ranges: &[],
        });
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[&shadow_bgl],
            push_constant_ranges: &[],
        });

        let world_mod = module(device, "world", WORLD, true);
        let bike_mod = module(device, "bike", BIKE, true);
        let sky_mod = module(device, "sky", SKY, true);
        let particle_mod = module(device, "particle", PARTICLE, true);
        let post_mod = module(device, "post", POST, false);

        let vertex = Vertex::layout();
        let prop_inst = instance_layout(std::mem::size_of::<PropInstance>(), &PROP_ATTRS);
        let bike_inst = instance_layout(std::mem::size_of::<BikeInstance>(), &BIKE_ATTRS);
        let particle_inst =
            instance_layout(std::mem::size_of::<ParticleInstance>(), &PARTICLE_ATTRS);
        let corner = wgpu::VertexBufferLayout {
            array_stride: 8,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &CORNER_ATTRS,
        };
        let opaque = Some((true, wgpu::CompareFunction::Greater));
        let shadow_depth = Some((true, wgpu::CompareFunction::LessEqual));
        let hdr = Some((hdr_format, None));

        let mk = |label,
                  layout,
                  module,
                  vs,
                  fs,
                  buffers: &[wgpu::VertexBufferLayout],
                  color,
                  depth,
                  bias| {
            pipeline(
                device,
                PipeDesc {
                    label,
                    layout,
                    module,
                    vs,
                    fs,
                    buffers,
                    color,
                    depth,
                    shadow_bias: bias,
                },
            )
        };
        let world_pipe = mk(
            "world",
            &scene_layout,
            &world_mod,
            "vs_static",
            "fs_main",
            std::slice::from_ref(&vertex),
            hdr,
            opaque,
            false,
        );
        let world_shadow_pipe = mk(
            "world shadow",
            &shadow_layout,
            &world_mod,
            "vs_static_shadow",
            "fs_shadow",
            std::slice::from_ref(&vertex),
            None,
            shadow_depth,
            true,
        );
        let prop_pipe = mk(
            "props",
            &scene_layout,
            &world_mod,
            "vs_prop",
            "fs_main",
            &[vertex.clone(), prop_inst.clone()],
            hdr,
            opaque,
            false,
        );
        let prop_shadow_pipe = mk(
            "props shadow",
            &shadow_layout,
            &world_mod,
            "vs_prop_shadow",
            "fs_shadow",
            &[vertex.clone(), prop_inst],
            None,
            shadow_depth,
            true,
        );
        let bike_pipe = mk(
            "bikes",
            &scene_layout,
            &bike_mod,
            "vs_main",
            "fs_main",
            &[vertex.clone(), bike_inst.clone()],
            hdr,
            opaque,
            false,
        );
        let bike_shadow_pipe = mk(
            "bikes shadow",
            &shadow_layout,
            &bike_mod,
            "vs_shadow",
            "fs_shadow",
            &[vertex, bike_inst],
            None,
            shadow_depth,
            true,
        );
        let sky_pipe = mk(
            "sky",
            &scene_layout,
            &sky_mod,
            "vs_main",
            "fs_main",
            &[],
            hdr,
            Some((false, wgpu::CompareFunction::GreaterEqual)),
            false,
        );
        let premul = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let particle_alpha_pipe = mk(
            "particles alpha",
            &scene_layout,
            &particle_mod,
            "vs_main",
            "fs_alpha",
            &[corner.clone(), particle_inst.clone()],
            Some((hdr_format, Some(premul))),
            Some((false, wgpu::CompareFunction::Greater)),
            false,
        );
        let particle_add_pipe = mk(
            "particles add",
            &scene_layout,
            &particle_mod,
            "vs_main",
            "fs_add",
            &[corner, particle_inst],
            Some((hdr_format, Some(additive))),
            Some((false, wgpu::CompareFunction::Greater)),
            false,
        );

        let post_entries = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &post_entries,
        });
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[&post_bgl],
            push_constant_ranges: &[],
        });
        // The composite also reads scene depth for the ink outlines.
        let composite_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite"),
            entries: &[
                post_entries[0],
                post_entries[1],
                post_entries[2],
                post_entries[3],
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // Bound as plain float, not depth, so the GLSL
                        // backend (WebGL2) can texelFetch it.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("composite"),
            bind_group_layouts: &[&composite_bgl],
            push_constant_ranges: &[],
        });
        let full = |label, fs, format| {
            mk(
                label,
                &post_layout,
                &post_mod,
                "vs_full",
                fs,
                &[],
                Some((format, None)),
                None,
                false,
            )
        };
        let prefilter_pipe = full("bloom prefilter", "fs_prefilter", hdr_format);
        let down_pipe = full("bloom down", "fs_down", hdr_format);
        let up_pipe = full("bloom up", "fs_up", hdr_format);
        let composite_pipe = mk(
            "composite",
            &composite_layout,
            &post_mod,
            "vs_full",
            "fs_composite",
            &[],
            Some((LDR, None)),
            None,
            false,
        );
        let fxaa_pipe = full("fxaa", "fs_fxaa", out_format);

        let post_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let post_uniforms: Vec<wgpu::Buffer> = (0..(2 * BLOOM_LEVELS + 1))
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("post uniform"),
                    size: std::mem::size_of::<PostUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();

        let corners: [[f32; 2]; 6] = [
            [-1.0, -1.0],
            [1.0, -1.0],
            [1.0, 1.0],
            [-1.0, -1.0],
            [1.0, 1.0],
            [-1.0, 1.0],
        ];
        let quad_vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particle quad"),
            contents: bytemuck::cast_slice(&corners),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let out_srgb = out_format.is_srgb();
        let (iw, ih) = scaled(width, height, settings.render_scale);
        let targets = create_targets(
            device,
            &post_bgl,
            &composite_bgl,
            &post_sampler,
            &post_uniforms,
            hdr_format,
            iw,
            ih,
        );
        let vb = wgpu::BufferUsages::VERTEX;
        let r = Self {
            device: device.clone(),
            queue: queue.clone(),
            out_format,
            out_srgb,
            hdr_format,
            hdr_scale,
            width,
            height,
            settings,
            globals,
            shadow_globals,
            scene_bg,
            shadow_bg,
            shadow_view,
            world_pipe,
            world_shadow_pipe,
            prop_pipe,
            prop_shadow_pipe,
            bike_pipe,
            bike_shadow_pipe,
            sky_pipe,
            particle_alpha_pipe,
            particle_add_pipe,
            prefilter_pipe,
            down_pipe,
            up_pipe,
            composite_pipe,
            fxaa_pipe,
            post_bgl,
            composite_bgl,
            post_sampler,
            post_uniforms,
            targets,
            world: None,
            prop_buf: GrowBuffer::new(device, "prop instances", 1 << 18, vb),
            bike_buf: GrowBuffer::new(device, "bike instances", 4096, vb),
            particle_buf: GrowBuffer::new(device, "particles", 1 << 16, vb),
            quad_vb,
            scratch_props: Vec::new(),
            scratch_particles: Vec::new(),
            prop_draws: Vec::new(),
            scratch_far: Vec::new(),
            stats: FrameStats::default(),
        };
        r.write_static_post();
        r
    }

    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.out_format
    }

    pub fn hdr_format(&self) -> wgpu::TextureFormat {
        self.hdr_format
    }

    pub fn internal_size(&self) -> (u32, u32) {
        (self.targets.iw, self.targets.ih)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.rebuild_targets();
    }

    /// Sets the internal resolution scale; targets are rebuilt only when the
    /// pixel size actually changes.
    pub fn set_render_scale(&mut self, scale: f32) {
        let scale = scale.clamp(0.35, 1.0);
        self.settings.render_scale = scale;
        let (iw, ih) = scaled(self.width, self.height, scale);
        if (iw, ih) != (self.targets.iw, self.targets.ih) {
            self.rebuild_targets();
        }
    }

    fn rebuild_targets(&mut self) {
        let (iw, ih) = scaled(self.width, self.height, self.settings.render_scale);
        self.targets = create_targets(
            &self.device,
            &self.post_bgl,
            &self.composite_bgl,
            &self.post_sampler,
            &self.post_uniforms,
            self.hdr_format,
            iw,
            ih,
        );
        self.write_static_post();
    }

    fn write_static_post(&self) {
        let t = &self.targets;
        let write = |i: usize, sw: u32, sh: u32| {
            let u = PostUniform {
                texel: [1.0 / sw as f32, 1.0 / sh as f32, 0.0, 0.0],
                params: [0.0; 4],
                grade: [1.0, 1.0, 1.0, 0.0],
                extra: [1.6, 0.8, 0.0, 1.0 / self.hdr_scale],
                ..Zeroable::zeroed()
            };
            self.queue
                .write_buffer(&self.post_uniforms[i], 0, bytemuck::bytes_of(&u));
        };
        // Prefilter reads the full-res HDR; each down reads the level above.
        write(0, t.iw, t.ih);
        for l in 1..BLOOM_LEVELS {
            let (_, w, h) = t.down[l - 1];
            write(l, w, h);
        }
        // Up passes read the level below.
        for l in 0..BLOOM_LEVELS - 1 {
            let (_, w, h) = t.down[l + 1];
            write(BLOOM_LEVELS + l, w, h);
        }
    }

    pub fn load_world(&mut self, track: &Track) {
        let geo = WorldGeometry::build(track);
        let vb = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("world vertices"),
                contents: bytemuck::cast_slice(&geo.mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let ib = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("world indices"),
                contents: bytemuck::cast_slice(&geo.mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        let count = (track.length / 40.0) as usize;
        self.world = Some(LoadedWorld {
            vb,
            ib,
            terrain: geo.terrain,
            road: geo.road,
            water: geo.water,
            props: geo.props,
            bike: geo.bike,
            theme: track.def.theme,
            length: track.length,
            half_width: track.half_width,
            lamp_spacing: track.length / count.max(1) as f32,
        });
    }

    fn globals_for(
        &self,
        w: &LoadedWorld,
        view_proj: Mat4,
        shadow_mat: Mat4,
        input: &FrameInput,
    ) -> Globals {
        let t = &w.theme;
        let sun = Vec3::from(t.sun_direction()).normalize();
        let lamp = if t.lamps {
            ((0.42 - t.sun_elevation) * 2.6).clamp(0.15, 1.0)
        } else {
            0.0
        };
        let cam = input.camera.pos;
        let texel = 1.0 / self.settings.shadow_size as f32;
        Globals {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            shadow_mat: shadow_mat.to_cols_array_2d(),
            cam_pos: [cam.x, cam.y, cam.z, input.time],
            sun_dir: [sun.x, sun.y, sun.z, 1.0],
            sun_color: v4(t.sun_color, self.hdr_scale),
            sky_zenith: v4(t.sky_zenith, t.fog_density),
            sky_horizon: v4(t.sky_horizon, t.cloud_cover),
            ground_bounce: v4(t.ground_bounce, t.water_level),
            water_color: v4(t.water_color, t.snow_line),
            grass: v4(t.grass, 0.0),
            grass_dry: v4(t.grass_dry, 0.0),
            rock: v4(t.rock, 0.0),
            sand: v4(t.sand, w.length),
            params: [
                input.start_lights,
                lamp,
                w.lamp_spacing,
                w.half_width + RUNOFF + 1.2 - 2.4,
            ],
            params2: [w.half_width, 1.0, texel, w.length - 120.0],
        }
    }

    fn shadow_matrix(&self, theme: &Theme, focus: Vec3) -> Mat4 {
        let sun = Vec3::from(theme.sun_direction()).normalize();
        let up = if sun.y.abs() > 0.95 { Vec3::Z } else { Vec3::Y };
        let view = Mat4::look_at_rh(Vec3::ZERO, -sun, up);
        // Snap the centre to whole shadow texels so edges don't crawl.
        let texel = 2.0 * SHADOW_HALF / self.settings.shadow_size as f32;
        let c = view.transform_point3(focus);
        let c = Vec3::new(
            (c.x / texel).round() * texel,
            (c.y / texel).round() * texel,
            c.z,
        );
        let proj = Mat4::orthographic_rh(
            c.x - SHADOW_HALF,
            c.x + SHADOW_HALF,
            c.y - SHADOW_HALF,
            c.y + SHADOW_HALF,
            -c.z - 400.0,
            -c.z + 250.0,
        );
        proj * view
    }

    /// Renders one frame into `target` (a view of the output surface).
    pub fn render(&mut self, target: &wgpu::TextureView, input: &FrameInput) {
        let Some(w) = self.world.take() else {
            return;
        };
        let (iw, ih) = (self.targets.iw, self.targets.ih);
        let aspect = iw as f32 / ih as f32;
        let view_proj = input.camera.view_proj(aspect);
        let frustum = Frustum::from_view_proj(view_proj);
        let shadow_mat = self.shadow_matrix(&w.theme, input.focus);
        let shadow_planes = Frustum::from_ortho(shadow_mat);
        let g = self.globals_for(&w, view_proj, shadow_mat, input);
        let mut sg = g;
        sg.view_proj = shadow_mat.to_cols_array_2d();
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&g));
        self.queue
            .write_buffer(&self.shadow_globals, 0, bytemuck::bytes_of(&sg));
        let cam = input.camera.pos;
        let mut stats = FrameStats::default();

        // Prop instances: visible to the camera, then to the sun.
        self.scratch_props.clear();
        self.prop_draws.clear();
        let stride = std::mem::size_of::<PropInstance>() as u64;
        for (mi, model) in w.props.iter().enumerate() {
            let start = self.scratch_props.len();
            self.scratch_far.clear();
            let d2 = model.draw_distance * model.draw_distance;
            let lod2 = if model.lod.is_some() {
                model.lod_distance * model.lod_distance
            } else {
                f32::MAX
            };
            for cell in &model.cells {
                let centre = (cell.lo + cell.hi) * 0.5;
                if centre.distance_squared(cam) > d2 * 1.1
                    || !frustum.aabb_visible(cell.lo, cell.hi)
                {
                    continue;
                }
                for inst in &cell.instances {
                    let p = Vec3::new(inst.pos_yaw[0], inst.pos_yaw[1], inst.pos_yaw[2]);
                    let fade = d2 * (0.6 + 0.4 * inst.params[1]);
                    let dd = p.distance_squared(cam);
                    if dd < lod2 {
                        self.scratch_props.push(*inst);
                    } else if dd < fade {
                        self.scratch_far.push(*inst);
                    }
                }
            }
            let near = (start, self.scratch_props.len() - start);
            let far_start = self.scratch_props.len();
            self.scratch_props.extend_from_slice(&self.scratch_far);
            let far = (far_start, self.scratch_far.len());
            let sstart = self.scratch_props.len();
            if model.casts_shadow {
                for cell in &model.cells {
                    if !aabb_in_planes(&shadow_planes, cell.lo, cell.hi) {
                        continue;
                    }
                    self.scratch_props.extend_from_slice(&cell.instances);
                }
            }
            let shadow = (sstart, self.scratch_props.len() - sstart);
            let at = |r: (usize, usize)| (r.0 as u64 * stride, r.1 as u32);
            self.prop_draws.push(PropDraw {
                model: mi,
                near: at(near),
                far: at(far),
                shadow: at(shadow),
            });
            stats.props += (near.1 + far.1) as u32;
        }
        self.prop_buf.write(
            &self.device,
            &self.queue,
            bytemuck::cast_slice(&self.scratch_props),
        );

        // Bikes.
        let bikes: Vec<BikeInstance> = input
            .bikes
            .iter()
            .map(|b| BikeInstance {
                m: b.model.to_cols_array_2d(),
                paint: v4(b.paint, b.ghost),
                paint2: v4(b.leathers, b.brake),
                anim: [b.front_spin, b.rear_spin, b.steer, b.punch],
                rider: [
                    b.rider_offset.x,
                    b.rider_offset.y,
                    b.rider_offset.z,
                    b.rider_spin,
                ],
            })
            .collect();
        self.bike_buf
            .write(&self.device, &self.queue, bytemuck::cast_slice(&bikes));

        // Particles: smoke sorted back to front, glow after it.
        self.scratch_particles.clear();
        let mut smoke: Vec<&Particle> = input
            .particles
            .iter()
            .filter(|p| p.kind == ParticleKind::Smoke)
            .collect();
        smoke.sort_by(|a, b| {
            b.pos
                .distance_squared(cam)
                .partial_cmp(&a.pos.distance_squared(cam))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let to_inst = |p: &Particle| {
            let mut c = p.color;
            if p.kind == ParticleKind::Smoke {
                c[0] *= c[3];
                c[1] *= c[3];
                c[2] *= c[3];
            }
            ParticleInstance {
                pos: [p.pos.x, p.pos.y, p.pos.z, p.size],
                color: c,
                vel: [p.vel.x, p.vel.y, p.vel.z, p.stretch],
            }
        };
        self.scratch_particles
            .extend(smoke.iter().map(|p| to_inst(p)));
        let n_smoke = self.scratch_particles.len() as u32;
        self.scratch_particles.extend(
            input
                .particles
                .iter()
                .filter(|p| p.kind == ParticleKind::Glow)
                .map(to_inst),
        );
        let n_glow = self.scratch_particles.len() as u32 - n_smoke;
        self.particle_buf.write(
            &self.device,
            &self.queue,
            bytemuck::cast_slice(&self.scratch_particles),
        );

        // Post uniforms that change per frame.
        let t = &w.theme;
        let style = track_style(t);
        let comp = PostUniform {
            texel: [1.0 / iw as f32, 1.0 / ih as f32, iw as f32, ih as f32],
            params: [
                t.exposure,
                if self.settings.bloom { 0.55 } else { 0.0 },
                0.32,
                input.boost.clamp(0.0, 1.0),
            ],
            grade: [t.grade[0], t.grade[1], t.grade[2], 0.0],
            extra: [1.6, 0.8, input.time.fract(), 1.0 / self.hdr_scale],
            ink: v4(
                style.ink.linear(),
                if self.settings.ink {
                    style.ink_strength
                } else {
                    0.0
                },
            ),
            fx: [
                if self.settings.flashes {
                    input.impact.clamp(0.0, 1.0)
                } else {
                    0.0
                },
                input.boost.clamp(0.0, 1.0),
                style.grain,
                input.camera.near,
            ],
            shadow_tint: v4(style.shadow_tint, 0.0),
            high_tint: v4(style.highlight_tint, 0.0),
            accent: v4(palette::HOT.linear(), 0.0),
            paper: v4(palette::PAPER.linear(), 0.0),
        };
        self.queue.write_buffer(
            &self.post_uniforms[2 * BLOOM_LEVELS - 1],
            0,
            bytemuck::bytes_of(&comp),
        );
        let fx = PostUniform {
            texel: [
                1.0 / iw as f32,
                1.0 / ih as f32,
                self.width as f32,
                self.height as f32,
            ],
            params: [0.0; 4],
            grade: [1.0, 1.0, 1.0, if self.out_srgb { 1.0 } else { 0.0 }],
            extra: [0.0; 4],
            ..Zeroable::zeroed()
        };
        self.queue.write_buffer(
            &self.post_uniforms[2 * BLOOM_LEVELS],
            0,
            bytemuck::bytes_of(&fx),
        );

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        // Shadow pass.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.shadow_bg, &[]);
            pass.set_vertex_buffer(0, w.vb.slice(..));
            pass.set_index_buffer(w.ib.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_pipeline(&self.world_shadow_pipe);
            // Road chunks carry the barriers; terrain self-shadowing inside the
            // 150 m shadow window is invisible at this resolution, so skip it.
            for c in &w.road {
                if aabb_in_planes(&shadow_planes, c.lo, c.hi) {
                    pass.draw_indexed(c.first..c.first + c.count, 0, 0..1);
                    stats.draws += 1;
                    stats.shadow_triangles += (c.count / 3) as u64;
                }
            }
            pass.set_pipeline(&self.prop_shadow_pipe);
            for d in &self.prop_draws {
                let (off, n) = d.shadow;
                if n == 0 {
                    continue;
                }
                let pm = &w.props[d.model];
                let m = pm.lod.as_ref().unwrap_or(&pm.mesh);
                pass.set_vertex_buffer(1, self.prop_buf.buf.slice(off..));
                pass.draw_indexed(m.first..m.first + m.count, 0, 0..n);
                stats.draws += 1;
                stats.shadow_triangles += (m.count / 3) as u64 * n as u64;
            }
            if !bikes.is_empty() {
                pass.set_pipeline(&self.bike_shadow_pipe);
                pass.set_vertex_buffer(1, self.bike_buf.buf.slice(..));
                pass.draw_indexed(
                    w.bike.first..w.bike.first + w.bike.count,
                    0,
                    0..bikes.len() as u32,
                );
                stats.draws += 1;
            }
        }

        // Main HDR pass.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.hdr,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.scene_bg, &[]);
            pass.set_vertex_buffer(0, w.vb.slice(..));
            pass.set_index_buffer(w.ib.slice(..), wgpu::IndexFormat::Uint32);
            // Bikes first: they cover the most pixels close up (early-z).
            if !bikes.is_empty() {
                pass.set_pipeline(&self.bike_pipe);
                pass.set_vertex_buffer(1, self.bike_buf.buf.slice(..));
                pass.draw_indexed(
                    w.bike.first..w.bike.first + w.bike.count,
                    0,
                    0..bikes.len() as u32,
                );
                stats.draws += 1;
                stats.triangles += (w.bike.count / 3) as u64 * bikes.len() as u64;
            }
            pass.set_pipeline(&self.world_pipe);
            let draw_chunk =
                |pass: &mut wgpu::RenderPass, stats: &mut FrameStats, c: &Chunk, max: f32| {
                    let centre = (c.lo + c.hi) * 0.5;
                    let r = (c.hi - c.lo).length() * 0.5;
                    if centre.distance(cam) - r > max || !frustum.aabb_visible(c.lo, c.hi) {
                        return;
                    }
                    pass.draw_indexed(c.first..c.first + c.count, 0, 0..1);
                    stats.draws += 1;
                    stats.triangles += (c.count / 3) as u64;
                };
            for c in &w.road {
                draw_chunk(&mut pass, &mut stats, c, 3000.0);
            }
            pass.set_pipeline(&self.prop_pipe);
            for d in &self.prop_draws {
                let pm = &w.props[d.model];
                for ((off, n), m) in [
                    (d.near, &pm.mesh),
                    (d.far, pm.lod.as_ref().unwrap_or(&pm.mesh)),
                ] {
                    if n == 0 {
                        continue;
                    }
                    pass.set_vertex_buffer(1, self.prop_buf.buf.slice(off..));
                    pass.draw_indexed(m.first..m.first + m.count, 0, 0..n);
                    stats.draws += 1;
                    stats.triangles += (m.count / 3) as u64 * n as u64;
                    stats.prop_triangles += (m.count / 3) as u64 * n as u64;
                }
            }
            pass.set_pipeline(&self.world_pipe);
            for c in &w.terrain {
                draw_chunk(&mut pass, &mut stats, c, TERRAIN_DRAW);
            }
            if let Some(c) = &w.water {
                draw_chunk(&mut pass, &mut stats, c, f32::MAX);
            }
            pass.set_pipeline(&self.sky_pipe);
            pass.draw(0..3, 0..1);
            if n_smoke > 0 {
                pass.set_pipeline(&self.particle_alpha_pipe);
                pass.set_vertex_buffer(0, self.quad_vb.slice(..));
                pass.set_vertex_buffer(1, self.particle_buf.buf.slice(..));
                pass.draw(0..6, 0..n_smoke);
            }
            if n_glow > 0 {
                let off = n_smoke as u64 * std::mem::size_of::<ParticleInstance>() as u64;
                pass.set_pipeline(&self.particle_add_pipe);
                pass.set_vertex_buffer(0, self.quad_vb.slice(..));
                pass.set_vertex_buffer(1, self.particle_buf.buf.slice(off..));
                pass.draw(0..6, 0..n_glow);
            }
        }

        // Bloom.
        let t = &self.targets;
        let full_pass = |enc: &mut wgpu::CommandEncoder,
                         pipe: &wgpu::RenderPipeline,
                         bg: &wgpu::BindGroup,
                         view: &wgpu::TextureView| {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        };
        full_pass(
            &mut enc,
            &self.prefilter_pipe,
            &t.post_groups[0],
            &t.down[0].0,
        );
        for l in 1..BLOOM_LEVELS {
            full_pass(&mut enc, &self.down_pipe, &t.post_groups[l], &t.down[l].0);
        }
        for l in (0..BLOOM_LEVELS - 1).rev() {
            full_pass(
                &mut enc,
                &self.up_pipe,
                &t.post_groups[BLOOM_LEVELS + l],
                &t.up[l],
            );
        }
        full_pass(
            &mut enc,
            &self.composite_pipe,
            &t.post_groups[2 * BLOOM_LEVELS - 1],
            &t.ldr,
        );
        full_pass(
            &mut enc,
            &self.fxaa_pipe,
            &t.post_groups[2 * BLOOM_LEVELS],
            target,
        );
        stats.draws += 2 * BLOOM_LEVELS as u32 + 1;

        self.queue.submit(Some(enc.finish()));
        self.stats = stats;
        self.world = Some(w);
    }
}

fn scaled(w: u32, h: u32, s: f32) -> (u32, u32) {
    (
        ((w as f32 * s).round() as u32).max(16),
        ((h as f32 * s).round() as u32).max(16),
    )
}

#[allow(clippy::too_many_arguments)]
fn create_targets(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    composite_bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    uniforms: &[wgpu::Buffer],
    hdr_format: wgpu::TextureFormat,
    iw: u32,
    ih: u32,
) -> Targets {
    let tex = |label: &str, w: u32, h: u32, format: wgpu::TextureFormat| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w.max(1),
                    height: h.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let hdr = tex("hdr", iw, ih, hdr_format);
    let depth = tex("depth", iw, ih, DEPTH);
    let ldr = tex("ldr", iw, ih, LDR);
    let mut down = Vec::new();
    let (mut w, mut h) = (iw, ih);
    for _ in 0..BLOOM_LEVELS {
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        down.push((tex("bloom down", w, h, hdr_format), w, h));
    }
    let up: Vec<wgpu::TextureView> = (0..BLOOM_LEVELS - 1)
        .map(|l| tex("bloom up", down[l].1, down[l].2, hdr_format))
        .collect();
    let group = |a: &wgpu::TextureView, b: &wgpu::TextureView, u: &wgpu::Buffer| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(a),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(b),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: u.as_entire_binding(),
                },
            ],
        })
    };
    let mut post_groups = vec![group(&hdr, &hdr, &uniforms[0])];
    for l in 1..BLOOM_LEVELS {
        post_groups.push(group(&down[l - 1].0, &down[l - 1].0, &uniforms[l]));
    }
    for l in 0..BLOOM_LEVELS - 1 {
        let low = if l + 1 == BLOOM_LEVELS - 1 {
            &down[l + 1].0
        } else {
            &up[l + 1]
        };
        post_groups.push(group(low, &down[l].0, &uniforms[BLOOM_LEVELS + l]));
    }
    post_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("composite"),
        layout: composite_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&hdr),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&up[0]),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: uniforms[2 * BLOOM_LEVELS - 1].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&depth),
            },
        ],
    }));
    post_groups.push(group(&ldr, &ldr, &uniforms[2 * BLOOM_LEVELS]));
    Targets {
        iw,
        ih,
        hdr,
        depth,
        ldr,
        down,
        up,
        post_groups,
    }
}
