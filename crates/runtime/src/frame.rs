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
/// Camera height above the road surface, in world units. High enough that
/// the player sprite sits low in frame with road visible ahead (C1).
pub const CAMERA_HEIGHT: f32 = 1.9;
/// How far behind the player the camera rides, in meters. Chosen with
/// CAMERA_HEIGHT so the player bike's base lands in the lower third of the
/// screen at a readable ~28% of viewport height (C1/C2).
pub const CAMERA_TRAIL: f32 = 4.0;
/// Where the vanishing point sits, as a fraction of the viewport height from
/// the top (C2).
pub const HORIZON_FRAC: f32 = 0.4;
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

/// Asphalt light shade. The light/dark pair differs by ~40% so the strip
/// rhythm survives small screens and motion (B5).
const ROAD_LIGHT: [f32; 4] = [0.36, 0.36, 0.38, 1.0];
/// Asphalt dark shade — the alternation reads as forward motion.
const ROAD_DARK: [f32; 4] = [0.21, 0.21, 0.23, 1.0];
/// Rumble-strip red and white, alternating along the road edges.
const RUMBLE_RED: [f32; 4] = [0.72, 0.13, 0.10, 1.0];
const RUMBLE_WHITE: [f32; 4] = [0.93, 0.93, 0.91, 1.0];
/// Length of one rumble alternation, in world meters: ~2 m so the pattern
/// reads as stripes, not blocks, near the camera (B6).
const RUMBLE_SEGMENT: f32 = 2.0;
/// Lane-line white.
const LANE_LINE: [f32; 4] = [0.90, 0.90, 0.88, 1.0];
/// Width of a lane line, in meters — wide enough to read at speed (B7).
const LANE_LINE_WIDTH: f32 = 0.4;
/// Painted length of one lane dash, in meters — an 8 m cycle with a 3 m
/// dash gives a clear broken line (B7).
const LANE_DASH_LENGTH: f32 = 3.0;
/// Lateral offsets of the lane lines dividing the road into three lanes.
const LANE_LINES: [f32; 2] = [-2.0, 2.0];
/// Sky above the horizon.
const SKY: [f32; 4] = [0.55, 0.73, 0.92, 1.0];
/// Height of the haze band painted above the horizon, in pixels, so the sky
/// and the fading road meet without a hard line (B2).
const SKY_HAZE_BAND_PX: f32 = 24.0;
/// Grass light band (B4).
const GRASS_LIGHT: [f32; 4] = [0.33, 0.56, 0.27, 1.0];
/// Grass dark band — the alternation makes off-road motion read (B4).
const GRASS_DARK: [f32; 4] = [0.27, 0.47, 0.22, 1.0];
/// How far grass fades toward the haze at the draw distance. Kept just
/// under 1.0 so a whisper of green survives to the horizon — the far road
/// must not read as a detached white patch on white (B2).
const GRASS_FOG: f32 = 0.9;
/// Distance haze: strips fade into this with depth, grounding the horizon.
const HAZE: [f32; 4] = [0.72, 0.80, 0.88, 1.0];
/// Muted blue-green of the backdrop ridge silhouettes (B3).
const BACKDROP: [f32; 4] = [0.40, 0.50, 0.45, 1.0];
/// Width of each rumble strip, in meters.
const RUMBLE_WIDTH: f32 = 1.1;
/// Spacing between roadside scenery slots, in meters.
const SCENERY_SPACING: f32 = 22.0;
/// Scenery sizes in world units: bushes and signs scaled up so they identify
/// at speed; nothing dropped (D6).
const TREE_SIZE: (f32, f32) = (4.0, 7.0);
const BUSH_SIZE: (f32, f32) = (3.0, 2.0);
const SIGN_SIZE: (f32, f32) = (2.4, 3.2);
/// Traffic-cone hazard size, scaled up to stay identifiable on the road (D6).
const CONE_SIZE: (f32, f32) = (1.0, 1.3);
/// Ground-shadow sizes in world units, under scenery and vehicles (D3).
const TREE_SHADOW: (f32, f32) = (5.0, 1.6);
const BUSH_SHADOW: (f32, f32) = (2.4, 0.7);
const SIGN_SHADOW: (f32, f32) = (1.4, 0.5);
const BIKE_SHADOW: (f32, f32) = (1.8, 0.6);
const CAR_SHADOW: (f32, f32) = (2.8, 1.0);
const CONE_SHADOW: (f32, f32) = (0.9, 0.35);
/// How much farther a shadow sprite sits than its object, in meters: just
/// enough that painter's order (far to near) paints the shadow first.
const SHADOW_DEPTH_BIAS: f32 = 0.05;
/// How hard the camera leans into a corner: lateral meters per
/// (curvature x speed). Damped from 4.0 — the old value swung the road so
/// far off-centre in corners that the view read as lopsided (B8).
const CAMERA_LEAN: f32 = 2.0;
/// Hard cap on the camera's lateral lean, in meters (B8).
const CAMERA_LEAN_MAX: f32 = 1.5;
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
    rival: [f32; 4],
    shadow: [f32; 4],
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
            rival: uv("bike_rival"),
            shadow: uv("shadow"),
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

    /// The rivals' rear-view riding frame — a distinct livery from the
    /// player's (C3).
    pub fn rival_uv(&self) -> [f32; 4] {
        self.rival
    }

    /// The soft ground shadow ellipse (D3).
    pub fn shadow_uv(&self) -> [f32; 4] {
        self.shadow
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
/// `roll` is the screen-space bank in radians; only riders ever lean.
struct DrawItem {
    distance: f32,
    lateral: f32,
    world_width: f32,
    world_height: f32,
    uv: [f32; 4],
    roll: f32,
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

/// Depth fog for a distance from the camera, in [0, 1]: quadratic, so the
/// near field stays crisp and the last stretch melts into the haze. Pure.
fn fog_at(depth: f32) -> f32 {
    (depth / ROAD_DRAW_DISTANCE).powi(2)
}

/// One parallax layer of the distant backdrop (B3): a base height, an
/// amplitude, and how strongly the road's bend over the next stretch shifts
/// it sideways (far layers shift less — cheap depth parallax).
struct BackdropLayer {
    base_px: f32,
    amp_px: f32,
    wavelength_px: f32,
    phase: f32,
    /// Lateral pixels of shift per meter of road bend.
    shift_per_m: f32,
    /// How far this layer's color is blended into the haze: farther layers
    /// sit closer to the sky.
    haze: f32,
}

/// Far to near: the haze-heaviest ridge first, painter's order.
const BACKDROP_LAYERS: [BackdropLayer; 3] = [
    BackdropLayer {
        base_px: 14.0,
        amp_px: 6.0,
        wavelength_px: 110.0,
        phase: 4.4,
        shift_per_m: 0.10,
        haze: 0.72,
    },
    BackdropLayer {
        base_px: 9.0,
        amp_px: 9.0,
        wavelength_px: 150.0,
        phase: 2.1,
        shift_per_m: 0.20,
        haze: 0.50,
    },
    BackdropLayer {
        base_px: 4.0,
        amp_px: 12.0,
        wavelength_px: 210.0,
        phase: 0.8,
        shift_per_m: 0.35,
        haze: 0.25,
    },
];

/// Backdrop silhouette column width, in pixels.
const BACKDROP_COLUMN_PX: f32 = 32.0;

/// How much road bend feeds the backdrop shift: the centreline's lateral
/// drift over this window ahead of the camera. A window bounds the shift on
/// straight runs and long curves alike.
const BACKDROP_BEND_AHEAD: f32 = 150.0;

/// Silhouette height for one backdrop layer at screen x, in [0, 1]: two
/// incommensurate sines so the ridge line never visibly repeats. Pure.
fn hill_height(px: f32, layer: &BackdropLayer) -> f32 {
    let k = std::f32::consts::TAU / layer.wavelength_px;
    let a = (px * k + layer.phase).sin();
    let b = (px * k * 2.7 + layer.phase * 1.6).sin();
    (0.5 + 0.5 * (0.65 * a + 0.35 * b)).clamp(0.0, 1.0)
}

/// Paints the layered hill silhouettes between the sky and the horizon
/// (B3): one flat-color column quad per 32 px of screen, their tops traced
/// by [`hill_height`], shifted sideways by the road's bend ahead.
fn push_backdrop(
    geometry: &RoadGeometry,
    camera: &world::Camera,
    road: &mut RoadMesh,
    horizon: f32,
) {
    let bend = geometry.center_offset(camera.distance + BACKDROP_BEND_AHEAD)
        - geometry.center_offset(camera.distance);
    for layer in &BACKDROP_LAYERS {
        let shift = -bend * layer.shift_per_m;
        let color = lerp_color(BACKDROP, HAZE, layer.haze);
        let mut x = 0.0;
        while x < VIEWPORT_WIDTH {
            let x1 = (x + BACKDROP_COLUMN_PX).min(VIEWPORT_WIDTH);
            let top = horizon - layer.base_px - layer.amp_px * hill_height(x + shift, layer);
            // One pixel past the horizon: the grass bands below cover the
            // seam, so no hairline gap opens between ridge and ground.
            road.push_screen_rect(x, top, x1, horizon + 1.0, color);
            x = x1;
        }
    }
}

/// Paints one world-anchored rumble alternation: red/white sub-quads on
/// both road edges over [`d_near`, `d_far`] (B6). Edges are projected per
/// segment so the pattern tiles without cracks and fades with depth (B2).
fn push_rumble(
    projector: &world::Projector<'_>,
    camera_distance: f32,
    range_near: f32,
    range_far: f32,
    half_width: f32,
    road: &mut RoadMesh,
) {
    let mut cursor = range_near.max(camera_distance + world::NEAR_PLANE);
    let mut segment = (cursor / RUMBLE_SEGMENT).floor() as i64;
    while cursor < range_far {
        let boundary = (segment as f32 + 1.0) * RUMBLE_SEGMENT;
        let end = boundary.min(range_far);
        let color = if segment.rem_euclid(2) == 0 {
            RUMBLE_RED
        } else {
            RUMBLE_WHITE
        };
        let fog = fog_at(cursor - camera_distance);
        for side in [-1.0, 1.0] {
            if let (Some(n), Some(f)) = (
                projector.project(cursor, side * half_width),
                projector.project(end, side * half_width),
            ) {
                let inner_near = n.screen_x;
                let inner_far = f.screen_x;
                // The outer edge projects at the rumble's outer lateral.
                let (Some(n_out), Some(f_out)) = (
                    projector.project(cursor, side * (half_width + RUMBLE_WIDTH)),
                    projector.project(end, side * (half_width + RUMBLE_WIDTH)),
                ) else {
                    continue;
                };
                road.push_quad(
                    QuadEdge {
                        lx: n_out.screen_x,
                        rx: inner_near,
                        y: n.screen_y,
                    },
                    QuadEdge {
                        lx: f_out.screen_x,
                        rx: inner_far,
                        y: f.screen_y,
                    },
                    lerp_color(color, HAZE, fog),
                );
            }
        }
        cursor = end;
        segment += 1;
    }
}

/// Paints the broken centre line over one strip's distance range (B7):
/// dashes are world-anchored on the same 8 m grid, 3 m painted of every
/// cycle, and clipped to the strip so a dash never crosses a later road
/// quad and gets overdrawn.
fn push_lane_dashes(
    projector: &world::Projector<'_>,
    camera_distance: f32,
    range_near: f32,
    range_far: f32,
    road: &mut RoadMesh,
) {
    let near_floor = camera_distance + world::NEAR_PLANE;
    let first_slot = (range_near / ROAD_STEP).floor() as i64;
    let last_slot = ((range_far - 0.001) / ROAD_STEP).floor() as i64;
    for slot in first_slot..=last_slot {
        let dash_start = (slot as f32 * ROAD_STEP).max(near_floor);
        let dash_end = (slot as f32 * ROAD_STEP + LANE_DASH_LENGTH).min(range_far);
        if dash_start >= dash_end {
            continue;
        }
        let fog = fog_at(dash_start - camera_distance);
        for lane in LANE_LINES {
            if let (Some(n), Some(f)) = (
                projector.project(dash_start, lane),
                projector.project(dash_end, lane),
            ) {
                // A dash whose near edge is below the screen would sweep in
                // from off-screen; skip it like the clamped strips.
                if n.screen_y > VIEWPORT_HEIGHT {
                    continue;
                }
                let half_px = |scale: f32| LANE_LINE_WIDTH * scale * 0.5;
                road.push_quad(
                    QuadEdge {
                        lx: n.screen_x - half_px(n.scale),
                        rx: n.screen_x + half_px(n.scale),
                        y: n.screen_y,
                    },
                    QuadEdge {
                        lx: f.screen_x - half_px(f.scale),
                        rx: f.screen_x + half_px(f.scale),
                        y: f.screen_y,
                    },
                    lerp_color(LANE_LINE, HAZE, fog),
                );
            }
        }
    }
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
    // bend — the same inside the riders bank toward), widen the view with
    // speed, and shake while the rider slides.
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
        lateral: (-player_curvature * player_speed * CAMERA_LEAN)
            .clamp(-CAMERA_LEAN_MAX, CAMERA_LEAN_MAX)
            + shake_lateral,
        focal: FOCAL * (1.0 - SPEED_WIDENING * (player_speed / game::bike::MAX_SPEED)),
        horizon_y_px: VIEWPORT_HEIGHT * HORIZON_FRAC,
        viewport: Viewport {
            width: VIEWPORT_WIDTH,
            height: VIEWPORT_HEIGHT,
        },
    };
    let projector = world::Projector::new(geometry, camera);
    let half_width = geometry.road_width() * 0.5;

    // Sky above the horizon, a haze band where sky meets distance, the
    // parallax backdrop, then the banded grass and the road over it.
    let horizon = projector.horizon_y();
    road.vertices.clear();

    batch.clear();
    road.push_screen_rect(0.0, 0.0, VIEWPORT_WIDTH, horizon, SKY);
    road.push_screen_rect(
        0.0,
        horizon - SKY_HAZE_BAND_PX,
        VIEWPORT_WIDTH,
        horizon,
        lerp_color(SKY, HAZE, 0.7),
    );
    push_backdrop(geometry, &camera, road, horizon);

    // Road: near-to-far trapezoid strips, alternating shades, with grass
    // bands behind every strip so the whole ground plane fades with depth
    // (B2), not just the asphalt.
    let mut last_far_y = VIEWPORT_HEIGHT;
    for (strip_index, unclamped) in projector
        .road_strips(ROAD_STEP, camera.distance + ROAD_DRAW_DISTANCE)
        .enumerate()
    {
        // Near-plane strips would sweep across the screen as diagonals;
        // clamp them to the viewport bottom or drop them.
        let Some(strip) = clamp_strip_to_viewport(unclamped) else {
            continue;
        };
        last_far_y = last_far_y.min(strip.far_y);
        let depth = (strip_index + 1) as f32 * ROAD_STEP;
        let fog = fog_at(depth);

        // Grass band for this strip's depth range, full width (B4).
        let grass_shade = if strip_index.is_multiple_of(2) {
            GRASS_LIGHT
        } else {
            GRASS_DARK
        };
        road.push_screen_rect(
            0.0,
            strip.far_y,
            VIEWPORT_WIDTH,
            strip.near_y,
            lerp_color(grass_shade, HAZE, fog * GRASS_FOG),
        );

        // Asphalt: alternating shades fading into the haze with depth.
        let shade = if strip_index.is_multiple_of(2) {
            ROAD_LIGHT
        } else {
            ROAD_DARK
        };
        road.push_strip(strip, lerp_color(shade, HAZE, fog));

        // Rumble strips and lane dashes are anchored to the world's own
        // 8 m grid, so their pattern sits still on the road instead of
        // crawling with the camera.
        let range_near = camera.distance + (strip_index as f32 * ROAD_STEP);
        let range_far = range_near + ROAD_STEP;
        push_rumble(
            &projector,
            camera.distance,
            range_near,
            range_far,
            half_width,
            road,
        );
        push_lane_dashes(&projector, camera.distance, range_near, range_far, road);
    }

    // The sliver between the last strip and the horizon: fully hazed grass,
    // matching what the bands fade into (B2).
    road.push_screen_rect(
        0.0,
        horizon,
        VIEWPORT_WIDTH,
        last_far_y,
        lerp_color(GRASS_LIGHT, HAZE, GRASS_FOG),
    );

    // Collect every drawable: bikes, traffic cars, cone hazards, and the
    // deterministic roadside scenery, then paint far to near. Each item's
    // ground shadow is a separate sprite a hair farther away, so painter's
    // order drops it onto the road just before the object (D3).
    let mut draws: Vec<DrawItem> = Vec::new();
    let shadow_uv = mapping.shadow_uv();
    for (slot, state) in race.bike_states().enumerate() {
        let control = if slot == 0 {
            player_control
        } else {
            race.bike_control(slot)
        };
        // The player's rear view carries the control-state frames; rivals
        // ride their own livery, down to the shared crash pose (C3).
        let uv = if slot == 0 {
            mapping.frame_uv(control)
        } else if control == Control::Crashed {
            mapping.frame_uv(Control::Crashed)
        } else {
            mapping.rival_uv()
        };
        draws.push(DrawItem {
            distance: state.pos_x + SHADOW_DEPTH_BIAS,
            lateral: state.pos_y,
            world_width: BIKE_SHADOW.0,
            world_height: BIKE_SHADOW.1,
            uv: shadow_uv,
            roll: 0.0,
        });
        draws.push(DrawItem {
            distance: state.pos_x,
            lateral: state.pos_y,
            world_width: BIKE_WORLD_WIDTH,
            world_height: BIKE_WORLD_HEIGHT,
            uv,
            // The rider's bank: the sim carries the lean, the instance
            // transform rolls the quad about its ground contact.
            roll: race.bike_lean(slot),
        });
    }
    for car in race.traffic_cars() {
        draws.push(DrawItem {
            distance: car.state.pos_x + SHADOW_DEPTH_BIAS,
            lateral: car.state.pos_y,
            world_width: CAR_SHADOW.0,
            world_height: CAR_SHADOW.1,
            uv: shadow_uv,
            roll: 0.0,
        });
        draws.push(DrawItem {
            distance: car.state.pos_x,
            lateral: car.state.pos_y,
            world_width: 2.0,
            world_height: 1.5,
            uv: mapping.car_uv(),
            roll: 0.0,
        });
    }
    for hazard in race.hazards() {
        draws.push(DrawItem {
            distance: hazard.pos_x + SHADOW_DEPTH_BIAS,
            lateral: hazard.lateral,
            world_width: CONE_SHADOW.0,
            world_height: CONE_SHADOW.1,
            uv: shadow_uv,
            roll: 0.0,
        });
        draws.push(DrawItem {
            distance: hazard.pos_x,
            lateral: hazard.lateral,
            world_width: CONE_SIZE.0,
            world_height: CONE_SIZE.1,
            uv: mapping.cone_uv(),
            roll: 0.0,
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
                roll: item.roll,
            },
            style,
        );
    }
}

/// Deterministic roadside scenery: every [`SCENERY_SPACING`] meters there is
/// a slot whose seeded roll picks a tree, bush, or sign. Sides alternate by
/// slot — guaranteed balance across the road, however the seeded rolls fall
/// — with an occasional opposite-side understory bush (D4). Same distance,
/// same dressing — no per-frame state.
fn push_scenery(
    geometry: &RoadGeometry,
    mapping: &AtlasMapping,
    draws: &mut Vec<DrawItem>,
    camera_distance: f32,
) {
    let half_width = geometry.road_width() * 0.5;
    let shadow_uv = mapping.shadow_uv();
    let first = ((camera_distance + 2.0) / SCENERY_SPACING).floor() as u64;
    let last = ((camera_distance + ROAD_DRAW_DISTANCE) / SCENERY_SPACING).ceil() as u64;
    for slot in first..=last {
        let distance = slot as f32 * SCENERY_SPACING;
        let mut roll = sim::rng::XorShift64::new(slot.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let kind = roll.next_f32_in(0.0, 1.0);
        let (uv, (width, height), shadow) = if kind < 0.55 {
            (mapping.tree_uv(), TREE_SIZE, TREE_SHADOW)
        } else if kind < 0.8 {
            (mapping.bush_uv(), BUSH_SIZE, BUSH_SHADOW)
        } else if kind < 0.92 {
            (mapping.sign_uv(), SIGN_SIZE, SIGN_SHADOW)
        } else {
            continue;
        };
        // Alternate sides by slot parity: even slots right, odd slots left.
        let side = if slot % 2 == 0 { 1.0 } else { -1.0 };
        let lateral = side * (half_width + 1.5 + roll.next_f32_in(0.0, 9.0));
        draws.push(DrawItem {
            distance: distance + SHADOW_DEPTH_BIAS,
            lateral,
            world_width: shadow.0,
            world_height: shadow.1,
            uv: shadow_uv,
            roll: 0.0,
        });
        draws.push(DrawItem {
            distance,
            lateral,
            world_width: width,
            world_height: height,
            uv,
            roll: 0.0,
        });
        // A quarter of slots also get a small bush on the opposite verge,
        // so neither side of the road ever reads empty (D4).
        if roll.next_f32_in(0.0, 1.0) < 0.25 {
            let opposite = -side * (half_width + 1.5 + roll.next_f32_in(0.0, 9.0));
            draws.push(DrawItem {
                distance: distance + SHADOW_DEPTH_BIAS,
                lateral: opposite,
                world_width: BUSH_SHADOW.0,
                world_height: BUSH_SHADOW.1,
                uv: shadow_uv,
                roll: 0.0,
            });
            draws.push(DrawItem {
                distance,
                lateral: opposite,
                world_width: BUSH_SIZE.0,
                world_height: BUSH_SIZE.1,
                uv: mapping.bush_uv(),
                roll: 0.0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack_placeholder_atlas;

    /// A mapping over the packed placeholder atlas, shared by these tests.
    fn mapping() -> AtlasMapping {
        let packed = pack_placeholder_atlas().expect("placeholder atlas packs");
        AtlasMapping::from_packed(&packed)
    }

    /// Releases the grid, rolls the player up to speed, then banks left for
    /// a few ticks.
    fn banked_left_race() -> (Race, RoadGeometry) {
        let (mut race, geometry) = demo_race();
        while race.is_counting_down() {
            race.tick();
        }
        for _ in 0..30 {
            race.push_input(GameInput::Accelerate);
            race.tick();
        }
        for _ in 0..3 {
            race.push_input(GameInput::SteerLeft);
            race.tick();
        }
        (race, geometry)
    }

    #[test]
    fn a_banked_rider_renders_a_rolled_sprite() {
        let mapping = mapping();
        let (race, geometry) = banked_left_race();
        assert!(race.player_lean() < -0.1, "the rider is banked left");

        let frame = GameFrame::build(&race, &geometry, &mapping);
        let rolls: Vec<f32> = frame
            .batch
            .instances()
            .iter()
            .map(|instance| instance.rotation)
            .collect();
        assert!(
            rolls.iter().any(|r| *r < -0.05),
            "at least the player's sprite is rolled left: {rolls:?}"
        );
        // Shadows, cones, cars, and scenery never roll: every roll in the
        // batch is a rider's bank, within the state machine's cap.
        assert!(rolls.iter().all(|r| r.abs() <= game::bike::LEAN_MAX + 1e-3));
    }

    #[test]
    fn an_upright_pack_renders_no_rolled_sprites() {
        let mapping = mapping();
        let (race, geometry) = demo_race();
        // Still on the grid: everyone upright.
        let frame = GameFrame::build(&race, &geometry, &mapping);
        assert!(frame
            .batch
            .instances()
            .iter()
            .all(|instance| instance.rotation == 0.0));
    }

    #[test]
    fn a_reused_batch_refills_per_frame_instead_of_piling_up() {
        // Regression: compose() reuses pre-owned batch and road buffers on
        // the steady-state path. A batch never cleared fills once, then
        // every later sprite push is rejected — the screen freezes on the
        // first frames' sprites while the road keeps animating.
        let mapping = mapping();
        let (mut race, geometry) = demo_race();
        let mut batch = SpriteBatch::for_tier(render::QualityTier::WebGpuCapable);
        let mut road = RoadMesh::default();

        advance_game_frame(
            &mut race,
            &geometry,
            &mapping,
            &[GameInput::Accelerate],
            &mut batch,
            &mut road,
        );
        let first_len = batch.len();
        assert!(first_len > 0, "a frame draws sprites");

        // Ten more frames on the same buffers: the player is accelerating
        // down the road, so every frame's sprite set is a different one.
        for _ in 0..10 {
            advance_game_frame(
                &mut race,
                &geometry,
                &mapping,
                &[GameInput::Accelerate],
                &mut batch,
                &mut road,
            );
        }
        assert_eq!(
            batch.len(),
            first_len,
            "each frame refills the batch; it must never grow"
        );

        // And the sprites are current, not a pile of stale first frames:
        // composing the same race into a fresh batch must agree with the
        // reused one, instance for instance.
        let mut fresh = SpriteBatch::for_tier(render::QualityTier::WebGpuCapable);
        compose(&race, &geometry, &mapping, &mut fresh, &mut road);
        let reused_positions: Vec<[f32; 2]> =
            batch.instances().iter().map(|i| i.position).collect();
        let fresh_positions: Vec<[f32; 2]> = fresh.instances().iter().map(|i| i.position).collect();
        assert_eq!(reused_positions, fresh_positions, "no stale sprites");
    }
}
