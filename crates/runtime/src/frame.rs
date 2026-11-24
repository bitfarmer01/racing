//! Frame composition: one tick of the race rendered into drawable state.
//!
//! [`GameFrame::advance`] advances the `game` race loop one 60 Hz tick and
//! builds everything the GPU path needs for that tick: the camera following
//! the player bike, the road mesh from the `world` crate's road geometry, and
//! the sprite batch — each bike projected through the `world` projector's
//! billboard output and pushed via `render::push_projected`. All projection
//! and batching decisions run here, on the CPU, before any device is touched.

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
/// Rumble-strip red and white, alternating along the road edges.
const RUMBLE_RED: [f32; 4] = [0.72, 0.13, 0.10, 1.0];
const RUMBLE_WHITE: [f32; 4] = [0.93, 0.93, 0.91, 1.0];
/// Lane-line white.
const LANE_LINE: [f32; 4] = [0.90, 0.90, 0.88, 1.0];
/// Sky above the horizon.
const SKY: [f32; 4] = [0.55, 0.73, 0.92, 1.0];
/// Grass below the horizon.
const GRASS: [f32; 4] = [0.30, 0.52, 0.26, 1.0];
/// Distance haze: strips fade into this with depth, grounding the horizon.
const HAZE: [f32; 4] = [0.72, 0.80, 0.88, 1.0];
/// Width of each rumble strip, in meters.
const RUMBLE_WIDTH: f32 = 1.1;
/// Width of a lane line, in meters.
const LANE_LINE_WIDTH: f32 = 0.25;
/// Lateral offsets of the lane lines dividing the road into three lanes.
const LANE_LINES: [f32; 2] = [-2.0, 2.0];
/// Spacing between roadside scenery slots, in meters.
const SCENERY_SPACING: f32 = 22.0;
/// How hard the camera leans into a corner: lateral meters per
/// (curvature x speed).
const CAMERA_LEAN: f32 = 4.0;
/// How much the view widens at full speed: focal shrinks by this fraction.
const SPEED_WIDENING: f32 = 0.25;
/// Camera shake amplitude while crashed, in world units.
const CRASH_SHAKE: f32 = 0.35;

/// One road-mesh vertex: screen-space position plus a flat color. The type
/// lives in the `render` crate (the shared GPU path draws it); re-exported
/// here so frame composition and the offscreen proof keep one import path.
pub use render::RoadVertex;

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
        self.push_quad(
            QuadEdge {
                lx: strip.near_x - strip.near_half_width,
                rx: strip.near_x + strip.near_half_width,
                y: strip.near_y,
            },
            QuadEdge {
                lx: strip.far_x - strip.far_half_width,
                rx: strip.far_x + strip.far_half_width,
                y: strip.far_y,
            },
            color,
        );
    }

    /// Appends one flat-colored quad from a near and a far edge. Triangle-
    /// strip order: near-left, near-right, far-left, far-right. Road
    /// markings, rumble strips, and the sky/ground bands all ride through
    /// here.
    fn push_quad(&mut self, near: QuadEdge, far: QuadEdge, color: [f32; 4]) {
        self.vertices.extend([
            RoadVertex {
                position: [near.lx, near.y],
                color,
            },
            RoadVertex {
                position: [near.rx, near.y],
                color,
            },
            RoadVertex {
                position: [far.lx, far.y],
                color,
            },
            RoadVertex {
                position: [far.rx, far.y],
                color,
            },
        ]);
    }

    /// Appends a screen-space rectangle as one quad: full-width sky and
    /// ground bands use this.
    fn push_screen_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) {
        self.push_quad(
            QuadEdge {
                lx: x0,
                rx: x1,
                y: y1,
            },
            QuadEdge {
                lx: x0,
                rx: x1,
                y: y0,
            },
            color,
        );
    }
}

