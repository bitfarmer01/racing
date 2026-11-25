//! Frame composition: one tick of the race rendered into drawable state.
//!
//! [`GameFrame::advance`] advances the `game` race loop one 60 Hz tick and
//! builds everything the GPU path needs for that tick: the camera following
//! the player bike, the road mesh from the `world` crate's road geometry, and
//! the sprite batch — each bike projected through the `world` projector's
//! billboard output and pushed via `render::push_projected`. All projection
//! and batching decisions run here, on the CPU, before any device is touched.

use game::bike::Control;
use game::combat::Attack;
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
/// Asphalt dark shade — the alternation reads as forward motion, softened
/// toward the HD bar (≤ 0.08 delta).
const ROAD_DARK: [f32; 4] = [0.29, 0.29, 0.31, 1.0];
/// Rumble-strip red and white, alternating along the road edges.
const RUMBLE_RED: [f32; 4] = [0.72, 0.13, 0.10, 1.0];
const RUMBLE_WHITE: [f32; 4] = [0.93, 0.93, 0.91, 1.0];
/// Length of one rumble alternation, in world meters: ~2 m so the pattern
/// reads as stripes, not blocks, near the camera (B6).
const RUMBLE_SEGMENT: f32 = 2.0;
/// Lane-line white.
const LANE_LINE: [f32; 4] = [0.90, 0.90, 0.88, 1.0];
/// Width of a lane line, in meters — wide enough to read at speed (B7).
const LANE_LINE_WIDTH: f32 = 0.25;
/// Painted length of one lane dash, in meters — an 8 m cycle with a 3 m
/// dash gives a clear broken line (B7).
const LANE_DASH_LENGTH: f32 = 2.0;
/// Lateral offsets of the lane lines dividing the road into three lanes.
const LANE_LINES: [f32; 2] = [-2.0, 2.0];
/// Sky above the horizon.
const SKY: [f32; 4] = [0.55, 0.73, 0.92, 1.0];

/// The sky is four stacked bands — deep zenith blue fading down to the haze
/// — instead of one flat fill; three extra quads, no per-frame cost.
const SKY_GRADIENT: [[f32; 4]; 3] = [
    [0.42, 0.62, 0.88, 1.0],
    [0.48, 0.67, 0.90, 1.0],
    [0.52, 0.70, 0.91, 1.0],
];

/// Rival size spread per pack slot: same rider class, slight build
/// differences so the field doesn't read as clones.
const RIVAL_SCALES: [f32; 3] = [1.0, 0.92, 1.06];

/// The sky's color as a function of height, `0.0` at the zenith (screen top)
/// and `1.0` at the horizon: the same stops the composed quad uses, linearly
/// interpolated between them — the gradient is continuous, with no band
/// edges to see (B1, HD direction). Pure — tests hold the look to this.
pub fn sky_color_at(frac: f32) -> [f32; 4] {
    let stops = [
        (0.0, SKY_GRADIENT[0]),
        (0.3, SKY_GRADIENT[1]),
        (0.6, SKY_GRADIENT[2]),
        (0.85, SKY),
        (1.0, lerp_color(SKY, HAZE, 0.7)),
    ];
    let frac = frac.clamp(0.0, 1.0);
    for window in stops.windows(2) {
        let (a0, ca) = window[0];
        let (a1, cb) = window[1];
        if frac <= a1 {
            let t = if a1 > a0 {
                (frac - a0) / (a1 - a0)
            } else {
                0.0
            };
            return lerp_color(ca, cb, t);
        }
    }
    stops[stops.len() - 1].1
}

/// Grass light band (B4).
const GRASS_LIGHT: [f32; 4] = [0.33, 0.56, 0.27, 1.0];
/// Grass dark band — the alternation makes off-road motion read (B4),
/// softened toward the HD bar (≤ 0.06 delta).
const GRASS_DARK: [f32; 4] = [0.29, 0.51, 0.24, 1.0];
/// How far grass fades toward the haze at the draw distance. Kept just
/// under 1.0 so a whisper of green survives to the horizon — the far road
/// must not read as a detached white patch on white (B2).
const GRASS_FOG: f32 = 0.9;
/// Distance haze: strips fade into this with depth, grounding the horizon.
const HAZE: [f32; 4] = [0.72, 0.80, 0.88, 1.0];
/// Muted blue-green of the backdrop ridge silhouettes (B3).
const BACKDROP: [f32; 4] = [0.40, 0.50, 0.45, 1.0];
/// How far the zone's sky tint pulls the ridge colour (item 9): enough that
/// each zone's ridge reads as its own, little enough that the ridge stays
/// the same grey silhouette with its pale haze band everywhere.
const BACKDROP_ZONE_TINT: f32 = 0.25;
/// Lap length of the demo course, in meters — zones and landmarks wrap
/// per lap (7c).
pub const LAP_LENGTH: f32 = 2_000.0;
/// How strongly the zone's sky tint pulls the gradient, per channel (7c).
const SKY_TINT_STRENGTH: f32 = 0.45;
/// How strongly the zone's grass tint pulls the alternating bands (7c).
const GRASS_TINT_STRENGTH: f32 = 0.45;
/// Zone dressing sizes in world units: palm, town block, ridge boulder (7c).
const PALM_SIZE: (f32, f32) = (5.0, 9.0);
const BUILDING_SIZE: (f32, f32) = (11.0, 14.0);
const BUILDING_SHADOW: (f32, f32) = (10.0, 3.0);
const ROCK_SIZE: (f32, f32) = (3.4, 2.4);
const ROCK_SHADOW: (f32, f32) = (3.2, 0.9);
/// Dense-zone detail: fence runs and power poles (7c).
const FENCE_SIZE: (f32, f32) = (8.0, 1.3);
const FENCE_SHADOW: (f32, f32) = (7.0, 0.5);
const POLE_SIZE: (f32, f32) = (1.6, 9.0);
const POLE_SHADOW: (f32, f32) = (1.6, 0.6);
/// Power-pole spacing in dense zones, in meters (7c).
const POLE_SPACING: f32 = 66.0;
/// Distance-marker posts: one every 500 m on both road edges, on a fixed
/// world-anchored grid (7c).
const MARKER_STEP: f32 = 500.0;
const MARKER_LATERAL: f32 = 7.2;
const MARKER_SIZE: (f32, f32) = (0.8, 1.6);
/// How far landmarks stay on the draw list: beyond this they are haze (7c).
const LANDMARK_DRAW_DISTANCE: f32 = 800.0;
/// Fog cap on landmarks: a ghost of the shape survives in the haze, so the
/// structure fades in instead of popping (7c).
const LANDMARK_FOG_MAX: f32 = 0.85;
/// Landmark sizes in world units: the large "you are here" anchors (7c).
const WATER_TOWER_SIZE: (f32, f32) = (14.0, 20.0);
const WINDMILL_SIZE: (f32, f32) = (12.0, 22.0);
const BILLBOARD_SIZE: (f32, f32) = (10.0, 8.0);
const LANDMARK_SHADOW: (f32, f32) = (12.0, 3.0);
/// Width of each rumble strip, in meters.
const RUMBLE_WIDTH: f32 = 1.1;
/// Spacing between roadside scenery slots, in meters.
const SCENERY_SPACING: f32 = 22.0;
/// Scenery sizes in world units: bushes and signs scaled up so they identify
/// at speed; nothing dropped (D6).
const TREE_SIZE: (f32, f32) = (4.0, 7.0);
const BROADLEAF_SIZE: (f32, f32) = (5.5, 6.5);
const TALL_PINE_SIZE: (f32, f32) = (2.8, 8.5);
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
/// How hard the camera leans with the player's heading: lateral meters per
/// radian of bike heading. The primary term — the camera leans with where
/// the rider is pointing first, so it never leans against the player.
const CAMERA_HEADING_LEAN: f32 = 4.0;
/// How hard the camera leans into a road bend, second to the player's
/// heading: lateral meters per (curvature x speed). Damped from 4.0 — the
/// old value swung the road so far off-centre in corners that the view
/// read as lopsided (B8).
const CAMERA_LEAN: f32 = 2.0;
/// Hard cap on the camera's lateral lean, in meters (B8).
const CAMERA_LEAN_MAX: f32 = 1.5;

