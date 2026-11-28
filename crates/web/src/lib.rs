//! Browser shell. `web/loader.js` boots one of two wasm bundles (WebGPU or
//! WebGL2, chosen by cargo feature), creates a [`Game`] on a canvas and calls
//! [`Game::frame`] from `requestAnimationFrame`. Everything visual is drawn by
//! the `render` crate; the page only paints HTML HUD and menus on top, fed
//! by the flat arrays this module hands back.

#![forbid(unsafe_code)]

use wasm_bindgen::prelude::*;

/// Which GPU backend this bundle was built for.
#[wasm_bindgen]
pub fn backend_name() -> String {
    if cfg!(feature = "webgl2") {
        "webgl2"
    } else {
        "webgpu"
    }
    .to_owned()
}

/// The track list as `slug|name|laps|blurb` lines, for the track picker.
#[wasm_bindgen]
pub fn track_list() -> String {
    game::tracks::catalog()
        .iter()
        .map(|d| format!("{}|{}|{}|{}", d.slug, d.name, d.laps, d.blurb))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The design system's CSS custom properties for a track (`--ink`, `--hot`,
/// `--accent` and the rest), so the HTML HUD wears the same palette as the
/// scene. Unknown slugs get the first track's.
#[wasm_bindgen]
pub fn style_css(slug: &str) -> String {
    let def = game::tracks::by_slug(slug).unwrap_or(game::tracks::catalog()[0]);
    render::style::css_vars(&def.theme)
}

/// Number of floats in [`Game::hud`].
pub const HUD_LEN: usize = 28;

/// Packs the HUD snapshot into a flat array (layout mirrored in loader.js).
pub fn pack_hud(h: &runtime::Hud, paused: bool) -> [f32; HUD_LEN] {
    let opt = |v: Option<f32>| v.unwrap_or(-1.0);
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    [
        h.position as f32,
        h.racers as f32,
        h.lap as f32,
        h.laps as f32,
        h.lap_time,
        opt(h.last_lap),
        opt(h.best_lap),
        h.race_time,
        h.speed_kmh,
        h.gear as f32,
        h.rpm,
        h.boost,
        b(h.boosting),
        h.countdown,
        b(h.wrong_way),
        b(h.crashed),
        opt(h.finished),
        h.balance,
        h.progress,
        b(h.drafting),
        b(paused),
        h.guard,
        b(h.strike_ready),
        weapon_code(h.weapon),
        h.weapon_uses as f32,
        h.knockoffs as f32,
        h.near_misses as f32,
        h.hits_landed as f32,
    ]
}

/// 0 bare-handed, 1 club, 2 chain.
fn weapon_code(w: Option<game::race::WeaponKind>) -> f32 {
    use game::race::WeaponKind;
    match w {
        None => 0.0,
        Some(WeaponKind::Club) => 1.0,
        Some(WeaponKind::Chain) => 2.0,
    }
}

/// Flattens cues into (code, a, b) triples (codes mirrored in loader.js).
pub fn pack_cues(cues: &[runtime::Cue], out: &mut Vec<f32>) {
    use runtime::Cue;
    out.clear();
    for c in cues {
        let (code, a, b) = match *c {
            Cue::Beep(n) => (1.0, n as f32, 0.0),
            Cue::Go => (2.0, 0.0, 0.0),
            Cue::WallHit(s) => (3.0, s, 0.0),
            Cue::Contact(s) => (4.0, s, 0.0),
            Cue::HitLanded(kick) => (5.0, if kick { 1.0 } else { 0.0 }, 0.0),
            Cue::GotHit => (6.0, 0.0, 0.0),
            Cue::Crash => (7.0, 0.0, 0.0),
            Cue::Lap(n, t) => (8.0, n as f32, t),
            Cue::Finished(p) => (9.0, p as f32, 0.0),
            Cue::Overtake(gained) => (10.0, if gained { 1.0 } else { 0.0 }, 0.0),
            Cue::Guarded => (11.0, 0.0, 0.0),
            Cue::Parried => (12.0, 0.0, 0.0),
            Cue::WasParried => (13.0, 0.0, 0.0),
            Cue::KnockOff => (14.0, 0.0, 0.0),
            Cue::Launch(perfect) => (15.0, if perfect { 1.0 } else { 0.0 }, 0.0),
            Cue::TrafficCrash => (16.0, 0.0, 0.0),
            Cue::NearMiss(oncoming) => (17.0, if oncoming { 1.0 } else { 0.0 }, 0.0),
            Cue::Armed(kind) => (18.0, weapon_code(Some(kind)), 0.0),
            Cue::Disarmed => (19.0, 0.0, 0.0),
            Cue::Hunted(i) => (20.0, i as f32, 0.0),
        };
        out.extend_from_slice(&[code, a, b]);
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use game::ai::Rival;
    use game::difficulty::Difficulty;
    use game::race::Loadout;
    use game::tracks::{by_slug, catalog};
    use render::{Renderer, Settings};
    use runtime::{CamMode, Input, Session};

    #[wasm_bindgen(start)]
    fn start() {
        console_error_panic_hook::set_once();
    }

    #[wasm_bindgen]
    pub struct Game {
        surface: wgpu::Surface<'static>,
        device: wgpu::Device,
        config: wgpu::SurfaceConfiguration,
        renderer: Renderer,
        session: Session,
        slug: String,
        seed: u64,
        difficulty: Difficulty,
        loadout: Loadout,
        cues: Vec<f32>,
        adapter_name: String,
    }

    fn err(e: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&e.to_string())
    }

    #[wasm_bindgen]
    impl Game {
        /// Creates the GPU context on `canvas` (sized by the caller) and
        /// loads `slug` in attract mode.
        pub async fn boot(
            canvas: web_sys::HtmlCanvasElement,
            slug: String,
        ) -> Result<Game, JsValue> {
            let backends = if cfg!(feature = "webgl2") {
                wgpu::Backends::GL
            } else {
                wgpu::Backends::BROWSER_WEBGPU
            };
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends,
                ..Default::default()
            });
            let (w, h) = (canvas.width().max(1), canvas.height().max(1));
            let surface = instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
                .map_err(err)?;
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: false,
                    compatible_surface: Some(&surface),
                })
                .await
                .map_err(err)?;
            let limits =
                wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("racing"),
                    required_features: wgpu::Features::empty(),
                    required_limits: limits,
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                })
                .await
                .map_err(err)?;
            let caps = surface.get_capabilities(&adapter);
            // The post chain writes display-encoded colour itself, so prefer a
            // plain UNORM surface; the renderer adapts if only sRGB exists.
            let format = caps
                .formats
                .iter()
                .copied()
                .find(|f| !f.is_srgb())
                .or_else(|| caps.formats.first().copied())
                .ok_or_else(|| err("surface has no formats"))?;
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: w,
                height: h,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: caps
                    .alpha_modes
                    .first()
                    .copied()
                    .unwrap_or(wgpu::CompositeAlphaMode::Auto),
                view_formats: vec![],
            };
            surface.configure(&device, &config);
            let renderer =
                Renderer::new(&adapter, &device, &queue, format, w, h, Settings::default());
            let def = by_slug(&slug).unwrap_or(catalog()[0]);
            let mut game = Game {
                surface,
                device,
                config,
                renderer,
                session: Session::new(def, 1),
                slug: def.slug.to_owned(),
                seed: 1,
                difficulty: Difficulty::Normal,
                loadout: Loadout::default(),
                cues: Vec::new(),
                adapter_name: adapter.get_info().name,
            };
            game.renderer.load_world(&game.session.race.track);
            game.attract();
            Ok(game)
        }

        pub fn backend(&self) -> String {
            backend_name()
        }

        pub fn adapter(&self) -> String {
            self.adapter_name.clone()
        }

        pub fn hdr_format(&self) -> String {
            format!("{:?}", self.renderer.hdr_format())
        }

        /// Current track's slug.
        pub fn track(&self) -> String {
            self.slug.clone()
        }

        fn new_session(&mut self, slug: &str) {
            let def = by_slug(slug).unwrap_or(catalog()[0]);
            self.seed = self
                .seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let cam = self.session.cam_mode;
            self.session =
                Session::with_loadout(def, self.seed >> 16, self.difficulty, self.loadout);
            self.session.cam_mode = cam;
            if def.slug != self.slug {
                self.slug = def.slug.to_owned();
                self.renderer.load_world(&self.session.race.track);
            }
        }

        /// Background demo: the player bike drives itself.
        pub fn attract(&mut self) {
            let slug = self.slug.clone();
            self.show(&slug);
        }

        /// Loads `slug` (if different) as an attract-mode demo.
        pub fn show(&mut self, slug: &str) {
            self.new_session(slug);
            self.session.autopilot = Some(Rival::new(0.15, 0.95, 0.4));
            self.session.cam_mode = CamMode::Far;
        }

        /// Starts a race the player drives, on `slug`, at `difficulty`
        /// (`easy`, `normal` or `hard`; anything else is normal), on a bike
        /// with garage upgrades `engine`, `handling` and `armour` (0..=3).
        pub fn start_race(
            &mut self,
            slug: &str,
            difficulty: &str,
            engine: u32,
            handling: u32,
            armour: u32,
        ) {
            self.difficulty = Difficulty::from_slug(difficulty);
            self.loadout = Loadout::from_levels(engine, handling, armour);
            self.new_session(slug);
            self.session.autopilot = None;
            if self.session.cam_mode == CamMode::Far {
                self.session.cam_mode = CamMode::Chase;
            }
        }

        pub fn set_paused(&mut self, paused: bool) {
            self.session.paused = paused;
        }

        pub fn cycle_camera(&mut self) -> u32 {
            self.session.cycle_camera();
            self.session.cam_mode as u32
        }

        /// The canvas backing store changed size.
        pub fn resize(&mut self, w: u32, h: u32) {
            let (w, h) = (w.max(1), h.max(1));
            if w == self.config.width && h == self.config.height {
                return;
            }
            self.config.width = w;
            self.config.height = h;
            self.surface.configure(&self.device, &self.config);
            self.renderer.resize(w, h);
        }

        /// Ink outlines and impact frames on or off (impact frames are off
        /// for players who ask for reduced motion).
        pub fn set_effects(&mut self, ink: bool, flashes: bool) {
            self.renderer.settings.ink = ink;
            self.renderer.settings.flashes = flashes;
        }

        /// Internal render resolution as a fraction of the canvas (0.35..1).
        pub fn set_render_scale(&mut self, s: f32) {
            self.renderer.set_render_scale(s);
        }

        /// Advances `dt` seconds of play and draws. Returns the cues raised,
        /// as (code, a, b) triples.
        #[allow(clippy::too_many_arguments)]
        pub fn frame(
            &mut self,
            dt: f32,
            throttle: f32,
            brake: f32,
            steer: f32,
            analog_steer: bool,
            boost: bool,
            attack: bool,
            kick: bool,
            block: bool,
        ) -> Vec<f32> {
            let input = Input {
                throttle,
                brake,
                steer,
                analog_steer,
                boost,
                attack,
                kick,
                block,
            };
            self.session.update(dt, &input);
            pack_cues(&self.session.cues, &mut self.cues);
            self.draw();
            self.cues.clone()
        }

        fn draw(&mut self) {
            let frame = match self.surface.get_current_texture() {
                Ok(f) => f,
                Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                    self.surface.configure(&self.device, &self.config);
                    return;
                }
                Err(_) => return,
            };
            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let bikes = self.session.bike_draws();
            let particles = self.session.particles();
            let cars = self.session.car_draws();
            let input = self.session.frame_input(&bikes, &cars, &particles);
            self.renderer.render(&view, &input);
            frame.present();
        }

        /// HUD snapshot; layout in `pack_hud`.
        pub fn hud(&self) -> Vec<f32> {
            pack_hud(&self.session.hud(), self.session.paused).to_vec()
        }

        /// Centreline as x,z pairs for the minimap.
        pub fn track_outline(&self) -> Vec<f32> {
            let t = &self.session.race.track;
            (0..t.len())
                .step_by(3)
                .flat_map(|i| {
                    let p = t.sample(i).pos;
                    [p.x, p.z]
                })
                .collect()
        }

        /// Every bike as x,z pairs, player first.
        pub fn bike_points(&self) -> Vec<f32> {
            self.session
                .bike_positions()
                .iter()
                .flat_map(|p| [p.x, p.y])
                .collect()
        }

        /// Standings as `name|time|finished` lines, leader first.
        pub fn standings(&self) -> String {
            self.session
                .standings_names()
                .into_iter()
                .map(|(n, t, done)| format!("{n}|{t}|{}", u8::from(done)))
                .collect::<Vec<_>>()
                .join("\n")
        }

        /// A racer's name by bike index (cues carry indices).
        pub fn racer_name(&self, i: u32) -> String {
            runtime::racer_name(i as usize).to_owned()
        }

        /// Draw calls, triangles and visible props in the last frame.
        pub fn stats(&self) -> Vec<f32> {
            let s = self.renderer.stats;
            vec![s.draws as f32, s.triangles as f32, s.props as f32]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game::ai::Rival;
    use game::tracks::catalog;
    use runtime::{Input, Session};

    #[test]
    fn track_list_has_every_track() {
        let list = track_list();
        assert_eq!(list.lines().count(), catalog().len());
        for line in list.lines() {
            assert_eq!(line.split('|').count(), 4, "{line}");
        }
    }

    #[test]
    fn style_css_follows_the_track() {
        let a = style_css(catalog()[0].slug);
        let b = style_css(catalog()[1].slug);
        assert!(
            a.contains("--accent:") && a.contains("--hot: #ff4d2e;"),
            "{a}"
        );
        assert_ne!(a, b, "tracks share the palette but not the accent");
        assert_eq!(style_css("nope"), a);
    }

    #[test]
    fn hud_and_cues_pack_through_a_race_start() {
        let mut s = Session::new(catalog()[0], 7);
        s.autopilot = Some(Rival::new(0.0, 0.9, 0.0));
        let mut out = Vec::new();
        let mut codes = Vec::new();
        for _ in 0..(60 * 8) {
            s.update(1.0 / 60.0, &Input::default());
            pack_cues(&s.cues, &mut out);
            assert_eq!(out.len() % 3, 0);
            codes.extend(out.chunks(3).map(|c| c[0] as u32));
        }
        assert!(codes.contains(&1), "countdown beeps");
        assert!(codes.contains(&2), "green light");
        let hud = pack_hud(&s.hud(), false);
        assert_eq!(hud.len(), HUD_LEN);
        assert!(hud[8] > 50.0, "moving: {} km/h", hud[8]);
        assert_eq!(hud[1] as usize, s.race.bikes.len());
    }
}
