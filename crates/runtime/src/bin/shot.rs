//! Offscreen screenshots and a frame-time bench, rendered natively through
//! the same renderer the browser uses (any Vulkan/GL adapter, including a
//! software one such as lavapipe).
//!
//!   cargo run --release -p runtime --bin shot -- <out_dir> [width height]
//!   cargo run --release -p runtime --bin shot -- --bench [width height]
//!
//! Env: WGPU_BACKEND=gl (the WebGL2 code path), RENDER_SCALE=0.5.

use std::future::Future;
use std::path::Path;
use std::time::Instant;

use game::ai::Rival;
use game::tracks::catalog;
use render::{Renderer, Settings};
use runtime::{CamMode, Input, Session};

fn block_on<T>(fut: impl Future<Output = T>) -> T {
    let mut fut = std::pin::pin!(fut);
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    loop {
        if let std::task::Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Gpu {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        // WGPU_BACKEND=gl exercises the GLES path the WebGL2 bundle uses.
        backends: wgpu::Backends::from_env().unwrap_or(
            wgpu::Backends::VULKAN
                | wgpu::Backends::GL
                | wgpu::Backends::METAL
                | wgpu::Backends::DX12,
        ),
        ..Default::default()
    });
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .expect("no GPU adapter");
    eprintln!("adapter: {:?}", adapter.get_info());
    // Stay inside WebGL2 limits so what works here works in the browser.
    let limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("shot"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .expect("device");
    Gpu {
        adapter,
        device,
        queue,
    }
}

struct Offscreen {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    w: u32,
    h: u32,
    row: u32,
}

impl Offscreen {
    fn new(device: &wgpu::Device, w: u32, h: u32) -> Self {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let row = (w * 4).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            tex,
            view,
            readback,
            w,
            h,
            row,
        }
    }

    fn save(&self, gpu: &Gpu, path: &Path) {
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            self.tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.row),
                    rows_per_image: Some(self.h),
                },
            },
            wgpu::Extent3d {
                width: self.w,
                height: self.h,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(enc.finish()));
        let slice = self.readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        gpu.device.poll(wgpu::PollType::Wait).expect("poll");
        let data = slice.get_mapped_range();
        let mut px = Vec::with_capacity((self.w * self.h * 4) as usize);
        for y in 0..self.h {
            let start = (y * self.row) as usize;
            px.extend_from_slice(&data[start..start + (self.w * 4) as usize]);
        }
        drop(data);
        self.readback.unmap();
        image::save_buffer(path, &px, self.w, self.h, image::ColorType::Rgba8).expect("png");
        eprintln!("wrote {}", path.display());
    }
}

fn render(r: &mut Renderer, s: &Session, view: &wgpu::TextureView) {
    render_impact(r, s, view, 0.0);
}

/// Low close-up from the player's side, to check how the bike sits on the road.
/// Three-quarter view from ahead of the player, low, for the bike close-up.
fn render_front(r: &mut Renderer, s: &Session, view: &wgpu::TextureView) {
    let bikes = s.bike_draws();
    let particles = s.particles();
    let cars = s.car_draws();
    let mut input = s.frame_input(&bikes, &cars, &particles);
    let b = &s.race.bikes[0];
    let f = glam::Vec3::new(b.heading.cos(), 0.0, b.heading.sin());
    let side = glam::Vec3::new(-f.z, 0.0, f.x);
    let at = b.pos + glam::Vec3::Y * 0.7;
    input.camera.pos = at + f * 3.0 + side * 1.9 + glam::Vec3::Y * 0.35;
    input.camera.target = at;
    input.camera.up = glam::Vec3::Y;
    input.camera.fov_y = 0.7;
    r.render(view, &input);
}

fn render_side(r: &mut Renderer, s: &Session, view: &wgpu::TextureView) {
    render_side_posed(r, s, view, 0.0, 0.0);
}

/// Side view with the player's rider forced into a kick (-1 left .. 1
/// right) and/or guard (0..1), to show the combat poses.
fn render_side_posed(
    r: &mut Renderer,
    s: &Session,
    view: &wgpu::TextureView,
    kick: f32,
    block: f32,
) {
    let mut bikes = s.bike_draws();
    if kick != 0.0 || block != 0.0 {
        bikes[0].kick = kick;
        bikes[0].block = block;
        bikes[0].tuck = 0.0;
    }
    let particles = s.particles();
    let cars = s.car_draws();
    let mut input = s.frame_input(&bikes, &cars, &particles);
    let b = &s.race.bikes[0];
    let f = glam::Vec3::new(b.heading.cos(), 0.0, b.heading.sin());
    let side = glam::Vec3::new(-f.z, 0.0, f.x);
    let at = b.pos + glam::Vec3::Y * 0.6;
    input.camera.pos = at + side * 3.2 + f * 1.4 + glam::Vec3::Y * 0.15;
    input.camera.target = at;
    input.camera.up = glam::Vec3::Y;
    input.camera.fov_y = 0.75;
    r.render(view, &input);
}