/// One horizontal edge of a [`RoadMesh::push_quad`]: left x, right x, y.
#[derive(Clone, Copy)]
struct QuadEdge {
    lx: f32,
    rx: f32,
    y: f32,
}

/// Clamps a road strip to the viewport: strips entirely below the screen
/// vanish, and a strip straddling the bottom edge has its near edge pulled
/// up to the screen bottom, interpolating along the strip axis. Without
/// this, the nearest strips are near-plane-deep wedges whose edges sweep
/// across the whole viewport as diagonals.
fn clamp_strip_to_viewport(strip: RoadStrip) -> Option<RoadStrip> {
    if strip.far_y >= VIEWPORT_HEIGHT {
        return None;
    }
    if strip.near_y <= VIEWPORT_HEIGHT {
        return Some(strip);
    }
    let t = (VIEWPORT_HEIGHT - strip.far_y) / (strip.near_y - strip.far_y);
    let lerp = |far: f32, near: f32| far + (near - far) * t;
    Some(RoadStrip {
        near_x: lerp(strip.far_x, strip.near_x),
        near_y: VIEWPORT_HEIGHT,
        near_half_width: lerp(strip.far_half_width, strip.near_half_width),
        far_x: strip.far_x,
        far_y: strip.far_y,
        far_half_width: strip.far_half_width,
    })
}

/// Where each sprite frame lives in the packed atlas, as shader UV rects.
pub struct AtlasMapping {
    dims: AtlasDims,
    ride: [f32; 4],
    brake: [f32; 4],
    crash: [f32; 4],
    tree: [f32; 4],
    bush: [f32; 4],
    sign: [f32; 4],
    car: [f32; 4],
    cone: [f32; 4],
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
            tree: uv("tree"),
            bush: uv("bush"),
            sign: uv("sign"),
            car: uv("car"),
            cone: uv("cone"),
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

    /// UV rects for the world's dressing: scenery and traffic.
    pub fn tree_uv(&self) -> [f32; 4] {
        self.tree
    }

    pub fn bush_uv(&self) -> [f32; 4] {
        self.bush
    }

    pub fn sign_uv(&self) -> [f32; 4] {
        self.sign
    }

    pub fn car_uv(&self) -> [f32; 4] {
        self.car
    }

    pub fn cone_uv(&self) -> [f32; 4] {
        self.cone
    }
}

/// The demo track: a straight start, a long right, a rolling climb, a short
/// left, and a straight run back to the line. Shared by the race and the
/// frame builder.
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
        vec![
            world::ElevationKey {
                distance: 0.0,
                elevation: 0.0,
            },
            world::ElevationKey {
                distance: 250.0,
                elevation: 0.0,
            },
            world::ElevationKey {
                distance: 450.0,
                elevation: 7.0,
            },
            world::ElevationKey {
                distance: 650.0,
                elevation: 14.0,
            },
            world::ElevationKey {
                distance: 900.0,
                elevation: 10.0,
            },
            world::ElevationKey {
                distance: 1_100.0,
                elevation: 2.0,
            },
            world::ElevationKey {
                distance: 1_400.0,
                elevation: 9.0,
            },
            world::ElevationKey {
                distance: 1_700.0,
                elevation: 3.0,
            },
            world::ElevationKey {
                distance: 2_000.0,
                elevation: 0.0,
            },
        ],
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

/// One drawable sprite collected before painter's-order sorting: everything
/// on the road — bikes, cars, cones, scenery — projects the same way.
struct DrawItem {
    distance: f32,
    lateral: f32,
    world_width: f32,
    world_height: f32,
    uv: [f32; 4],
}

