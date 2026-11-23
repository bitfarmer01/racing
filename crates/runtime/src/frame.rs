//! Frame composition: one tick of the race rendered into drawable state.
//!
//! [`GameFrame::advance`] advances the `game` race loop one 60 Hz tick and
//! builds everything the GPU path needs for that tick: the camera following
//! the player bike, the road mesh from the `world` crate's road geometry, and
//! the sprite batch — each bike projected through the `world` projector's
//! billboard output and pushed via `render::push_projected`. All projection
//! and batching decisions run here, on the CPU, before any device is touched.

use bytemuck::{Pod, Zeroable};
use game::bike::Control;
use game::input::GameInput;
use game::race::{Race, RaceConfig};
use render::{AtlasDims, OrthoCamera, RoadPlacement, SpriteBatch, SpriteStyle};
use world::{RoadGeometry, RoadStrip, Viewport};

/// Proof/viewport width in pixels.
pub const VIEWPORT_WIDTH: f32 = 800.0;
/// Proof/viewport height in pixels.
pub const VIEWPORT_HEIGHT: f32 = 450.0;
/// Camera height above the road surface, in world units.
pub const CAMERA_HEIGHT: f32 = 1.3;
/// How far behind the player the camera rides, in meters.
pub const CAMERA_TRAIL: f32 = 6.0;
/// Focal length in pixels: pixels-per-world-unit at one unit of depth.
pub const FOCAL: f32 = 300.0;
/// Road strip depth step, in meters.
pub const ROAD_STEP: f32 = 8.0;
/// How far down the road strips are drawn, in meters from the camera.
pub const ROAD_DRAW_DISTANCE: f32 = 400.0;
/// Bike sprite width in world units.
pub const BIKE_WORLD_WIDTH: f32 = 1.0;
/// Bike sprite height in world units.
pub const BIKE_WORLD_HEIGHT: f32 = 1.7;

/// Asphalt light shade.
const ROAD_LIGHT: [f32; 4] = [0.30, 0.30, 0.32, 1.0];
/// Asphalt dark shade — the alternation reads as forward motion.
const ROAD_DARK: [f32; 4] = [0.25, 0.25, 0.27, 1.0];

/// One road-mesh vertex: screen-space position plus a flat color.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct RoadVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

/// The road as drawn: one triangle-strip quad (4 vertices) per road strip,
/// screen-space, trapezoid-shaped to match the projected road.
#[derive(Debug, Default)]
pub struct RoadMesh {
    pub vertices: Vec<RoadVertex>,
}

impl RoadMesh {
    /// Appends one strip as a triangle-strip quad: near-left, near-right,
    /// far-left, far-right.
    fn push_strip(&mut self, strip: RoadStrip, color: [f32; 4]) {
        self.vertices.extend([
            RoadVertex {
                position: [strip.near_x - strip.near_half_width, strip.near_y],
                color,
            },
            RoadVertex {
                position: [strip.near_x + strip.near_half_width, strip.near_y],
                color,
            },
            RoadVertex {
                position: [strip.far_x - strip.far_half_width, strip.far_y],
                color,
            },
            RoadVertex {
                position: [strip.far_x + strip.far_half_width, strip.far_y],
                color,
            },
        ]);
    }
}

/// Where each bike frame lives in the packed atlas, as shader UV rects.
pub struct AtlasMapping {
    dims: AtlasDims,
    ride: [f32; 4],
    brake: [f32; 4],
    crash: [f32; 4],
}

impl AtlasMapping {
    /// Reads the mapping out of a packed placeholder atlas. Panics on a
    /// missing frame name: the placeholder set is fixed in this crate, so a
    /// miss is a programming error, not a runtime condition.
    pub fn from_packed(packed: &assets::PackedAtlas) -> Self {
        let dims = AtlasDims::new(packed.atlas.width(), packed.atlas.height());
        let uv = |name: &str| {
            let rect = packed
                .index
                .get(name)
                .unwrap_or_else(|| panic!("placeholder atlas is missing '{name}'"));
            dims.uv_inset(*rect, 0.5)
        };
        Self {
            dims,
            ride: uv("bike_ride"),
            brake: uv("bike_brake"),
            crash: uv("bike_crash"),
        }
    }

    /// Atlas dimensions, for texture upload.
    pub fn dims(&self) -> AtlasDims {
        self.dims
    }

    /// The UV rect matching a bike's control state.
    pub fn frame_uv(&self, control: Control) -> [f32; 4] {
        match control {
            Control::Brake => self.brake,
            Control::Crashed => self.crash,
            Control::Accelerate | Control::Coast => self.ride,
        }
    }
}

