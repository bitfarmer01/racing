//! WASM browser shell: the layer between the page and the composed runtime.
//!
//! One wasm artifact ships per GPU backend (cargo features `webgpu` /
//! `webgl2`, matching [`render::QualityTier::backend`]); the JS loader in
//! `web/loader.js` feature-detects the browser and boots exactly one of them.
//! Inside a build nothing branches on backend capability at runtime beyond
//! [`render::QualityTier::select`] — the loader's adapter probe already made
//! that decision.
//!
//! The shell reuses `runtime`'s [`advance_game_frame`] composition for the
//! frame loop instead of re-wiring camera, road mesh, and sprite batching
//! here: per frame the JS loader calls [`BrowserGame::advance_frame`] with
//! input flags, the race advances one 60 Hz tick, the pre-owned batch and
//! road mesh refill, and [`render::GpuRenderer::draw`] presents.
//!
//! Boundaries:
//! - The canvas backing store is sized by the loader (window size ×
//!   devicePixelRatio, capped) before construction and refreshed on window
//!   resize. The renderer was built for the boot size and reconfigures its
//!   surface implicitly when the canvas element's size changes — wgpu 25
//!   checks the canvas size at `get_current_texture` — so no explicit
//!   resize path lives here.
//! - The camera's clip transform is a fixed ortho over the runtime's
//!   800×450 pixel-space viewport (per-frame rider positions already live
//!   in screen space), so one camera serves every frame at any surface
//!   size; a non-16:9 window stretches the composition non-uniformly. That
//!   fixed viewport is load-bearing in `runtime`'s frame builder, so it is
//!   documented rather than changed.
//! - No browser here on the dev host: this crate is compile-checked for
//!   wasm32 (`cargo check -p web --target wasm32-unknown-unknown`, both
//!   feature builds) and exercised in a browser only by the recorded
//!   follow-up smoke test.
//!
//! No GPU here on the dev host: this crate is compile-checked for wasm32
//! (`cargo check -p web --target wasm32-unknown-unknown`, both feature
//! builds) and exercised in a browser only by the recorded follow-up smoke
//! test. What IS testable headless is the boot path's course resolution —
//! `BrowserGame::boot` resolves the track-select value through
//! `resolve_course` and builds the race through `race_for_course`, and the
//! wave-8 gate asks for headless boot evidence that each selection loads its
//! own course (see the `tests` module at the bottom).

#![forbid(unsafe_code)]

// The browser shell only exists on wasm: wgpu's `SurfaceTarget::Canvas` is
// gated to wasm targets (wgpu's build script aliases `webgpu`/`webgl` to
// `not(native)`), so every item that touches a GPU type is compiled only for
// `wasm32-unknown-unknown` — the target the two workspace wasm checks build.
// Host builds see the crate as the panic hook plus `backend_name`.
use wasm_bindgen::prelude::*;
// The race-construction path exists only on wasm and in the headless boot
// tests; host non-test builds see neither, so the imports carry its gate.
#[cfg(any(target_arch = "wasm32", test))]
use game::race::{Race, RaceConfig};
#[cfg(target_arch = "wasm32")]
use render::{AtlasDims, AtlasUpload, GpuRenderer, OrthoCamera, QualityTier, SpriteBatch};
#[cfg(target_arch = "wasm32")]
use runtime::{advance_game_frame, pack_placeholder_atlas, AtlasMapping, RoadMesh};
#[cfg(any(target_arch = "wasm32", test))]
use world::{Course, CourseId, RoadGeometry};

#[cfg(target_arch = "wasm32")]
use game::bike::Control;
#[cfg(target_arch = "wasm32")]
use game::input::GameInput;

/// Installs the panic hook once: panics surface in the browser console
/// instead of vanishing behind a frozen loading screen.
#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// Which backend this build was compiled for, as the loader picked it.
#[wasm_bindgen]
pub fn backend_name() -> String {
    if cfg!(feature = "webgl2") {
        "webgl2".to_owned()
    } else {
        "webgpu".to_owned()
    }
}