/// Blends `a` toward `b` by `t` in [0, 1], per channel. Pure.
fn lerp_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        1.0,
    ]
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
    let player_speed = race.player_speed();
    let player_distance = race.player_distance();
    let player_curvature = geometry.curvature_at(player_distance);
    let player_control = race.player_control();

    // Camera feel: lean into the corner (shifted toward the inside of the
    // bend), widen the view with speed, and shake while the rider slides.
    // The shake jitter is deterministic per tick — no clock, no host RNG.
    let mut jitter = sim::rng::XorShift64::new(race.tick_index() | 1);
    let shake = if player_control == Control::Crashed {
        CRASH_SHAKE * (race.player_crash_timer() / game::bike::CRASH_DURATION_SECS)
    } else {
        0.0
    };
    let shake_lateral = (jitter.next_f32_in(-1.0, 1.0)) * shake;
    let shake_height = (jitter.next_f32_in(-1.0, 1.0)) * shake * 0.6;
    let camera = world::Camera {
        distance: player_distance - CAMERA_TRAIL,
        height: CAMERA_HEIGHT + shake_height,
        lateral: (-player_curvature * player_speed * CAMERA_LEAN).clamp(-3.0, 3.0) + shake_lateral,
        focal: FOCAL * (1.0 - SPEED_WIDENING * (player_speed / game::bike::MAX_SPEED)),
        viewport: Viewport {
            width: VIEWPORT_WIDTH,
            height: VIEWPORT_HEIGHT,
        },
    };
    let projector = world::Projector::new(geometry, camera);
    let half_width = geometry.road_width() * 0.5;

    // Sky above the horizon, grass below, then the road over the grass.
    let horizon = projector.horizon_y();
    road.vertices.clear();
    road.push_screen_rect(0.0, 0.0, VIEWPORT_WIDTH, horizon, SKY);
    road.push_screen_rect(0.0, horizon, VIEWPORT_WIDTH, VIEWPORT_HEIGHT, GRASS);

    // Road: near-to-far trapezoid strips, alternating shades, fading into
    // the haze with depth so the horizon reads as distance.
    for (strip_index, unclamped) in projector
        .road_strips(ROAD_STEP, camera.distance + ROAD_DRAW_DISTANCE)
        .enumerate()
    {
        // Near-plane strips would sweep across the screen as diagonals;
        // clamp them to the viewport bottom or drop them.
        let Some(strip) = clamp_strip_to_viewport(unclamped) else {
            continue;
        };
        let depth = (strip_index + 1) as f32 * ROAD_STEP;
        let fog = (depth / ROAD_DRAW_DISTANCE).powi(2);
        let shade = if strip_index.is_multiple_of(2) {
            ROAD_LIGHT
        } else {
            ROAD_DARK
        };
        road.push_strip(strip, lerp_color(shade, HAZE, fog));

        // Rumble strips: red/white quads hugging both road edges.
        let rumble = if strip_index.is_multiple_of(2) {
            RUMBLE_RED
        } else {
            RUMBLE_WHITE
        };
        for side in [-1.0, 1.0] {
            road.push_quad(
                QuadEdge {
                    lx: strip.near_x + side * strip.near_half_width,
                    rx: strip.near_x
                        + side
                            * (strip.near_half_width
                                + RUMBLE_WIDTH * strip.near_half_width / half_width),
                    y: strip.near_y,
                },
                QuadEdge {
                    lx: strip.far_x + side * strip.far_half_width,
                    rx: strip.far_x
                        + side
                            * (strip.far_half_width
                                + RUMBLE_WIDTH * strip.far_half_width / half_width),
                    y: strip.far_y,
                },
                rumble,
            );
        }

        // Lane lines: dashed white, half the strips painted.
        if strip_index.is_multiple_of(2) {
            let near_d = camera.distance + strip_index as f32 * ROAD_STEP + 1.0;
            let far_d = near_d + ROAD_STEP;
            for lane in LANE_LINES {
                if let (Some(n), Some(f)) = (
                    projector.project(near_d, lane),
                    projector.project(far_d, lane),
                ) {
                    // A dash whose near edge is below the screen would sweep
                    // in from off-screen; skip it like the clamped strips.
                    if n.screen_y > VIEWPORT_HEIGHT {
                        continue;
                    }
                    road.push_quad(
                        QuadEdge {
                            lx: n.screen_x - LANE_LINE_WIDTH * n.scale * 0.5,
                            rx: n.screen_x + LANE_LINE_WIDTH * n.scale * 0.5,
                            y: n.screen_y,
                        },
                        QuadEdge {
                            lx: f.screen_x - LANE_LINE_WIDTH * f.scale * 0.5,
                            rx: f.screen_x + LANE_LINE_WIDTH * f.scale * 0.5,
                            y: f.screen_y,
                        },
                        LANE_LINE,
                    );
                }
            }
        }
    }

    // Collect every drawable: bikes, traffic cars, cone hazards, and the
    // deterministic roadside scenery, then paint far to near.
    let mut draws: Vec<DrawItem> = Vec::new();
    for (slot, state) in race.bike_states().enumerate() {
        let control = if slot == 0 {
            player_control
        } else {
            Control::Accelerate
        };
        draws.push(DrawItem {
            distance: state.pos_x,
            lateral: state.pos_y,
            world_width: BIKE_WORLD_WIDTH,
            world_height: BIKE_WORLD_HEIGHT,
            uv: mapping.frame_uv(control),
        });
    }
    for car in race.traffic_cars() {
        draws.push(DrawItem {
            distance: car.state.pos_x,
            lateral: car.state.pos_y,
            world_width: 2.0,
            world_height: 1.5,
            uv: mapping.car_uv(),
        });
    }
    for hazard in race.hazards() {
        draws.push(DrawItem {
            distance: hazard.pos_x,
            lateral: hazard.lateral,
            world_width: 0.5,
            world_height: 0.7,
            uv: mapping.cone_uv(),
        });
    }
    push_scenery(geometry, mapping, &mut draws, camera.distance);
    draws.sort_by(|a, b| b.distance.total_cmp(&a.distance));
    for item in draws {
        let style = SpriteStyle {
            uv_rect: item.uv,
            tint: [1.0, 1.0, 1.0, 1.0],
        };
        // Items behind the camera plane are culled by the projector —
        // push_projected reports that as `false` and that is expected.
        let _ = render::push_projected(
            batch,
            &projector,
            RoadPlacement {
                distance: item.distance,
                lateral: item.lateral,
                world_width: item.world_width,
                world_height: item.world_height,
            },
            style,
        );
    }
}

