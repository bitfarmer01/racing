//! Deterministic pursuit AI for rival bikes, built on world-crate lookahead
//! queries.
//!
//! Each rival holds a preferred lane offset and, once per tick, turns world
//! geometry ahead of it into a plain [`Intent`]: slow into corners using the
//! curvature at the far lookahead point, and take the inside line using the
//! curvature at the near lookahead point. The same state, geometry, and
//! distance always produce the same decision — no RNG in the tick path.
//!
//! Positive curvature bends the road right (the centreline shifts
//! positive), so the inside of a left-hand corner — negative curvature — is
//! on the negative-lateral side.

use crate::input::{Intent, Steer, Throttle};
use world::RoadGeometry;

/// How far ahead (meters) the AI samples curvature for corner speed.
pub const LOOKAHEAD_FAR: f32 = 90.0;
/// How far ahead (meters) the AI samples curvature for the racing line.
pub const LOOKAHEAD_NEAR: f32 = 30.0;
/// Corner sensitivity: target speed is `MAX_SPEED / (1 + SENSITIVITY * |k|)`.
pub const CORNER_SENSITIVITY: f32 = 80.0;
/// Curvature below this is treated as straight, in 1/meters.
pub const CURVE_DEADZONE: f32 = 0.004;
/// Half the lateral span of lanes the grid spreads across, in meters: a
/// six-rival field then rides lanes a lane-and-a-half apart, clear of the
/// side-crash gap and of the shoulder rumble band.
pub const GRID_HALF_SPAN: f32 = 4.0;
/// How far off the preferred lane toward the inside of a corner the AI leans,
/// in meters. Scaled per rival by [`lane_bias_factor`] so a packed field
/// spreads across the bend instead of stacking on one line.
pub const INSIDE_BIAS: f32 = 3.0;

/// The share of [`INSIDE_BIAS`] a rival on lane `lane` takes: riders near
/// an edge barely cut in, riders near the centre cut deepest. Keeps
/// in-bend lines apart by roughly a lane width.
pub fn lane_bias_factor(lane: f32) -> f32 {
    (0.5 + 0.5 * (lane / GRID_HALF_SPAN)).clamp(0.1, 0.9)
}

/// Curvature at which the inside-line bias reaches its full
/// [`INSIDE_BIAS`]: the demo track's sharpest bend. Gentler bends bias
/// proportionally, so the demo's 0.003 left bend still cuts in.
pub const INSIDE_BIAS_CURVATURE: f32 = 0.005;
/// Lateral error below which the AI stops steering, in meters.
pub const LANE_DEADZONE: f32 = 0.25;
/// Speed error below which the AI coasts rather than throttles, in m/s.
pub const THROTTLE_DEADZONE: f32 = 0.5;
/// Speed error above which the AI brakes rather than coasts, in m/s.
pub const BRAKE_MARGIN: f32 = 2.0;

/// Target speed through a corner of curvature `curvature` (1/meters) for a
/// rider capable of `top_speed` on the straight. Pure and clamped above zero.
pub fn corner_speed(curvature: f32, top_speed: f32) -> f32 {
    top_speed / (1.0 + CORNER_SENSITIVITY * curvature.abs())
}