/// The demo track: a straight start, a long right, a short left, and a
/// straight run back. Shared by the race and the frame builder.
pub fn track_geometry() -> RoadGeometry {
    RoadGeometry::new(
        vec![
            world::CurveSegment {
                start: 200.0,
                length: 700.0,
                curvature: -0.004,
            },
            world::CurveSegment {
                start: 1200.0,
                length: 400.0,
                curvature: 0.005,
            },
        ],
        Vec::new(),
        12.0,
    )
}

/// A demo race on [`track_geometry`]: the player plus three rivals, three
/// laps. The caller keeps the geometry alongside the race; `Race` owns it
/// privately and the frame builder needs to borrow it again.
pub fn demo_race() -> (Race, RoadGeometry) {
    let geometry = track_geometry();
    let race = Race::new(RaceConfig {
        rivals: 3,
        track_length: 2_000.0,
        total_laps: 3,
        seed: 0x5EED_C0DE,
        geometry: geometry.clone(),
    });
    (race, geometry)
}

/// Everything the GPU path needs to draw one composed frame.
pub struct GameFrame {
    pub camera: OrthoCamera,
    pub road: RoadMesh,
    pub batch: SpriteBatch,
}

impl GameFrame {
    /// Builds drawable state from the race's current state. Allocates the
    /// batch and road mesh — the per-frame path reuses them via
    /// [`advance_game_frame`](Self) into pre-owned buffers instead.
    pub fn build(race: &Race, geometry: &RoadGeometry, mapping: &AtlasMapping) -> Self {
        let mut frame = Self {
            camera: OrthoCamera::new(VIEWPORT_WIDTH, VIEWPORT_HEIGHT),
            road: RoadMesh::default(),
            batch: SpriteBatch::for_tier(render::QualityTier::WebGpuCapable),
        };
        compose(race, geometry, mapping, &mut frame.batch, &mut frame.road);
        frame
    }

    /// Advances the race one 60 Hz tick (queueing `inputs` first) and builds
    /// the frame for the new state.
    pub fn advance(
        race: &mut Race,
        geometry: &RoadGeometry,
        mapping: &AtlasMapping,
        inputs: &[GameInput],
    ) -> Self {
        for input in inputs {
            race.push_input(*input);
        }
        race.tick();
        Self::build(race, geometry, mapping)
    }
}

/// Advances the race one tick and fills pre-owned batch and road buffers —
/// the steady-state per-frame path, no reallocation after warmup.
pub fn advance_game_frame(
    race: &mut Race,
    geometry: &RoadGeometry,
    mapping: &AtlasMapping,
    inputs: &[GameInput],
    batch: &mut SpriteBatch,
    road: &mut RoadMesh,
) {
    for input in inputs {
        race.push_input(*input);
    }
    race.tick();
    compose(race, geometry, mapping, batch, road);
}

/// Builds camera-following drawable state from the live race, filling the
/// batch and road mesh.
fn compose(
    race: &Race,
    geometry: &RoadGeometry,
    mapping: &AtlasMapping,
    batch: &mut SpriteBatch,
    road: &mut RoadMesh,
) {
    // The camera rides behind the player, so the player bike is on screen
    // (in front of the near plane) and the road streams underneath.
    let camera = world::Camera {
        distance: race.player_distance() - CAMERA_TRAIL,
        height: CAMERA_HEIGHT,
        focal: FOCAL,
        viewport: Viewport {
            width: VIEWPORT_WIDTH,
            height: VIEWPORT_HEIGHT,
        },
    };
    let projector = world::Projector::new(geometry, camera);

    // Road: near-to-far trapezoid strips, alternating shades.
    road.vertices.clear();
    for (index, strip) in projector
        .road_strips(ROAD_STEP, camera.distance + ROAD_DRAW_DISTANCE)
        .enumerate()
    {
        road.push_strip(
            strip,
            if index % 2 == 0 {
                ROAD_LIGHT
            } else {
                ROAD_DARK
            },
        );
    }

    // Bikes: far to near for correct painter's order. The player keeps the
    // state machine's frame; rivals are always shown riding.
    let mut riders: Vec<(usize, sim::pool::BikeState)> = race.bike_states().enumerate().collect();
    riders.sort_by(|a, b| b.1.pos_x.total_cmp(&a.1.pos_x));
    for (slot, state) in &riders {
        let control = if *slot == 0 {
            race.player_control()
        } else {
            Control::Accelerate
        };
        let style = SpriteStyle {
            uv_rect: mapping.frame_uv(control),
            tint: [1.0, 1.0, 1.0, 1.0],
        };
        // Bikes behind the camera plane are culled by the projector —
        // push_projected reports that as `false` and that is expected.
        let _ = render::push_projected(
            batch,
            &projector,
            RoadPlacement {
                distance: state.pos_x,
                lateral: state.pos_y,
                world_width: BIKE_WORLD_WIDTH,
                world_height: BIKE_WORLD_HEIGHT,
            },
            style,
        );
    }
}