/// Deterministic roadside scenery: every [`SCENERY_SPACING`] meters there is
/// a slot whose seeded roll picks a tree, bush, or sign at a randomized side
/// and offset. Same distance, same dressing — no per-frame state.
fn push_scenery(
    geometry: &RoadGeometry,
    mapping: &AtlasMapping,
    draws: &mut Vec<DrawItem>,
    camera_distance: f32,
) {
    let half_width = geometry.road_width() * 0.5;
    let first = ((camera_distance + 2.0) / SCENERY_SPACING).floor() as u64;
    let last = ((camera_distance + ROAD_DRAW_DISTANCE) / SCENERY_SPACING).ceil() as u64;
    for slot in first..=last {
        let distance = slot as f32 * SCENERY_SPACING;
        let mut roll = sim::rng::XorShift64::new(slot.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let kind = roll.next_f32_in(0.0, 1.0);
        let (uv, width, height) = if kind < 0.55 {
            (mapping.tree_uv(), 4.0, 7.0)
        } else if kind < 0.8 {
            (mapping.bush_uv(), 2.2, 1.4)
        } else if kind < 0.92 {
            (mapping.sign_uv(), 1.6, 2.2)
        } else {
            continue;
        };
        let side = if roll.next_f32_in(0.0, 1.0) < 0.5 {
            1.0
        } else {
            -1.0
        };
        let lateral = side * (half_width + 1.5 + roll.next_f32_in(0.0, 9.0));
        draws.push(DrawItem {
            distance,
            lateral,
            world_width: width,
            world_height: height,
            uv,
        });
    }
}