/// Folds the loader's input flags into the events queued for one tick:
/// the last throttle-family command wins (brake over throttle), then the
/// last steering direction. Pure — no device or race access.
#[cfg(target_arch = "wasm32")]
fn frame_inputs(
    accelerate: bool,
    brake: bool,
    steer_left: bool,
    steer_right: bool,
    punch: bool,
    kick: bool,
) -> ([GameInput; 3], usize) {
    let throttle = if brake {
        Some(GameInput::Brake)
    } else if accelerate {
        Some(GameInput::Accelerate)
    } else {
        None
    };
    let steer = if steer_left {
        Some(GameInput::SteerLeft)
    } else if steer_right {
        Some(GameInput::SteerRight)
    } else {
        None
    };
    // One swing per press; punch outranks kick if both land together.
    let attack = if punch {
        Some(GameInput::Punch)
    } else if kick {
        Some(GameInput::Kick)
    } else {
        None
    };
    let mut inputs = [
        GameInput::Accelerate,
        GameInput::Accelerate,
        GameInput::Accelerate,
    ];
    let mut count = 0;
    for input in [throttle, steer, attack].into_iter().flatten() {
        inputs[count] = input;
        count += 1;
    }
    (inputs, count)
}

/// The booted game: one race, its frame-composition buffers, and the GPU
/// renderer presenting to the loader's canvas. Held across frames so the
/// steady-state loop allocates nothing.
///
/// wasm-only, and not by choice: wgpu's `SurfaceTarget::Canvas` exists only
/// on wasm targets (wgpu's build script aliases `webgpu`/`webgl` to
/// `not(native)`), so this type cannot compile on host targets. The wasm32
/// checks in the workspace gate compile the real code.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub struct BrowserGame {
    race: game::race::Race,
    /// The course this race was booted from: the restart path re-boots the
    /// same course, never falling back to the demo (spec: locked decision).
    course: Course,
    geometry: RoadGeometry,
    mapping: AtlasMapping,
    batch: SpriteBatch,
    road: RoadMesh,
    camera: OrthoCamera,
    renderer: GpuRenderer,
}

#[cfg(target_arch = "wasm32")]
pub(crate) enum BootError {
    Atlas(String),
    Render(String),
    Course(String),
}

#[cfg(target_arch = "wasm32")]
impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Atlas(e) => write!(f, "placeholder atlas pack failed: {e}"),
            Self::Render(e) => write!(f, "renderer bring-up failed: {e}"),
            Self::Course(e) => write!(f, "track selection failed: {e}"),
        }
    }
}

/// Builds the race for a catalog course: the same shape `runtime::demo_race`
/// gives the demo — three rivals, three laps, the fixed seed — but with the
/// selected course's own lap length and geometry (spec: "Track-select flow").
/// Lives here rather than in `runtime`: `runtime::frame` is another
/// workstream's lane, so its five `ZoneLayout::for_track()` call sites (the
/// composed scenery still samples the demo table) move onto the selected
/// course's layout when that lane migrates.
#[cfg(any(target_arch = "wasm32", test))]
fn race_for_course(course: &Course) -> (Race, RoadGeometry) {
    let geometry = RoadGeometry::new(
        course.curves.to_vec(),
        course.elevation.to_vec(),
        course.road_width,
    );
    let race = Race::new(RaceConfig {
        rivals: 3,
        // One lap = one pass through every zone; landmarks and dressing
        // repeat per lap on the same fixed distances (7c).
        track_length: course.layout.lap_length,
        total_laps: 3,
        seed: 0x5EED_C0DE,
        geometry: geometry.clone(),
    });
    (race, geometry)
}