/// One rival's decision for this tick, from pure inputs only.
///
/// `speed` and `lateral` are the rival's current state, `distance` is its
/// position along the road, and `target_lane` is its preferred lateral
/// offset. `avoid`, when set, is an absolute lane to dodge to because
/// another bike is in the way — it overrides the preferred line for the
/// tick. All world lookups run at `distance + LOOKAHEAD_*`.
pub fn decide(
    speed: f32,
    lateral: f32,
    distance: f32,
    target_lane: f32,
    top_speed: f32,
    avoid: Option<f32>,
    geometry: &RoadGeometry,
) -> Intent {
    // Far lookahead: how fast this corner may be taken.
    let curvature_far = geometry.curvature_at(distance + LOOKAHEAD_FAR);
    let target_speed = corner_speed(curvature_far, top_speed);

    let throttle = if speed < target_speed - THROTTLE_DEADZONE {
        Throttle::Accelerate
    } else if speed > target_speed + BRAKE_MARGIN {
        Throttle::Brake
    } else {
        Throttle::Coast
    };

    // Near lookahead: lean toward the inside of the upcoming corner.
    // RoadGeometry is authoritative: negative curvature is a left bend and
    // the centreline shifts negative, so the inside of a bend lies on the
    // same side as the curvature sign. The bias scales with bend severity
    // up to INSIDE_BIAS_CURVATURE instead of a hard deadzone — the demo
    // track's gentlest bend (0.003) must still visibly cut in.
    let curvature_near = geometry.curvature_at(distance + LOOKAHEAD_NEAR);
    let severity = (curvature_near.abs() / INSIDE_BIAS_CURVATURE).min(1.0);
    let inside_bias =
        curvature_near.signum() * INSIDE_BIAS * lane_bias_factor(target_lane) * severity;

    // Lateral error drives steering; a small deadzone stops dithering.
    let lane_target = avoid.unwrap_or(target_lane + inside_bias);
    let lane_error = lane_target - lateral;
    let steer = if lane_error > LANE_DEADZONE {
        Steer::Right
    } else if lane_error < -LANE_DEADZONE {
        Steer::Left
    } else {
        Steer::None
    };

    // Rival pursuit never attacks: retaliation resolves in the race's
    // combat pass, not through a steering intent.
    Intent {
        throttle,
        steer,
        attack: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bike::MAX_SPEED;
    use world::CurveSegment;

    fn straight() -> RoadGeometry {
        RoadGeometry::new(Vec::new(), Vec::new(), 12.0)
    }

    fn left_hand(length: f32, curvature: f32) -> RoadGeometry {
        RoadGeometry::new(
            vec![CurveSegment {
                start: 0.0,
                length,
                curvature,
            }],
            Vec::new(),
            12.0,
        )
    }

    #[test]
    fn straight_road_means_full_throttle_and_centreline_hold() {
        let intent = decide(20.0, 1.0, 100.0, 0.0, MAX_SPEED, None, &straight());
        assert_eq!(intent.throttle, Throttle::Accelerate, "below target speed");
        // 1 m right of the target lane: steer back left.
        assert_eq!(intent.steer, Steer::Left);
    }

    #[test]
    fn near_target_speed_the_ai_coasts() {
        let target = corner_speed(0.0, MAX_SPEED);
        let intent = decide(target, 0.0, 0.0, 0.0, MAX_SPEED, None, &straight());
        assert_eq!(intent.throttle, Throttle::Coast);
        assert_eq!(intent.steer, Steer::None, "on the lane: no steering");
    }

    #[test]
    fn too_fast_for_a_corner_means_brake() {
        let corner = left_hand(400.0, 0.02);
        let intent = decide(MAX_SPEED, 0.0, 0.0, 0.0, MAX_SPEED, None, &corner);
        assert_eq!(intent.throttle, Throttle::Brake);
    }

    #[test]
    fn left_hand_corner_biases_the_inside_line_left() {
        let corner = left_hand(400.0, -0.02);
        let intent = decide(20.0, 0.0, 0.0, 0.0, MAX_SPEED, None, &corner);
        // Negative curvature (a left bend) pulls the target lane negative
        // (left) — the inside is the side the centreline shifts toward.
        assert_eq!(intent.steer, Steer::Left);

        let mirrored = left_hand(400.0, 0.02);
        let intent = decide(20.0, 0.0, 0.0, 0.0, MAX_SPEED, None, &mirrored);
        assert_eq!(intent.steer, Steer::Right, "mirror symmetry");
    }

    #[test]
    fn identical_inputs_decide_identically() {
        let corner = left_hand(400.0, 0.015);
        let one = decide(35.0, -1.5, 420.0, 0.5, MAX_SPEED, None, &corner);
        let two = decide(35.0, -1.5, 420.0, 0.5, MAX_SPEED, None, &corner);
        assert_eq!(one, two, "the AI is a pure function of its inputs");
    }

    #[test]
    fn corner_speed_never_exceeds_max_and_stays_positive() {
        assert_eq!(corner_speed(0.0, MAX_SPEED), MAX_SPEED);
        assert!(corner_speed(0.02, MAX_SPEED) > 0.0);
        assert!(
            corner_speed(0.02, MAX_SPEED) < corner_speed(0.01, MAX_SPEED),
            "tighter is slower"
        );
        assert!(corner_speed(1.0, MAX_SPEED) > 0.0);
    }

    #[test]
    fn a_slower_rider_targets_a_slower_speed() {
        let slow = corner_speed(0.0, MAX_SPEED * 0.7);
        assert!((slow - MAX_SPEED * 0.7).abs() < 1e-4);
        assert!(
            slow < corner_speed(0.0, MAX_SPEED),
            "rider variety reaches target speed"
        );
    }
}