/// Side view with the player holding a weapon (1 club, 2 chain), mid-swing.
fn render_weapon(r: &mut Renderer, s: &Session, view: &wgpu::TextureView, weapon: f32) {
    let mut bikes = s.bike_draws();
    bikes[0].weapon = weapon;
    bikes[0].weapon_hand = 1.0;
    bikes[0].punch = 0.55;
    bikes[0].tuck = 0.0;
    let particles = s.particles();
    let cars = s.car_draws();
    let mut input = s.frame_input(&bikes, &cars, &particles);
    let b = &s.race.bikes[0];
    let f = glam::Vec3::new(b.heading.cos(), 0.0, b.heading.sin());
    let side = glam::Vec3::new(-f.z, 0.0, f.x);
    let at = b.pos + glam::Vec3::Y * 0.9;
    input.camera.pos = at + side * 2.6 + f * 2.2 + glam::Vec3::Y * 0.5;
    input.camera.target = at;
    input.camera.up = glam::Vec3::Y;
    input.camera.fov_y = 0.75;
    r.render(view, &input);
}

/// Behind and above the nearest car ahead of the player, looking at it.
fn render_traffic(r: &mut Renderer, s: &Session, view: &wgpu::TextureView) -> bool {
    let me = s.race.bikes[0].track_pos.s;
    let Some(car) = s
        .race
        .traffic
        .cars
        .iter()
        .filter(|c| (8.0..600.0).contains(&s.race.track.delta_s(me, c.s)))
        .min_by(|a, b| {
            s.race
                .track
                .delta_s(me, a.s)
                .total_cmp(&s.race.track.delta_s(me, b.s))
        })
    else {
        return false;
    };
    let bikes = s.bike_draws();
    let particles = s.particles();
    let cars = s.car_draws_near(car.pos);
    let mut input = s.frame_input(&bikes, &cars, &particles);
    let f = s.race.track.frame_at(car.s);
    let fwd = glam::Vec3::new(f.fwd.x, 0.0, f.fwd.y);
    let right = glam::Vec3::new(f.right.x, 0.0, f.right.y);
    input.camera.pos = car.pos - fwd * 9.0 + right * 2.5 + glam::Vec3::Y * 2.6;
    input.camera.target = car.pos + glam::Vec3::Y * 0.8 + fwd * 4.0;
    input.camera.up = glam::Vec3::Y;
    input.camera.fov_y = 0.9;
    r.render(view, &input);
    true
}

/// Looks along the wide straight at its first kicker, from behind and above.
fn render_ramp(r: &mut Renderer, s: &Session, view: &wgpu::TextureView) -> bool {
    let track = &s.race.track;
    let Some(ramp) = track.ramps.first() else {
        return false;
    };
    let bikes = s.bike_draws();
    let particles = s.particles();
    let cars = s.car_draws();
    let mut input = s.frame_input(&bikes, &cars, &particles);
    let f = track.frame_at(ramp.s);
    let fwd = glam::Vec3::new(f.fwd.x, 0.0, f.fwd.y);
    let right = glam::Vec3::new(f.right.x, 0.0, f.right.y);
    let foot = f.pos + right * ramp.lateral;
    input.camera.pos =
        foot - fwd * 16.0 - right * ramp.lateral.signum() * 6.0 + glam::Vec3::Y * 4.5;
    input.camera.target = foot + fwd * 8.0 + glam::Vec3::Y * 0.6;
    input.camera.up = glam::Vec3::Y;
    input.camera.fov_y = 0.9;
    r.render(view, &input);
    true
}

/// Renders with an impact frame forced on (1) or left to the session (0).
fn render_impact(r: &mut Renderer, s: &Session, view: &wgpu::TextureView, impact: f32) {
    let bikes = s.bike_draws();
    let particles = s.particles();
    let cars = s.car_draws();
    let mut input = s.frame_input(&bikes, &cars, &particles);
    input.impact = input.impact.max(impact);
    r.render(view, &input);
}