/// The camera's corner-drift term, in lateral meters (2b): how the camera
/// pans for the road's bend. The sign convention makes `RoadGeometry`
/// authoritative — negative curvature is a left bend, center_offset falls —
/// and the pan must act *opposite* to the sign of center_offset's change:
/// in a left bend the camera pans left (negative lateral), which slides the
/// world right on screen, into the corner the road is opening toward.
/// Pure.
fn camera_bend_lean(curvature: f32, speed: f32) -> f32 {
    curvature * speed * CAMERA_LEAN
}
/// How much the view widens at full speed: focal shrinks by this fraction.
const SPEED_WIDENING: f32 = 0.25;
/// Camera shake amplitude while crashed, in world units.
const CRASH_SHAKE: f32 = 0.35;
/// Camera shake while on the shoulder rumble strip — the warning the
/// rider feels before running off the road at speed is a crash.
const RUMBLE_SHAKE: f32 = 0.09;

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

    /// Appends a screen-space rectangle as one quad shading from `top` at
    /// `y0` to `bottom` at `y1`. Vertex colors interpolate linearly across
    /// the quad, so the shade is continuous — the sky gradient and the
    /// depth-fogged grass bands ride through here (B1, HD direction).
    fn push_screen_rect_gradient(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        top: [f32; 4],
        bottom: [f32; 4],
    ) {
        self.vertices.extend([
            RoadVertex {
                position: [x0, y1],
                color: bottom,
            },
            RoadVertex {
                position: [x1, y1],
                color: bottom,
            },
            RoadVertex {
                position: [x0, y0],
                color: top,
            },
            RoadVertex {
                position: [x1, y0],
                color: top,
            },
        ]);
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
    punch: [f32; 4],
    kick: [f32; 4],
    /// The rivals' liveries, one per pack slot cycling `(slot - 1) % 3`.
    rival: [f32; 4],
    rival2: [f32; 4],
    rival3: [f32; 4],
    /// Swing poses per rival livery, same livery order.
    rival_punch: [[f32; 4]; 3],
    rival_kick: [[f32; 4]; 3],
    shadow: [f32; 4],
    tree: [f32; 4],
    tree2: [f32; 4],
    tree3: [f32; 4],
    bush: [f32; 4],
    sign: [f32; 4],
    car: [f32; 4],
    cone: [f32; 4],
    /// Wave 7c zone dressing and landmark frames.
    palm: [f32; 4],
    building: [f32; 4],
    fence: [f32; 4],
    pole: [f32; 4],
    marker: [f32; 4],
    rock: [f32; 4],
    water_tower: [f32; 4],
    windmill: [f32; 4],
    billboard: [f32; 4],
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
            punch: uv("bike_punch"),
            kick: uv("bike_kick"),
            rival: uv("bike_rival"),
            rival2: uv("bike_rival2"),
            rival3: uv("bike_rival3"),
            rival_punch: [
                uv("bike_punch_rival"),
                uv("bike_punch_rival2"),
                uv("bike_punch_rival3"),
            ],
            rival_kick: [
                uv("bike_kick_rival"),
                uv("bike_kick_rival2"),
                uv("bike_kick_rival3"),
            ],
            shadow: uv("shadow"),
            tree: uv("tree"),
            tree2: uv("tree2"),
            tree3: uv("tree3"),
            bush: uv("bush"),
            sign: uv("sign"),
            car: uv("car"),
            cone: uv("cone"),
            palm: uv("palm"),
            building: uv("building"),
            fence: uv("fence"),
            pole: uv("pole"),
            marker: uv("marker"),
            rock: uv("rock"),
            water_tower: uv("water_tower"),
            windmill: uv("windmill"),
            billboard: uv("billboard"),
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
    pub fn tree_variant_uv(&self, slot: usize) -> [f32; 4] {
        match slot % 3 {
            0 => self.tree,
            1 => self.tree2,
            _ => self.tree3,
        }
    }

    /// The rivals' rear-view riding frames — distinct liveries indexed by
    /// pack slot (`slot - 1`), so the pack never shows two identical bikes
    /// side by side (C3).
    pub fn rival_uv(&self, slot: usize) -> [f32; 4] {
        match (slot - 1) % 3 {
            0 => self.rival,
            1 => self.rival2,
            _ => self.rival3,
        }
    }

    /// The mid-swing frame for a rider's attack: the player's own pair, or
    /// the swing pair cut in the striking rival's livery.
    pub fn swing_uv(&self, attack: Attack, slot: usize) -> [f32; 4] {
        let livery = (slot - 1) % 3;
        match (attack, slot) {
            (Attack::Punch, 0) => self.punch,
            (Attack::Kick, 0) => self.kick,
            (Attack::Punch, _) => self.rival_punch[livery],
            (Attack::Kick, _) => self.rival_kick[livery],
        }
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

    /// Wave 7c zone dressing frames.
    pub fn palm_uv(&self) -> [f32; 4] {
        self.palm
    }

    pub fn building_uv(&self) -> [f32; 4] {
        self.building
    }

    pub fn fence_uv(&self) -> [f32; 4] {
        self.fence
    }

    pub fn pole_uv(&self) -> [f32; 4] {
        self.pole
    }

    pub fn marker_uv(&self) -> [f32; 4] {
        self.marker
    }

    pub fn rock_uv(&self) -> [f32; 4] {
        self.rock
    }

    /// Landmark frames — the large "you are here" anchors.
    pub fn water_tower_uv(&self) -> [f32; 4] {
        self.water_tower
    }

    pub fn windmill_uv(&self) -> [f32; 4] {
        self.windmill
    }

    pub fn billboard_uv(&self) -> [f32; 4] {
        self.billboard
    }
}

/// The demo track: a straight start, a long gentle left into a medium
/// right, a climbing hairpin right, a descent into a mirrored hairpin
/// left, and a straight run back to the line — all over a rolling hill
/// profile. Shared by the race and the frame builder.
pub fn track_geometry() -> RoadGeometry {
    RoadGeometry::new(
        vec![
            world::CurveSegment {
                start: 200.0,
                length: 500.0,
                // Opening bend: winnable flat out under the corner-force
                // balance — a fight but a survivable one.
                curvature: -0.003,
            },
            world::CurveSegment {
                start: 700.0,
                length: 200.0,
                // Medium right out of the opening left: rideable flat with
                // a steady hand, but it punishes a lazy line.
                curvature: 0.009,
            },
            world::CurveSegment {
                start: 1_100.0,
                length: 140.0,
                // The Ridgeback right: a climbing hairpin — flat out, the
                // bend outlasts the grip; brake to the corner speed or run
                // off the tarmac.
                curvature: 0.016,
            },
            world::CurveSegment {
                start: 1_450.0,
                length: 140.0,
                // Mirror hairpin left, entered on the descent from the
                // crest: same bargain, opposite direction.
                curvature: -0.016,
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
        // One lap = one pass through every zone; landmarks and dressing
        // repeat per lap on the same fixed distances (7c).
        track_length: LAP_LENGTH,
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
        compose(
            race,
            geometry,
            mapping,
            &mut frame.batch,
            &mut frame.road,
            &mut Vec::new(),
            &TickBlend::identity(race),
        );
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
/// the steady-state per-tick path, no reallocation after warmup. Draws the
/// exact tick it ran: between-tick interpolation needs the caller's frame
/// delta, which this signature does not carry — [`GameSession::advance`] is
/// the interpolation-aware steady-state path.
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
    // The shell's per-tick path has no buffer of its own for the draw list
    // (its signature predates it), so the list lives in a thread-local and
    // is reused across ticks — one grow, then none. Single-threaded wasm and
    // test hosts; never re-entered (compose does not call back in here).
    thread_local! {
        static DRAWS: std::cell::RefCell<Vec<DrawItem>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    DRAWS.with_borrow_mut(|draws| {
        compose(
            race,
            geometry,
            mapping,
            batch,
            road,
            draws,
            &TickBlend::identity(race),
        );
    });
}

/// The steady-state per-frame path (items 13/15): pre-owned batch, road
/// mesh, and draw list, plus the tick clock and the last two ticks' bike
/// poses that between-tick interpolation needs. [`Self::advance`] feeds one
/// rendered frame's delta, runs the due 60 Hz ticks, and composes with the
/// clock's leftover alpha — no allocation after warmup, and every rendered
/// frame moves.
pub struct GameSession {
    batch: SpriteBatch,
    road: RoadMesh,
    draws: Vec<DrawItem>,
    clock: sim::clock::TickClock,
    blend: TickBlend,
    /// The player's interpolated draw pose from the last `advance`.
    player_pose: BikePose,
}

impl GameSession {
    pub fn new(tier: render::QualityTier) -> Self {
        Self {
            batch: SpriteBatch::for_tier(tier),
            road: RoadMesh::default(),
            draws: Vec::new(),
            clock: sim::clock::TickClock::new(),
            blend: TickBlend::default(),
            player_pose: [0.0, 0.0],
        }
    }

    /// The composed sprite batch for this frame's draw.
    pub fn batch(&self) -> &SpriteBatch {
        &self.batch
    }

    /// The composed road mesh for this frame's draw.
    pub fn road(&self) -> &RoadMesh {
        &self.road
    }

    /// The interpolation alpha the last `advance` composed with.
    pub fn alpha(&self) -> f64 {
        self.clock.alpha()
    }

    /// The player's `(distance, lateral)` draw pose from the last `advance`
    /// — the interpolated position the player sprite was drawn at.
    pub fn player_draw_pose(&self) -> BikePose {
        self.player_pose
    }

    /// Capacity of the reused draw list (test seam for the no-realloc
    /// guarantee).
    pub fn draw_list_capacity(&self) -> usize {
        self.draws.capacity()
    }

    /// Feeds one rendered frame's delta, runs the due 60 Hz ticks (queueing
    /// `inputs` before the first), and composes the interpolated frame.
    /// The tick rate never changes: `frame_dt_secs` only decides how many
    /// ticks run and how far between the last two this draw sits.
    pub fn advance(
        &mut self,
        race: &mut Race,
        geometry: &RoadGeometry,
        mapping: &AtlasMapping,
        inputs: &[GameInput],
        frame_dt_secs: f64,
    ) {
        for input in inputs {
            race.push_input(*input);
        }
        let ticks = self.clock.update(frame_dt_secs);
        for _ in 0..ticks {
            self.blend.snapshot_before_tick(race);
            race.tick();
        }
        self.blend.curr = capture_poses(race);
        self.blend.alpha = self.clock.alpha() as f32;
        compose(
            race,
            geometry,
            mapping,
            &mut self.batch,
            &mut self.road,
            &mut self.draws,
            &self.blend,
        );
        if let Some(state) = race.bike_states().next() {
            self.player_pose = self.blend.interpolate(0, [state.pos_x, state.pos_y]);
        }
    }
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
    /// How far this item is pulled into the haze: 0 for everything on the
    /// road's fog budget, up to a capped ghost for distant landmarks (7c).
    fog: f32,
}

/// Zone grass band: the alternating base tone pulled toward the zone's
/// grass tint (7c). Pure — the same base and tint always blend the same way.
fn grass_shade_for(base: [f32; 4], tint: [f32; 4]) -> [f32; 4] {
    lerp_color(base, tint, GRASS_TINT_STRENGTH)
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

/// One parallax layer of the distant backdrop (B3, 7c). The ridge profile
/// is a function of *backdrop meters*: each screen column samples a slice
/// of it, a road bend ahead shifts it, and each zone contributes its own
/// phase — so the horizon changes by zone and on straights, which a
/// screen-x sine never does (item 9: never with distance travelled).
struct BackdropLayer {
    base_px: f32,
    amp_px: f32,
    wavelength_px: f32,
    phase: f32,
    /// Backdrop meters per screen column.
    world_per_px: f32,
    /// Backdrop meters of shift per meter of road bend ahead.
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
        world_per_px: 1.0,
        shift_per_m: 0.3,
        haze: 0.55,
    },
    BackdropLayer {
        base_px: 9.0,
        amp_px: 9.0,
        wavelength_px: 150.0,
        phase: 2.1,
        world_per_px: 1.2,
        shift_per_m: 0.5,
        haze: 0.38,
    },
    BackdropLayer {
        base_px: 4.0,
        amp_px: 12.0,
        wavelength_px: 210.0,
        phase: 0.8,
        world_per_px: 1.5,
        shift_per_m: 0.8,
        haze: 0.15,
    },
];

/// Backdrop silhouette column width, in pixels. Narrow enough that a
/// 32 px-wide column never reads as a stepped bar on the ridge line.
const BACKDROP_COLUMN_PX: f32 = 12.0;

/// How much road bend feeds the backdrop shift: the centreline's lateral
/// drift over this window ahead of the camera. A window bounds the shift on
/// straight runs and long curves alike.
const BACKDROP_BEND_AHEAD: f32 = 150.0;

/// How much zone-to-zone backdrop-amplitude spread feeds the ridge phase:
/// phase = (amp - RIDGE_AMP_ORIGIN) * RIDGE_PHASE_PER_AMP, in backdrop
/// meters per unit of amplitude. Carried by the zone sample's blended
/// amplitude so the ridge handover slides across boundary windows.
const RIDGE_PHASE_PER_AMP: f32 = 240.0;
/// The amplitude the phase origin maps to (the coast zone's backdrop_amp).
const RIDGE_AMP_ORIGIN: f32 = 0.7;

/// The zone-anchored ridge phase for a camera distance (item 9): each zone
/// contributes a constant offset to the ridge's horizontal coordinate, so
/// the far profile changes by zone — never slides with distance travelled.
/// The phase is carried by the zone sample's blended `backdrop_amp`, so it
/// hands over smoothly across boundary windows. Pure.
fn ridge_zone_phase(camera_distance: f32) -> f32 {
    let amp = world::ZoneLayout::for_track()
        .sample(camera_distance)
        .backdrop_amp;
    (amp - RIDGE_AMP_ORIGIN) * RIDGE_PHASE_PER_AMP
}

/// Ridge profile value for one layer at backdrop coordinate `u`, in [0, 1]:
/// two incommensurate sines of backdrop meters so the ridge line never
/// visibly repeats as it scrolls. Pure.
fn ridge_value(u: f32, layer: &BackdropLayer) -> f32 {
    let k = std::f32::consts::TAU / (layer.wavelength_px * layer.world_per_px);
    let a = (u * k + layer.phase).sin();
    let b = (u * k * 2.7 + layer.phase * 1.6).sin();
    (0.5 + 0.5 * (0.65 * a + 0.35 * b)).clamp(0.0, 1.0)
}

/// The world-anchored ridge profile for one screen column at a zone phase
/// and bend shift, in [0, 1]. Pure — the zone-anchoring regression holds
/// this (item 9): inside one zone the value never changes with camera
/// distance, and the per-zone phase makes the profile differ between
/// zones. `push_backdrop` renders through this same function, so the
/// tested profile is the shipped one.
fn backdrop_profile(zone_phase: f32, px: f32, shift: f32, layer: &BackdropLayer) -> f32 {
    ridge_value(zone_phase + px * layer.world_per_px + shift, layer)
}

/// The backdrop's bend shift, in backdrop meters (2b): how a road bend
/// ahead swings the ridge line. The sign convention makes `RoadGeometry`
/// authoritative — negative curvature is a left bend, center_offset falls —
/// and the swing must act *opposite* to the sign of center_offset's change:
/// in a left bend (negative `bend`) the shift goes negative, which slides
/// the ridge content right on screen, the way turning left swings the
/// scenery rightward through the view. Pure.
fn backdrop_bend_shift(bend: f32, layer: &BackdropLayer) -> f32 {
    bend * layer.shift_per_m
}

/// Paints the layered hill silhouettes between the sky and the horizon
/// (B3, 7c): one flat-color column quad per [`BACKDROP_COLUMN_PX`] pixels of
/// screen, their tops traced by the zone-anchored [`backdrop_profile`]
/// (item 9), shifted by the road's bend ahead, and pulled toward the
/// zone's own sky tint — also blended across boundaries.
fn push_backdrop(
    geometry: &RoadGeometry,
    camera: &world::Camera,
    road: &mut RoadMesh,
    horizon: f32,
    zone: &world::ZoneSample,
) {
    let bend = geometry.center_offset(camera.distance + BACKDROP_BEND_AHEAD)
        - geometry.center_offset(camera.distance);
    // The zone's own ridge phase and colour (item 9): both ride values the
    // zone sample blends smoothly across boundary windows, while staying
    // recognisably the same grey ridge and pale band.
    let zone_phase = ridge_zone_phase(camera.distance);
    let zone_ridge = lerp_color(BACKDROP, zone.sky_tint, BACKDROP_ZONE_TINT);
    for layer in &BACKDROP_LAYERS {
        // The bend shift moves in backdrop meters now, on the same
        // world-anchored coordinate the camera scrolls.
        let bend_shift = backdrop_bend_shift(bend, layer);
        let color = lerp_color(zone_ridge, HAZE, layer.haze);
        let mut x = 0.0;
        while x < VIEWPORT_WIDTH {
            let x1 = (x + BACKDROP_COLUMN_PX).min(VIEWPORT_WIDTH);
            let height = backdrop_profile(zone_phase, x, bend_shift, layer);
            let top = horizon - layer.base_px - layer.amp_px * zone.backdrop_amp * height;
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

/// The camera over the player: distance trailing the bike, leaning with
/// the player's heading first and the road bend second — the view never
/// leans against the rider — widening with speed, and carrying the tick's
/// shake offsets. Pure: the shake terms arrive precomputed so the caller
/// keeps the one deterministic RNG.
fn player_camera(
    race: &Race,
    geometry: &RoadGeometry,
    shake_lateral: f32,
    shake_height: f32,
) -> world::Camera {
    let player_speed = race.player_speed();
    let player_distance = race.player_distance();
    let player_curvature = geometry.curvature_at(player_distance);
    let player_heading = race.player_heading();
    let player_lateral = race.player_lateral();
    // Lean with the player's steering and the road bend (2b), then clamp —
    // but only on the tarmac. Off it, the camera follows the bike's
    // lateral outright (3b): clamped at ±1.5 m, the sprite rode off the
    // screen for the whole OFF ROAD stretch.
    let lean =
        player_heading * CAMERA_HEADING_LEAN + camera_bend_lean(player_curvature, player_speed);
    let lateral = if player_lateral.abs() > geometry.road_width() * 0.5 {
        player_lateral + lean
    } else {
        lean.clamp(-CAMERA_LEAN_MAX, CAMERA_LEAN_MAX)
    };
    world::Camera {
        distance: player_distance - CAMERA_TRAIL,
        height: CAMERA_HEIGHT + shake_height,
        lateral: lateral + shake_lateral,
        focal: FOCAL * (1.0 - SPEED_WIDENING * (player_speed / game::bike::MAX_SPEED)),
        horizon_y_px: VIEWPORT_HEIGHT * HORIZON_FRAC,
        viewport: Viewport {
            width: VIEWPORT_WIDTH,
            height: VIEWPORT_HEIGHT,
        },
    }
}

/// Bike poses at the last two ticks, plus the leftover fraction between
/// them (item 13): what between-tick interpolation draws with. `identity`
/// pins the draw to the latest tick — the single-tick paths' exactness.
#[derive(Debug, Clone, Default)]
struct TickBlend {
    /// `(distance, lateral)` per bike slot at the tick before last.
    prev: Vec<[f32; 2]>,
    /// `(distance, lateral)` per bike slot at the latest tick.
    curr: Vec<[f32; 2]>,
    /// The clock's leftover fraction between the two ticks, in [0, 1).
    alpha: f32,
}

/// One bike's pose at a tick: course distance and lateral offset.
type BikePose = [f32; 2];

/// Captures every bike's pose from the live race, in slot order.
fn capture_poses(race: &Race) -> Vec<BikePose> {
    race.bike_states().map(|s| [s.pos_x, s.pos_y]).collect()
}

impl TickBlend {
    /// Draw exactly the current tick: no interpolation. The one-tick paths
    /// (`GameFrame::build`, `advance_game_frame`) compose the state they
    /// just ticked to.
    fn identity(race: &Race) -> Self {
        let curr = capture_poses(race);
        Self {
            prev: curr.clone(),
            curr,
            alpha: 0.0,
        }
    }

    /// Records the pre-tick state; call immediately before `race.tick()`.
    fn snapshot_before_tick(&mut self, race: &Race) {
        self.prev = capture_poses(race);
    }

    /// The pose to draw for bike `slot` now: the last two ticks' poses
    /// blended by the clock's leftover fraction. Falls back to the passed
    /// current pose when the slot has no history (first frame, new rider).
    fn interpolate(&self, slot: usize, curr: BikePose) -> BikePose {
        let Some(prev) = self.prev.get(slot) else {
            return curr;
        };
        let a = self.alpha;
        [
            prev[0] + (curr[0] - prev[0]) * a,
            prev[1] + (curr[1] - prev[1]) * a,
        ]
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
    draws: &mut Vec<DrawItem>,
    blend: &TickBlend,
) {
    let player_control = race.player_control();

    // Camera feel: lean with the player's steering and the road bend (in
    // that order — the view never leans against the rider), widen the view
    // with speed, and shake while the rider slides.
    // The shake jitter is deterministic per tick — no clock, no host RNG.
    let mut jitter = sim::rng::XorShift64::new(race.tick_index() | 1);
    let shake = if player_control == Control::Crashed {
        CRASH_SHAKE * (race.player_crash_timer() / game::bike::CRASH_DURATION_SECS)
    } else if race.player_on_rumble() {
        RUMBLE_SHAKE
    } else {
        0.0
    };
    let shake_lateral = (jitter.next_f32_in(-1.0, 1.0)) * shake;
    let shake_height = (jitter.next_f32_in(-1.0, 1.0)) * shake * 0.6;
    let camera = player_camera(race, geometry, shake_lateral, shake_height);
    let projector = world::Projector::new(geometry, camera);
    let half_width = geometry.road_width() * 0.5;

    // Sky above the horizon, the parallax backdrop, then the banded grass
    // and the road over it.
    let horizon = projector.horizon_y();
    road.vertices.clear();

    batch.clear();
    // Zone sample at the camera (7c): one sample drives the sky tint and
    // backdrop amplitude; per-strip samples drive the grass.
    let zones = world::ZoneLayout::for_track();
    let zone = zones.sample(camera.distance);
    // Sky: one quad shading continuously from the zenith blue down to the
    // horizon haze — vertex colors interpolate, so there are no band edges
    // anywhere in the sky (B1, HD direction). The haze blend the old haze
    // band carried is the gradient's horizon stop. The zone's tint pulls
    // both stops, so the whole sky shifts with the place you ride through.
    road.push_screen_rect_gradient(
        0.0,
        0.0,
        VIEWPORT_WIDTH,
        horizon,
        lerp_color(sky_color_at(0.0), zone.sky_tint, SKY_TINT_STRENGTH),
        lerp_color(sky_color_at(1.0), zone.sky_tint, SKY_TINT_STRENGTH),
    );
    push_backdrop(geometry, &camera, road, horizon, &zone);

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

        // Grass band for this strip's depth range, full width (B4),
        // shading from the near edge's fog level to the far edge's — the
        // depth fade becomes a continuous gradient per band instead of one
        // flat shade (HD direction). The tint is the zone at the strip's
        // mid-point in world space, so bands blend across zone boundaries.
        let light_band = strip_index.is_multiple_of(2);
        let zone_mid = zones.sample(camera.distance + (strip_index as f32 + 0.5) * ROAD_STEP);
        let fog_far = fog_at(depth);
        let band = if light_band { GRASS_LIGHT } else { GRASS_DARK };
        road.push_screen_rect_gradient(
            0.0,
            strip.far_y,
            VIEWPORT_WIDTH,
            strip.near_y,
            lerp_color(
                grass_shade_for(band, zone_mid.grass_tint),
                HAZE,
                fog_far * GRASS_FOG,
            ),
            lerp_color(
                grass_shade_for(band, zone_mid.grass_tint),
                HAZE,
                fog_at(depth - ROAD_STEP) * GRASS_FOG,
            ),
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
    draws.clear();
    let shadow_uv = mapping.shadow_uv();
    for (slot, state) in race.bike_states().enumerate() {
        // Between-tick interpolation (item 13): the draw pose blends the
        // last two ticks' poses by the clock's leftover fraction, so a
        // rendered frame between ticks still moves.
        let [draw_distance, draw_lateral] = blend.interpolate(slot, [state.pos_x, state.pos_y]);
        let control = if slot == 0 {
            player_control
        } else {
            race.bike_control(slot)
        };
        // The player's rear view carries the control-state frames; rivals
        // ride their own livery per pack slot, down to the shared crash
        // pose (C3), with a slight per-slot size spread so the pack is a
        // line of distinct riders, not clones.
        let rival_scale = if slot == 0 {
            1.0
        } else {
            RIVAL_SCALES[(slot - 1) % RIVAL_SCALES.len()]
        };
        // Swing first: a mid-punch rider shows the punch frame even while
        // braking; the crash pose still wins over a downed rider's swing.
        let uv = if control == Control::Crashed {
            mapping.frame_uv(Control::Crashed)
        } else if let Some(attack) = race.bike_attack_pose(slot) {
            mapping.swing_uv(attack, slot)
        } else if slot == 0 {
            mapping.frame_uv(control)
        } else {
            mapping.rival_uv(slot)
        };
        draws.push(DrawItem {
            distance: draw_distance + SHADOW_DEPTH_BIAS,
            lateral: draw_lateral,
            world_width: BIKE_SHADOW.0 * rival_scale,
            world_height: BIKE_SHADOW.1 * rival_scale,
            uv: shadow_uv,
            roll: 0.0,
            fog: 0.0,
        });
        draws.push(DrawItem {
            distance: draw_distance,
            lateral: draw_lateral,
            world_width: BIKE_WORLD_WIDTH * rival_scale,
            world_height: BIKE_WORLD_HEIGHT * rival_scale,
            uv,
            // The rider's bank: the sim carries the lean, the instance
            // transform rolls the quad about its ground contact.
            roll: race.bike_lean(slot),
            fog: 0.0,
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
            fog: 0.0,
        });
        draws.push(DrawItem {
            distance: car.state.pos_x,
            lateral: car.state.pos_y,
            world_width: 2.0,
            world_height: 1.5,
            uv: mapping.car_uv(),
            roll: 0.0,
            fog: 0.0,
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
            fog: 0.0,
        });
        draws.push(DrawItem {
            distance: hazard.pos_x,
            lateral: hazard.lateral,
            world_width: CONE_SIZE.0,
            world_height: CONE_SIZE.1,
            uv: mapping.cone_uv(),
            roll: 0.0,
            fog: 0.0,
        });
    }
    push_scenery(geometry, mapping, draws, camera.distance);
    push_landmarks(geometry, mapping, draws, camera.distance);
    push_distance_markers(
        mapping,
        draws,
        camera.distance + 2.0,
        camera.distance + ROAD_DRAW_DISTANCE,
    );
    push_poles(geometry, mapping, draws, camera.distance);
    draws.sort_by(|a, b| b.distance.total_cmp(&a.distance));
    for item in draws {
        let style = SpriteStyle {
            uv_rect: item.uv,
            // Fog pulls distant sprites into the haze; landmarks cap it so
            // their shape ghosts in before they arrive.
            tint: lerp_color([1.0, 1.0, 1.0, 1.0], HAZE, item.fog),
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

/// Large one-off landmarks per lap (7c): the "you are here" anchors. Each
/// lap's instances are placed at fixed world distances; the closest not yet
/// passed is drawn from [`LANDMARK_DRAW_DISTANCE`] out, fog-capped so the
/// shape ghosts in through the haze long before the rider reaches it. Pure
/// — same seed, same landmarks.
fn push_landmarks(
    _geometry: &RoadGeometry,
    mapping: &AtlasMapping,
    draws: &mut Vec<DrawItem>,
    camera_distance: f32,
) {
    let shadow_uv = mapping.shadow_uv();
    let zones = world::ZoneLayout::for_track();
    let lap_length = zones.lap_length;
    for landmark in world::course_landmarks() {
        // All instances of this landmark: every lap it belongs to that is
        // still within the draw window ahead.
        for lap in 0..2 {
            let distance = landmark.distance + lap as f32 * lap_length;
            let depth = distance - camera_distance;
            if depth <= world::NEAR_PLANE + 1.0 || depth > LANDMARK_DRAW_DISTANCE {
                continue;
            }
            let (uv, size) = match landmark.kind {
                world::LandmarkKind::WaterTower => (mapping.water_tower_uv(), WATER_TOWER_SIZE),
                world::LandmarkKind::Windmill => (mapping.windmill_uv(), WINDMILL_SIZE),
                world::LandmarkKind::Billboards => (mapping.billboard_uv(), BILLBOARD_SIZE),
            };
            let fog = fog_at(depth).min(LANDMARK_FOG_MAX);
            draws.push(DrawItem {
                distance: distance + SHADOW_DEPTH_BIAS,
                lateral: landmark.lateral,
                world_width: LANDMARK_SHADOW.0,
                world_height: LANDMARK_SHADOW.1,
                uv: shadow_uv,
                roll: 0.0,
                fog,
            });
            draws.push(DrawItem {
                distance,
                lateral: landmark.lateral,
                world_width: size.0,
                world_height: size.1,
                uv,
                roll: 0.0,
                fog,
            });
        }
    }
}

/// Distance-marker posts every [`MARKER_STEP`] meters on both road edges
/// (7c): the cheap "how far along am I" rhythm. World-anchored grid — the
/// posts sit still in the world as the rider passes.
fn push_distance_markers(
    mapping: &AtlasMapping,
    draws: &mut Vec<DrawItem>,
    window_near: f32,
    window_far: f32,
) {
    let shadow_uv = mapping.shadow_uv();
    let first = ((window_near + 2.0) / MARKER_STEP).floor() as u64;
    let last = (window_far / MARKER_STEP).ceil() as u64;
    for slot in first..=last {
        let distance = slot as f32 * MARKER_STEP;
        for side in [-1.0_f32, 1.0] {
            let lateral = side * MARKER_LATERAL;
            draws.push(DrawItem {
                distance: distance + SHADOW_DEPTH_BIAS,
                lateral,
                world_width: MARKER_SIZE.0,
                world_height: MARKER_SIZE.1,
                uv: shadow_uv,
                roll: 0.0,
                fog: 0.0,
            });
            draws.push(DrawItem {
                distance,
                lateral,
                world_width: MARKER_SIZE.0,
                world_height: MARKER_SIZE.1,
                uv: mapping.marker_uv(),
                roll: 0.0,
                fog: 0.0,
            });
        }
    }
}

/// Power poles in the zones that ask for them (7c): repeating quads on a
/// fixed world grid, alternating sides. Pure — same distance, same poles.
fn push_poles(
    geometry: &RoadGeometry,
    mapping: &AtlasMapping,
    draws: &mut Vec<DrawItem>,
    camera_distance: f32,
) {
    let half_width = geometry.road_width() * 0.5;
    let zones = world::ZoneLayout::for_track();
    let shadow_uv = mapping.shadow_uv();
    let first = ((camera_distance + 2.0) / POLE_SPACING).floor() as u64;
    let last = ((camera_distance + ROAD_DRAW_DISTANCE) / POLE_SPACING).ceil() as u64;
    for slot in first..=last {
        let distance = slot as f32 * POLE_SPACING;
        if !zones.sample(distance).poles {
            continue;
        }
        let side = if slot % 2 == 0 { 1.0 } else { -1.0 };
        let lateral = side * (half_width + 2.2);
        draws.push(DrawItem {
            distance: distance + SHADOW_DEPTH_BIAS,
            lateral,
            world_width: POLE_SHADOW.0,
            world_height: POLE_SHADOW.1,
            uv: shadow_uv,
            roll: 0.0,
            fog: 0.0,
        });
        draws.push(DrawItem {
            distance,
            lateral,
            world_width: POLE_SIZE.0,
            world_height: POLE_SIZE.1,
            uv: mapping.pole_uv(),
            roll: 0.0,
            fog: 0.0,
        });
    }
}

/// Deterministic roadside scenery: every [`SCENERY_SPACING`] meters there is
/// a slot whose seeded roll picks a tree, bush, or sign. Sides alternate by
/// slot — guaranteed balance across the road, however the seeded rolls fall
/// — with an occasional opposite-side understory bush (D4). Same distance,
/// same dressing — no per-frame state. The zone at the slot's distance
/// supplies the dressing mix (7c): the coast grows palms, the town grows
/// buildings, and the ridge scatters boulders, while dense zones line the
/// verge with fence runs.
fn push_scenery(
    geometry: &RoadGeometry,
    mapping: &AtlasMapping,
    draws: &mut Vec<DrawItem>,
    camera_distance: f32,
) {
    let half_width = geometry.road_width() * 0.5;
    let shadow_uv = mapping.shadow_uv();
    let zones = world::ZoneLayout::for_track();
    let first = ((camera_distance + 2.0) / SCENERY_SPACING).floor() as u64;
    let last = ((camera_distance + ROAD_DRAW_DISTANCE) / SCENERY_SPACING).ceil() as u64;
    for slot in first..=last {
        let distance = slot as f32 * SCENERY_SPACING;
        let zone = zones.sample(distance);
        let mut roll = sim::rng::XorShift64::new(slot.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let kind = world::pick_dressing(zone.dressing, roll.next_f32_in(0.0, 1.0));
        let (uv, (width, height), shadow) = match kind {
            world::DressingKind::Palm => (mapping.palm_uv(), PALM_SIZE, TREE_SHADOW),
            world::DressingKind::Tree => {
                // Trees come in three silhouettes (pine, broadleaf, tall
                // pine), cycled by slot, so the verge is a treeline, not a
                // stamp.
                match slot % 3 {
                    0 => (mapping.tree_variant_uv(0), TREE_SIZE, TREE_SHADOW),
                    1 => (mapping.tree_variant_uv(1), BROADLEAF_SIZE, TREE_SHADOW),
                    _ => (mapping.tree_variant_uv(2), TALL_PINE_SIZE, (4.0, 1.2)),
                }
            }
            world::DressingKind::Building => {
                (mapping.building_uv(), BUILDING_SIZE, BUILDING_SHADOW)
            }
            world::DressingKind::Rock => (mapping.rock_uv(), ROCK_SIZE, ROCK_SHADOW),
            world::DressingKind::Bush => (mapping.bush_uv(), BUSH_SIZE, BUSH_SHADOW),
            world::DressingKind::Sign => (mapping.sign_uv(), SIGN_SIZE, SIGN_SHADOW),
            world::DressingKind::Skip => continue,
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
            fog: 0.0,
        });
        draws.push(DrawItem {
            distance,
            lateral,
            world_width: width,
            world_height: height,
            uv,
            roll: 0.0,
            fog: 0.0,
        });
        // Dense zones line both verges with fence sections: cheap
        // repeating quads on the same slot grid (7c).
        if zone.fences {
            for fence_side in [-1.0_f32, 1.0] {
                let fence_lateral = fence_side * (half_width + 0.9);
                draws.push(DrawItem {
                    distance: distance + SHADOW_DEPTH_BIAS,
                    lateral: fence_lateral,
                    world_width: FENCE_SHADOW.0,
                    world_height: FENCE_SHADOW.1,
                    uv: shadow_uv,
                    roll: 0.0,
                    fog: 0.0,
                });
                draws.push(DrawItem {
                    distance,
                    lateral: fence_lateral,
                    world_width: FENCE_SIZE.0,
                    world_height: FENCE_SIZE.1,
                    uv: mapping.fence_uv(),
                    roll: 0.0,
                    fog: 0.0,
                });
            }
        }
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
                fog: 0.0,
            });
            draws.push(DrawItem {
                distance,
                lateral: opposite,
                world_width: BUSH_SIZE.0,
                world_height: BUSH_SIZE.1,
                uv: mapping.bush_uv(),
                roll: 0.0,
                fog: 0.0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack_placeholder_atlas;
    use world::ZoneLayout;

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
    fn camera_lean_follows_player_heading_change() {
        let (mut race, geometry) = demo_race();
        while race.is_counting_down() {
            race.tick();
        }
        // Roll up to speed on the opening straight, before the first bend:
        // same distance, same speed in both runs, so the only difference is
        // the player's held steering.
        for _ in 0..30 {
            race.push_input(GameInput::Accelerate);
            race.tick();
        }
        let neutral = player_camera(&race, &geometry, 0.0, 0.0);
        for _ in 0..6 {
            race.push_input(GameInput::SteerRight);
            race.tick();
        }
        let steering = player_camera(&race, &geometry, 0.0, 0.0);
        assert!(
            steering.lateral > neutral.lateral,
            "camera leans with the player's right-hand steer, not against it"
        );
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
        let mut draws = Vec::new();
        let blend = TickBlend::identity(&race);
        compose(
            &race, &geometry, &mapping, &mut fresh, &mut road, &mut draws, &blend,
        );
        let reused_positions: Vec<[f32; 2]> =
            batch.instances().iter().map(|i| i.position).collect();
        let fresh_positions: Vec<[f32; 2]> = fresh.instances().iter().map(|i| i.position).collect();
        assert_eq!(reused_positions, fresh_positions, "no stale sprites");
    }

    /// Groups the road mesh's vertex stream back into the quads that pushed
    /// it: every four vertices are one quad (near-left, near-right, far-left,
    /// far-right).
    fn road_quads(road: &RoadMesh) -> impl Iterator<Item = &[RoadVertex]> {
        road.vertices.as_chunks::<4>().0.iter().map(|q| &q[..])
    }

    /// A full-width quad: a screen-space band (sky, grass, the horizon
    /// sliver) rather than a road trapezoid.
    fn is_full_width_band(quad: &[RoadVertex]) -> bool {
        let left_ok = quad[0].position[0] == 0.0 && quad[2].position[0] == 0.0;
        let right_ok =
            quad[1].position[0] == VIEWPORT_WIDTH && quad[3].position[0] == VIEWPORT_WIDTH;
        left_ok && right_ok
    }

    #[test]
    fn the_sky_is_one_smooth_gradient_not_stacked_bands() {
        let mapping = mapping();
        let (race, geometry) = demo_race();
        let frame = GameFrame::build(&race, &geometry, &mapping);

        // The sky is the band whose far (top) edge is the screen top.
        let sky: Vec<&[RoadVertex]> = road_quads(&frame.road)
            .filter(|q| is_full_width_band(q) && q[2].position[1] == 0.0)
            .collect();
        assert_eq!(sky.len(), 1, "the sky composes as one quad, not bands");
        let quad = sky[0];
        assert!(
            quad[0].position[1] >= VIEWPORT_HEIGHT * HORIZON_FRAC,
            "the gradient reaches the horizon (down to {}), not just the top third",
            quad[0].position[1]
        );
        assert_ne!(
            quad[0].color, quad[2].color,
            "the sky shades from zenith to horizon haze — not one flat fill"
        );
    }

    #[test]
    fn the_sky_gradient_is_continuous_from_zenith_to_haze() {
        // Adjacent samples 1% of the sky apart may not jump: a step over
        // ~8 levels of 255 reads as a band edge, which is the pixel-art
        // look the HD direction retires.
        for step in 0..100 {
            let a = sky_color_at(step as f32 * 0.01);
            let b = sky_color_at((step as f32 + 1.0) * 0.01);
            for c in 0..3 {
                let delta = ((a[c] - b[c]).abs() * 255.0).round();
                assert!(
                    delta <= 8.0,
                    "sky at {step}% jumps {delta} levels per 1% of height — banded, not smooth"
                );
            }
        }
        assert_ne!(
            sky_color_at(0.0),
            sky_color_at(1.0),
            "the gradient actually spans zenith to haze"
        );
    }

    #[test]
    fn near_ground_bands_shade_as_gradients() {
        let mapping = mapping();
        let (race, geometry) = banked_left_race();
        let frame = GameFrame::build(&race, &geometry, &mapping);

        // Close ground = the full-width bands at the bottom of the screen.
        let near_ground: Vec<&[RoadVertex]> = road_quads(&frame.road)
            .filter(|q| is_full_width_band(q) && q[0].position[1] > 400.0)
            .collect();
        assert!(!near_ground.is_empty(), "ground reaches the screen bottom");
        assert!(
            near_ground.iter().any(|q| q[0].color != q[2].color),
            "near ground bands shade as vertical gradients, not flat fills"
        );
    }

    #[test]
    fn band_contrast_is_softened_not_removed() {
        // The light/dark alternation is the off-road motion cue (B4/B5) and
        // must survive; the HD direction only softens how hard the bands
        // snap. Deltas are per-channel, in 0..=1 color space.
        let max_delta = |a: [f32; 4], b: [f32; 4]| {
            a.iter()
                .zip(b.iter())
                .map(|(x, y)| (x - y).abs())
                .fold(0.0_f32, f32::max)
        };
        let grass = max_delta(GRASS_LIGHT, GRASS_DARK);
        assert!(
            grass > 0.0,
            "the grass alternation must survive — it is the off-road motion cue"
        );
        assert!(
            grass <= 0.06,
            "grass band contrast {grass} is harsh; soften toward 0.06"
        );
        let road = max_delta(ROAD_LIGHT, ROAD_DARK);
        assert!(
            road > 0.0,
            "the asphalt alternation must survive — it reads as forward motion"
        );
        assert!(
            road <= 0.08,
            "road band contrast {road} is harsh; soften toward 0.08"
        );
    }

    // -----------------------------------------------------------------------
    // Wave 7c — world construction: distance-anchored horizon, zones,
    // landmarks. Red-first per the spec's verification list.
    // -----------------------------------------------------------------------

    #[test]
    fn zone_tints_reach_the_composed_frame() {
        let mapping = mapping();
        // The zone table gives different zones different tints to deliver.
        assert_ne!(
            ZoneLayout::for_track().sample(200.0).sky_tint,
            ZoneLayout::for_track().sample(800.0).sky_tint,
        );
        // The composed sky carries the zone tint: a frame composed at the
        // start line (coast) differs from one composed in the town. The
        // ride out is deterministic — same inputs, same sim.
        let (mut race, geometry) = demo_race();
        while race.is_counting_down() {
            race.tick();
        }
        let early = GameFrame::build(&race, &geometry, &mapping);
        let mut in_town = false;
        for _ in 0..4000 {
            race.push_input(GameInput::Accelerate);
            race.tick();
            if race.player_distance() > 700.0 {
                in_town = true;
                break;
            }
        }
        assert!(in_town, "the race reaches the town zone");
        let late = GameFrame::build(&race, &geometry, &mapping);
        let early_sky = road_quads(&early.road)
            .filter(|q| is_full_width_band(q) && q[2].position[1] == 0.0)
            .map(|q| q[2].color)
            .next()
            .expect("the sky composes as one quad");
        let late_sky = road_quads(&late.road)
            .filter(|q| is_full_width_band(q) && q[2].position[1] == 0.0)
            .map(|q| q[2].color)
            .next()
            .expect("the sky composes as one quad");
        assert_ne!(
            early_sky, late_sky,
            "the zone's sky tint must reach the composed frame"
        );
    }

    #[test]
    fn grass_shade_follows_the_zone_tint() {
        // The per-strip grass shade must carry the zone's tint: the same
        // alternating base bands read differently in coast and forest.
        let coast = ZoneLayout::for_track().sample(200.0).grass_tint;
        let forest = ZoneLayout::for_track().sample(1300.0).grass_tint;
        assert_ne!(
            grass_shade_for(GRASS_LIGHT, coast),
            grass_shade_for(GRASS_LIGHT, forest),
            "the zone tint must actually reach the grass shade"
        );
    }

    #[test]
    fn landmarks_and_markers_ride_the_sprite_pipeline() {
        let geometry = track_geometry();
        let mapping = mapping();

        // Landmarks: at the start line the water tower (380 m) is already on
        // the draw list — visible well before you reach it — while nothing
        // beyond the visibility range is.
        let mut draws = Vec::new();
        push_landmarks(&geometry, &mapping, &mut draws, 0.0);
        assert!(
            draws
                .iter()
                .any(|d| d.uv == mapping.water_tower_uv() && d.world_width > 8.0),
            "the water tower is pushed as a large billboard sprite"
        );
        let mut from_tower = Vec::new();
        push_landmarks(&geometry, &mapping, &mut from_tower, 380.0);
        // From the tower's own distance the windmill (940 m) is listed; the
        // tower itself is behind the camera and gone.
        assert!(from_tower.iter().any(|d| d.uv == mapping.windmill_uv()));
        assert!(!from_tower.iter().any(|d| d.uv == mapping.water_tower_uv()));

        // Distance markers: every 500 m, both sides — pure function of the
        // camera's distance window.
        let mut marks = Vec::new();
        push_distance_markers(&mapping, &mut marks, 100.0, 900.0);
        let mark_distances: Vec<f32> = marks.iter().map(|d| d.distance).collect();
        assert!(
            mark_distances.iter().any(|d| (*d - 500.0).abs() < 1.0),
            "a marker post sits at 500 m: {mark_distances:?}"
        );
        // Same window, same markers — no per-frame state.
        let mut marks_again = Vec::new();
        push_distance_markers(&mapping, &mut marks_again, 100.0, 900.0);
        assert_eq!(
            marks.iter().map(|d| d.distance).collect::<Vec<f32>>(),
            marks_again.iter().map(|d| d.distance).collect::<Vec<f32>>(),
        );
    }

    #[test]
    fn scenery_mix_follows_the_zone_of_its_distance() {
        let geometry = track_geometry();
        let mapping = mapping();
        // The coast grows palms; the town grows buildings. Same slot grid,
        // different mixes — mile 800 must not look like mile 200.
        let mut coast = Vec::new();
        push_scenery(&geometry, &mapping, &mut coast, 200.0);
        let mut town = Vec::new();
        push_scenery(&geometry, &mapping, &mut town, 800.0);
        assert!(
            coast.iter().any(|d| d.uv == mapping.palm_uv()),
            "palms on the coast"
        );
        assert!(
            town.iter().any(|d| d.uv == mapping.building_uv()),
            "buildings in the town"
        );
        // The mix actually switched: the coast window is palm-dominated,
        // the town window (which still reaches back into the coast's
        // dressing range) is not.
        let coast_palms = coast.iter().filter(|d| d.uv == mapping.palm_uv()).count();
        let town_palms = town.iter().filter(|d| d.uv == mapping.palm_uv()).count();
        assert!(
            coast_palms > town_palms,
            "the coast grows more palms than the town: {coast_palms} vs {town_palms}"
        );
        // Same window twice, same dressing — the determinism pattern.
        let mut town_again = Vec::new();
        push_scenery(&geometry, &mapping, &mut town_again, 800.0);
        assert_eq!(
            town.len(),
            town_again.len(),
            "dressing is a pure function of distance"
        );
    }

    #[test]
    fn two_composes_of_one_state_build_identical_frames() {
        // The world construction adds zone sampling to compose(); it must
        // stay stateless: composing the same race twice gives the same
        // batch, instance for instance.
        let mapping = mapping();
        let (mut race, geometry) = demo_race();
        while race.is_counting_down() {
            race.tick();
        }
        for _ in 0..30 {
            race.push_input(GameInput::Accelerate);
            race.tick();
        }
        let mut a = SpriteBatch::for_tier(render::QualityTier::WebGpuCapable);
        let mut b = SpriteBatch::for_tier(render::QualityTier::WebGpuCapable);
        let mut road_a = RoadMesh::default();
        let mut road_b = RoadMesh::default();
        let mut draws = Vec::new();
        let blend = TickBlend::identity(&race);
        compose(
            &race,
            &geometry,
            &mapping,
            &mut a,
            &mut road_a,
            &mut draws,
            &blend,
        );
        compose(
            &race,
            &geometry,
            &mapping,
            &mut b,
            &mut road_b,
            &mut draws,
            &blend,
        );
        assert_eq!(a.instances(), b.instances());
        assert_eq!(road_a.vertices, road_b.vertices);
    }

    // -------------------------------------------------------------------
    // Wave 7 audit — rendering and camera (items 2b/3b/9/10/13/15).
    // Sign convention: RoadGeometry is authoritative — negative curvature
    // is a left bend (center_offset falls); effects of bend act opposite
    // to the sign of center_offset's change.
    // -------------------------------------------------------------------

    /// A race on a synthetic constant-curvature track, released from the
    /// grid and rolled up to speed with held throttle. Mirrored bends come
    /// from flipping the curvature sign; every other input is identical.
    fn bend_race(curvature: f32) -> (Race, RoadGeometry) {
        let geometry = RoadGeometry::new(
            vec![world::CurveSegment {
                start: 0.0,
                length: 2_000.0,
                curvature,
            }],
            Vec::new(),
            12.0,
        );
        let mut race = Race::new(RaceConfig {
            rivals: 1,
            track_length: LAP_LENGTH,
            total_laps: 1,
            seed: 0x5EED_C0DE,
            geometry: geometry.clone(),
        });
        while race.is_counting_down() {
            race.tick();
        }
        for _ in 0..40 {
            race.push_input(GameInput::Accelerate);
            race.tick();
        }
        (race, geometry)
    }

    #[test]
    fn the_camera_bend_lean_acts_opposite_to_the_center_offset_change() {
        // Left bend: curvature < 0, center_offset falls (the convention's
        // "-15 m at 300 m"). The camera must pan INTO the corner — negative
        // lateral, which slides the world right on screen, opposite to the
        // center_offset change — not slide left with the drift (the old
        // sign doubled the drift).
        assert!(
            camera_bend_lean(-0.002, 30.0) < 0.0,
            "left bend: camera pans left (world slides right)"
        );
        assert!(
            camera_bend_lean(0.002, 30.0) > 0.0,
            "right bend: camera pans right (world slides left)"
        );

        // End to end through the live camera: mirrored bends must mirror
        // the pan, on the convention's side each time.
        let (left_race, left_geo) = bend_race(-0.003);
        let (right_race, right_geo) = bend_race(0.003);
        assert!(
            left_geo.center_offset(300.0) < 0.0 && right_geo.center_offset(300.0) > 0.0,
            "the synthetic tracks really bend left/right"
        );
        let left = player_camera(&left_race, &left_geo, 0.0, 0.0);
        let right = player_camera(&right_race, &right_geo, 0.0, 0.0);
        assert!(
            left.lateral < 0.0,
            "left bend: camera pans left so the world slides right (got {})",
            left.lateral
        );
        assert!(
            right.lateral > 0.0,
            "right bend: camera pans right (got {})",
            right.lateral
        );
        assert!(
            left.lateral.abs() <= CAMERA_LEAN_MAX && right.lateral.abs() <= CAMERA_LEAN_MAX,
            "the bend lean stays clamped"
        );
    }

    #[test]
    fn the_backdrop_bend_shift_moves_the_ridge_opposite_to_the_center_offset_change() {
        // A left bend ahead: center_offset over the backdrop window falls
        // (negative bend). The ridge must slide RIGHT on screen — opposite
        // to the center_offset change — so the profile's tallest column
        // moves to a larger px, the way turning left swings the scenery
        // rightward through the view.
        let layer = &BACKDROP_LAYERS[0];
        let bend = -60.0; // meters of center_offset change: a hard left
        let shift = backdrop_bend_shift(bend, layer);
        assert!(
            shift < 0.0,
            "left bend: negative shift slides ridge content right (got {shift})"
        );
        let peak = |shift: f32| {
            let mut best = (f32::MIN, 0.0_f32);
            let mut x = 0.0;
            while x < VIEWPORT_WIDTH {
                let v = backdrop_profile(0.0, x, shift, layer);
                if v > best.0 {
                    best = (v, x);
                }
                x += BACKDROP_COLUMN_PX;
            }
            best.1
        };
        assert!(
            peak(shift) > peak(0.0),
            "the ridge peak must move right in a left bend: {} -> {}",
            peak(0.0),
            peak(shift)
        );
    }

    #[test]
    fn the_camera_follows_the_bike_when_it_leaves_the_tarmac() {
        let (mut race, geometry) = demo_race();
        while race.is_counting_down() {
            race.tick();
        }
        let half_width = geometry.road_width() * 0.5;
        // Off the tarmac, right verge: the camera must track the bike's
        // lateral so the sprite stays on screen (the OFF ROAD sprite
        // disappearing was the hosted-run defect).
        race.place_bike(0, 300.0, half_width + 3.0);
        let off = player_camera(&race, &geometry, 0.0, 0.0);
        assert!(
            (off.lateral - race.player_lateral()).abs() < 0.5,
            "off tarmac the camera must follow the bike's lateral (bike {}, camera {})",
            race.player_lateral(),
            off.lateral
        );
        // On the tarmac the soft-lean clamp still applies: the camera stays
        // near the centreline even when the bike hugs the edge.
        race.place_bike(0, 300.0, half_width - 1.0);
        let on = player_camera(&race, &geometry, 0.0, 0.0);
        assert!(
            on.lateral.abs() <= CAMERA_LEAN_MAX + 1e-3,
            "on tarmac the lean stays clamped (got {})",
            on.lateral
        );
    }

    #[test]
    fn the_far_ridge_profile_is_zone_anchored_not_distance_scrolled() {
        let layer = &BACKDROP_LAYERS[2];
        let columns = || (0..VIEWPORT_WIDTH as u32).step_by(50);
        // Two camera distances inside the coast, past its blend window, on
        // a straight (zero bend shift): identical profile, column for
        // column — the ridge does not slide with distance travelled.
        for px in columns() {
            assert_eq!(
                backdrop_profile(ridge_zone_phase(100.0), px as f32, 0.0, layer),
                backdrop_profile(ridge_zone_phase(160.0), px as f32, 0.0, layer),
                "inside one zone the ridge must not move with distance at px {px}"
            );
        }
        // Different zones, different ridge: each zone owns a profile.
        let coast: Vec<f32> = columns()
            .map(|px| backdrop_profile(ridge_zone_phase(100.0), px as f32, 0.0, layer))
            .collect();
        let town: Vec<f32> = columns()
            .map(|px| backdrop_profile(ridge_zone_phase(700.0), px as f32, 0.0, layer))
            .collect();
        assert_ne!(coast, town, "each zone must have its own ridge profile");
    }

    #[test]
    fn the_ridge_hands_over_smoothly_across_a_zone_boundary() {
        // The coast→town boundary sits at 520 m. The zone-carried phase
        // (and with it the ridge profile) must slide across the blend
        // window, not snap: consecutive steps move by a small fraction of
        // the full zone-to-zone phase gap.
        let mut prev = ridge_zone_phase(460.0);
        for d in [480.0, 500.0, 520.0, 540.0, 560.0] {
            let phase = ridge_zone_phase(d);
            assert!(
                (phase - prev).abs() < RIDGE_PHASE_PER_AMP * 0.2,
                "the ridge phase must blend across the boundary: {prev} -> {phase}"
            );
            prev = phase;
        }
    }

    #[test]
    fn a_landmark_grows_as_the_camera_closes() {
        // Item 10: the landmark's projected height must increase at every
        // step as the camera closes from 800 m out to 50 m.
        let geometry = track_geometry();
        let landmark = &world::course_landmarks()[0];
        let (width, height) = WATER_TOWER_SIZE;
        let camera_at = |distance: f32| world::Camera {
            distance,
            height: CAMERA_HEIGHT,
            lateral: 0.0,
            focal: FOCAL,
            horizon_y_px: VIEWPORT_HEIGHT * HORIZON_FRAC,
            viewport: Viewport {
                width: VIEWPORT_WIDTH,
                height: VIEWPORT_HEIGHT,
            },
        };
        let mut prev = 0.0;
        let mut steps = 0;
        // 800 m out down to 50 m, in 50 m steps.
        for step in 1..=16u32 {
            let depth = 850.0 - step as f32 * 50.0;
            let projector = world::Projector::new(&geometry, camera_at(landmark.distance - depth));
            let billboard = projector
                .billboard(landmark.distance, landmark.lateral, width, height)
                .expect("the landmark is inside the draw window at this depth");
            assert!(
                billboard.height > prev,
                "landmark height must grow at every step: depth {depth} m"
            );
            prev = billboard.height;
            steps += 1;
        }
        assert_eq!(steps, 16, "the closing run samples the full 800→50 m range");
    }

    #[test]
    fn frames_between_ticks_move_the_bike_by_alpha() {
        // Item 13: a rendered frame between two ticks must not repeat the
        // last tick's position — the draw blends the last two ticks' poses
        // by the clock's leftover fraction, without changing the tick rate.
        let mapping = mapping();
        let (mut race, geometry) = demo_race();
        while race.is_counting_down() {
            race.tick();
        }
        let mut session = GameSession::new(render::QualityTier::WebGpuCapable);
        // Two full ticks prime the blend so every sampled frame below has a
        // real (prev, curr) tick pair to interpolate.
        for _ in 0..2 {
            session.advance(
                &mut race,
                &geometry,
                &mapping,
                &[GameInput::Accelerate],
                sim::clock::TICK_DURATION_SECS,
            );
        }
        let half_tick = sim::clock::TICK_DURATION_SECS * 0.5;
        let mut poses = Vec::new();
        for _ in 0..60 {
            session.advance(
                &mut race,
                &geometry,
                &mapping,
                &[GameInput::Accelerate],
                half_tick,
            );
            poses.push(session.player_draw_pose());
        }
        for pair in poses.windows(2) {
            assert!(
                pair[1][0] != pair[0][0],
                "the bike's draw pose must change every frame (alpha {})",
                session.alpha()
            );
        }
    }

    #[test]
    fn the_draw_list_is_reused_across_frames() {
        // Item 15: composing must reuse one draw list, like the batch and
        // road mesh — capacity may grow once, never per frame.
        let mapping = mapping();
        let (mut race, geometry) = demo_race();
        while race.is_counting_down() {
            race.tick();
        }
        let mut batch = SpriteBatch::for_tier(render::QualityTier::WebGpuCapable);
        let mut road = RoadMesh::default();
        let mut draws: Vec<DrawItem> = Vec::new();
        let blend = TickBlend::identity(&race);
        compose(
            &race, &geometry, &mapping, &mut batch, &mut road, &mut draws, &blend,
        );
        let first = draws.capacity();
        assert!(first > 0, "the first compose sizes the draw list");
        for _ in 0..100 {
            race.push_input(GameInput::Accelerate);
            race.tick();
            let blend = TickBlend::identity(&race);
            compose(
                &race, &geometry, &mapping, &mut batch, &mut road, &mut draws, &blend,
            );
            assert_eq!(
                draws.capacity(),
                first,
                "the draw list must be reused, not reallocated"
            );
        }
    }

    #[test]
    fn demo_track_elevation_rolls_gently_across_the_lap() {
        // 7.5c (a): the demo lap must carry gentle inclines and declines —
        // never flat, never a wall. Sample every 10 m and read the profile.
        let geometry = track_geometry();
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        let mut max_grade = 0.0_f32;
        let mut prev = geometry.elevation_at(0.0);
        for step in 0..=200 {
            let d = step as f32 * 10.0;
            let elevation = geometry.elevation_at(d);
            min = min.min(elevation);
            max = max.max(elevation);
            max_grade = max_grade.max((elevation - prev).abs() / 10.0);
            prev = elevation;
        }
        assert!(
            max - min > 10.0,
            "the lap must roll: elevation spans {min}..{max}"
        );
        assert!(
            max_grade <= 0.041,
            "no section may wall up: steepest grade {max_grade}"
        );
        // The lap closes: the line sits at the same height it started.
        assert_eq!(geometry.elevation_at(0.0), 0.0);
        assert_eq!(geometry.elevation_at(2_000.0), 0.0);
    }

    #[test]
    fn road_projection_rises_over_the_climb_and_dips_into_the_valley() {
        // 7.5c (a) evidence: the projected road strip must carry the hill.
        // Each landmark is compared against the same point on a flat
        // reference course: the climb must project visibly above the flat
        // line and the valley visibly below it. (The projection converges
        // to the viewport centre, not the camera's horizon_y_px, so the
        // flat line — not the horizon — is the honest reference.)
        let geometry = track_geometry();
        let flat = world::RoadGeometry::new(Vec::new(), Vec::new(), 12.0);
        let camera_at = |distance: f32| world::Camera {
            distance,
            height: CAMERA_HEIGHT,
            lateral: 0.0,
            focal: FOCAL,
            horizon_y_px: VIEWPORT_HEIGHT * HORIZON_FRAC,
            viewport: Viewport {
                width: VIEWPORT_WIDTH,
                height: VIEWPORT_HEIGHT,
            },
        };

        // From the climb's base (300 m) the 14 m crest at 650 m looms
        // above where flat ground would project.
        let climbing = world::Projector::new(&geometry, camera_at(300.0));
        let flat_view = world::Projector::new(&flat, camera_at(300.0));
        let crest = climbing
            .project(650.0, 0.0)
            .expect("the crest is ahead of the camera");
        let crest_flat = flat_view
            .project(650.0, 0.0)
            .expect("flat ground at the same depth");
        assert!(
            crest.screen_y < crest_flat.screen_y - 5.0,
            "the climb must visibly rise above the flat line (crest {}, flat {})",
            crest.screen_y,
            crest_flat.screen_y
        );

        // From the crest the valley at 1 050 m (elevation 4 m) falls
        // below where flat ground would project.
        let on_crest = world::Projector::new(&geometry, camera_at(650.0));
        let crest_view = world::Projector::new(&flat, camera_at(650.0));
        let valley = on_crest
            .project(1_050.0, 0.0)
            .expect("the valley is ahead of the camera");
        let valley_flat = crest_view
            .project(1_050.0, 0.0)
            .expect("flat ground at the same depth");
        assert!(
            valley.screen_y > valley_flat.screen_y + 5.0,
            "the descent must visibly fall below the flat line (valley {}, flat {})",
            valley.screen_y,
            valley_flat.screen_y
        );
    }
}