/// Resolves the track-select control's value to a catalog course. An unknown
/// value is a boot error on the loading screen, same path as today's boot
/// failures (spec: "Track-select flow").
#[cfg(any(target_arch = "wasm32", test))]
fn resolve_course(course_id: &str) -> Result<&'static Course, String> {
    let id = CourseId::from_slug(course_id).ok_or_else(|| {
        format!("unknown track '{course_id}' — expected one of the catalog slugs")
    })?;
    world::for_course(id).ok_or_else(|| format!("course '{course_id}' is not registered"))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl BrowserGame {
    /// Boots the selected course on `canvas`: packs the placeholder atlas,
    /// selects the quality tier from the loader's capability facts, and
    /// brings up the GPU pipeline (warm behind the loading screen per the
    /// blueprint).
    ///
    /// `low_end_device` comes from the loader's device heuristics;
    /// `has_webgpu` is compile-time — this build either carries the WebGPU
    /// backend or it does not, and the loader already chose the build to
    /// match its adapter probe. `course_id` is the track-select control's
    /// value, resolved through `resolve_course`; an unknown value takes the
    /// loading screen's error path.
    ///
    /// The canvas is sized before this call; its current `width`/`height`
    /// become the swap-chain size.
    ///
    /// Async factory rather than an async constructor: wasm-bindgen has
    /// deprecated async constructors (invalid TS bindings).
    #[wasm_bindgen]
    pub async fn boot(
        canvas: web_sys::HtmlCanvasElement,
        low_end_device: bool,
        course_id: &str,
    ) -> Result<BrowserGame, JsError> {
        Self::bring_up(canvas, low_end_device, course_id)
            .await
            .map_err(|e| JsError::new(&e.to_string()))
    }

    async fn bring_up(
        canvas: web_sys::HtmlCanvasElement,
        low_end_device: bool,
        course_id: &str,
    ) -> Result<Self, BootError> {
        // The course resolves before any GPU work: an unknown track-select
        // value fails fast on the loading screen.
        let course = resolve_course(course_id).map_err(BootError::Course)?;
        let has_webgpu = cfg!(feature = "webgpu");
        let tier = QualityTier::select(has_webgpu, low_end_device);

        let packed = pack_placeholder_atlas().map_err(|e| BootError::Atlas(e.to_string()))?;
        let mapping = AtlasMapping::from_packed(&packed);
        let (width, height) = (packed.atlas.width(), packed.atlas.height());
        let rgba = packed.atlas.into_raw();
        let atlas = AtlasUpload::from_rgba(AtlasDims::new(width, height), rgba)
            .map_err(|e| BootError::Atlas(e.to_string()))?;

        let (race, geometry) = race_for_course(course);

        // One backend per bundle — the loader's probe already picked it.
        // This matters even though wgpu only compiles the matching backend:
        // with default features the render/runtime dependency graph unifies
        // `wgpu/webgpu` back in, and an `Backends::all()` instance then binds
        // the canvas for BOTH backends. In headless Chrome the WebGPU path's
        // `canvas.getContext('webgpu')` races the WebGL2 binding and wins,
        // leaving the GL surface without a context (`NoAdapter` at bring-up,
        // or a renderer presenting into a void). Requesting exactly one
        // backend keeps the canvas owned by the build's renderer.
        let backends = if cfg!(feature = "webgpu") {
            wgpu::Backends::BROWSER_WEBGPU
        } else {
            wgpu::Backends::GL
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::default()
        });
        let renderer = GpuRenderer::new(
            &instance,
            wgpu::SurfaceTarget::Canvas(canvas.clone()),
            (canvas.width(), canvas.height()),
            &atlas,
            tier,
        )
        .await
        .map_err(|e| BootError::Render(e.to_string()))?;

        Ok(Self {
            race,
            course: *course,
            geometry,
            mapping,
            batch: SpriteBatch::for_tier(tier),
            road: RoadMesh::default(),
            // The camera's clip transform is a fixed ortho over the runtime's
            // pixel-space viewport; per-frame rider positions already live in
            // screen space, so one camera serves every frame.
            camera: OrthoCamera::new(runtime::VIEWPORT_WIDTH, runtime::VIEWPORT_HEIGHT),
            renderer,
        })
    }

    /// Advances the race one 60 Hz tick under the loader's input flags and
    /// draws the composed frame. Steady state: no allocation after warmup —
    /// the batch and road mesh refill through `advance_game_frame`.
    pub fn advance_frame(
        &mut self,
        accelerate: bool,
        brake: bool,
        steer_left: bool,
        steer_right: bool,
        punch: bool,
        kick: bool,
    ) -> Result<(), JsError> {
        let (inputs, count) = frame_inputs(accelerate, brake, steer_left, steer_right, punch, kick);
        advance_game_frame(
            &mut self.race,
            &self.geometry,
            &self.mapping,
            &inputs[..count],
            &mut self.batch,
            &mut self.road,
        );
        self.renderer
            .draw(&self.batch, &self.road.vertices, &self.camera)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Current race lap index, for the loader's HUD.
    pub fn lap_index(&self) -> u32 {
        self.race.lap_index()
    }

    /// Current player distance along the track, in meters, for the HUD.
    pub fn player_distance(&self) -> f32 {
        self.race.player_distance()
    }

    /// Player speed in km/h, for the HUD.
    pub fn speed_kmh(&self) -> u32 {
        (self.race.player_speed() * 3.6) as u32
    }

    /// The player's race position, 1-based, for the HUD.
    pub fn race_position(&self) -> usize {
        self.race.race_position()
    }

    /// How many bikes are racing, for the HUD's `pos / total` display.
    pub fn bike_count(&self) -> usize {
        self.race.bike_count()
    }

    /// Seconds left on the green-light countdown, for the HUD.
    pub fn countdown_secs(&self) -> u32 {
        self.race.countdown_secs()
    }

    /// Whether the lights are still red.
    pub fn is_counting_down(&self) -> bool {
        self.race.is_counting_down()
    }

    /// Progress through the player's current lap, in `[0, 1)`, for the HUD's
    /// progress bar.
    pub fn lap_progress(&self) -> f32 {
        self.race.lap_progress()
    }

    /// Laps needed to finish, for the HUD's `lap x of N` display.
    pub fn total_laps(&self) -> u32 {
        self.race.total_laps()
    }

    /// Length of one lap in meters, for the HUD's progress display.
    pub fn lap_length(&self) -> f32 {
        self.race.lap_length()
    }

    /// The rider's state as a bitmask for the HUD badge: 1 = crashed,
    /// 2 = off-road, 4 = braking. A stable u32 rather than a string keeps
    /// the per-frame HUD read allocation-free; the loader maps bits to
    /// labels and colors. Off-road is derived here the same way the bike
    /// state machine draws the line: lateral distance beyond the road's
    /// half width.
    pub fn rider_state(&self) -> u32 {
        let mut state = 0;
        if self.race.player_control() == Control::Crashed {
            state |= 1;
        }
        if self.race.player_lateral().abs() > self.geometry.road_width() * 0.5 {
            state |= 2;
        }
        if self.race.player_control() == Control::Brake {
            state |= 4;
        }
        state
    }

    /// The player's combat stamina bar, 0..1, for the HUD strip.
    pub fn player_stamina(&self) -> f32 {
        self.race.player_stamina()
    }

    /// Whether the player has finished every lap.
    pub fn is_finished(&self) -> bool {
        self.race.is_finished()
    }

    /// Back to the grid: a fresh race on the same course the player chose at
    /// boot. Inputs and camera state rebuild through the normal per-frame
    /// path.
    pub fn restart(&mut self) {
        let (race, _geometry) = race_for_course(&self.course);
        self.race = race;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wave-8 gate evidence (headless boot): the boot path resolves each
    /// track-select value and loads that course's own race. BrowserGame's
    /// GPU bring-up needs a canvas, but the course resolution and the race
    /// it builds are the same host-side calls, so each selection's course
    /// can be checked headless.
    #[test]
    fn booting_each_track_select_option_loads_that_course() {
        for (slug, lap_length) in [("coast-run", 2_000.0), ("island-hop", 1_600.0)] {
            let course = resolve_course(slug)
                .unwrap_or_else(|error| panic!("booting '{slug}' failed: {error}"));
            let (race, geometry) = race_for_course(course);
            assert!(
                (race.lap_length() - lap_length).abs() < 1e-3,
                "'{slug}' booted a {:#} m lap, expected {lap_length} m",
                race.lap_length()
            );
            assert!((geometry.road_width() - course.road_width).abs() < 1e-6);
        }
    }

    /// The selection semantic the shell consumes: two track-select values,
    /// two different loaded courses.
    #[test]
    fn booting_different_tracks_loads_different_courses() {
        let coast = resolve_course("coast-run").expect("coast-run must resolve");
        let island = resolve_course("island-hop").expect("island-hop must resolve");
        assert_ne!(coast.id, island.id);
        let (coast_race, _coast_geometry) = race_for_course(coast);
        let (island_race, _island_geometry) = race_for_course(island);
        assert!(
            (coast_race.lap_length() - island_race.lap_length()).abs() > 1.0,
            "distinct track-select values must load distinct courses"
        );
    }

    /// An unknown track-select value is a boot error on the loading screen,
    /// same path as today's boot failures.
    #[test]
    fn an_unknown_track_select_value_is_a_boot_error() {
        let error = resolve_course("demo").expect_err("an unknown slug must fail to resolve");
        assert!(error.contains("demo"));
    }
}