fn advance(s: &mut Session, secs: f32) {
    let input = Input::default();
    let steps = (secs * 60.0) as usize;
    for _ in 0..steps {
        s.update(1.0 / 60.0, &input);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bench = args.first().map(|a| a == "--bench").unwrap_or(false);
    let rest: Vec<&String> = args.iter().skip(1).collect();
    let w: u32 = rest.first().and_then(|v| v.parse().ok()).unwrap_or(1280);
    let h: u32 = rest.get(1).and_then(|v| v.parse().ok()).unwrap_or(720);
    let gpu = gpu();
    let off = Offscreen::new(&gpu.device, w, h);
    let mut r = Renderer::new(
        &gpu.adapter,
        &gpu.device,
        &gpu.queue,
        wgpu::TextureFormat::Rgba8Unorm,
        w,
        h,
        Settings::default(),
    );
    eprintln!("hdr format: {:?}", r.hdr_format());
    // RENDER_SCALE=0.5 benches what the browser's dynamic resolution falls back to.
    if let Some(scale) = std::env::var("RENDER_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        r.set_render_scale(scale);
    }

    if bench {
        for def in catalog() {
            let t0 = Instant::now();
            let mut s = Session::new(*def, 11);
            r.load_world(&s.race.track);
            let load = t0.elapsed();
            s.autopilot = Some(Rival::new(0.0, 0.9, 0.3));
            advance(&mut s, 8.0);
            // Warm up, then time frames end to end (CPU + GPU, synchronous).
            for _ in 0..3 {
                render(&mut r, &s, &off.view);
                gpu.device.poll(wgpu::PollType::Wait).ok();
            }
            let frames = 30;
            let mut sim = std::time::Duration::ZERO;
            let t = Instant::now();
            for _ in 0..frames {
                let ts = Instant::now();
                s.update(1.0 / 60.0, &Input::default());
                sim += ts.elapsed();
                render(&mut r, &s, &off.view);
                gpu.device.poll(wgpu::PollType::Wait).ok();
            }
            let per = t.elapsed().as_secs_f64() * 1000.0 / frames as f64;
            println!(
                "{:<16} load {:>6.0} ms | frame {:>7.2} ms ({:>5.1} fps) | sim+fx {:>5.3} ms | {} draws, {} k tris ({} k props, {} k shadow), {} props",
                def.name,
                load.as_secs_f64() * 1000.0,
                per,
                1000.0 / per,
                sim.as_secs_f64() * 1000.0 / frames as f64,
                r.stats.draws,
                r.stats.triangles / 1000,
                r.stats.prop_triangles / 1000,
                r.stats.shadow_triangles / 1000,
                r.stats.props
            );
        }
        return;
    }

    let out = Path::new(args.first().map(|s| s.as_str()).unwrap_or("shots"));
    std::fs::create_dir_all(out).expect("out dir");
    for def in catalog() {
        let mut s = Session::new(*def, 5);
        r.load_world(&s.race.track);
        s.autopilot = Some(Rival::new(0.1, 0.92, 0.6));
        advance(&mut s, 2.2);
        render(&mut r, &s, &off.view);
        off.save(&gpu, &out.join(format!("{}-1-grid.png", def.slug)));
        advance(&mut s, 14.0);
        render(&mut r, &s, &off.view);
        off.save(&gpu, &out.join(format!("{}-2-chase.png", def.slug)));
        render_side(&mut r, &s, &off.view);
        off.save(&gpu, &out.join(format!("{}-5-side.png", def.slug)));
        render_front(&mut r, &s, &off.view);
        off.save(&gpu, &out.join(format!("{}-6-front.png", def.slug)));
        // The camera sits on the bike's right, so kick to the right.
        render_side_posed(&mut r, &s, &off.view, 1.0, 0.0);
        off.save(&gpu, &out.join(format!("{}-7-kick.png", def.slug)));
        render_side_posed(&mut r, &s, &off.view, 0.0, 1.0);
        off.save(&gpu, &out.join(format!("{}-8-guard.png", def.slug)));
        render_impact(&mut r, &s, &off.view, 1.0);
        off.save(&gpu, &out.join(format!("{}-4-impact.png", def.slug)));
        render_weapon(&mut r, &s, &off.view, 1.0);
        off.save(&gpu, &out.join(format!("{}-9-club.png", def.slug)));
        render_weapon(&mut r, &s, &off.view, 2.0);
        off.save(&gpu, &out.join(format!("{}-10-chain.png", def.slug)));
        if render_ramp(&mut r, &s, &off.view) {
            off.save(&gpu, &out.join(format!("{}-12-ramp.png", def.slug)));
        }
        if render_traffic(&mut r, &s, &off.view) {
            off.save(&gpu, &out.join(format!("{}-11-traffic.png", def.slug)));
        }
        advance(&mut s, 9.0);
        s.cam_mode = CamMode::Far;
        advance(&mut s, 0.5);
        render(&mut r, &s, &off.view);
        off.save(&gpu, &out.join(format!("{}-3-far.png", def.slug)));
    }
}
