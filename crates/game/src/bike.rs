//! The per-bike control state machine: accelerate, brake, coast, crash, and
//! the off-road modifier.
//!
//! [`step_bike`] is a pure function from one tick's intent plus the current
//! snapshot to the next snapshot. The race driver owns everything the state
//! machine does not: position integration, collisions, and scoring.
//!
//! All speeds are meters per second; positions are meters.

use crate::input::{Intent, Steer, Throttle};

/// Top speed on tarmac, in meters per second (~216 km/h).
pub const MAX_SPEED: f32 = 60.0;
/// Throttle acceleration, in meters per second squared.
pub const ACCEL: f32 = 18.0;
/// Brake deceleration, in meters per second squared.
pub const BRAKE_DECEL: f32 = 35.0;
/// Engine-braking drag while coasting, in meters per second squared.
pub const COAST_DRAG: f32 = 6.0;
/// Top speed off the tarmac, in meters per second.
pub const OFFROAD_MAX_SPEED: f32 = 18.0;
/// Extra drag applied while off-road above [`OFFROAD_MAX_SPEED`], in meters
/// per second squared.
pub const OFFROAD_DRAG: f32 = 14.0;
/// Lateral speed while leaning, in meters per second.
pub const STEER_RATE: f32 = 5.0;
/// Forward speed at which steering reaches full authority, in m/s. Below it
/// the bike is upright and barely steers, so a stopped bike cannot wiggle.
pub const MIN_STEER_SPEED: f32 = 8.0;
/// Cornering force: taking a curve at speed throws the bike outward at
/// `speed² · curvature · CORNER_FORCE` m/s. Corners cost speed or skill.
pub const CORNER_FORCE: f32 = 0.32;
/// How hard the bike scrubs speed while sliding in a crash, in meters per
/// second squared.
pub const CRASH_DRAG: f32 = 25.0;
/// Duration of a crash, in seconds, before the rider picks the bike up.
pub const CRASH_DURATION_SECS: f32 = 2.0;
/// Going off-road above this speed is a crash, in meters per second.
pub const OFFROAD_CRASH_SPEED: f32 = 30.0;

/// Control state of one bike.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Control {
    /// No throttle: engine braking only.
    #[default]
    Coast,
    /// Throttle on.
    Accelerate,
    /// Brake on.
    Brake,
    /// Sliding: no control until the crash timer expires.
    Crashed,
}

/// Snapshot of everything the state machine owns for one bike.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BikeSnapshot {
    /// Forward speed, in meters per second.
    pub speed: f32,
    /// Lateral position across the road, in meters (negative is left).
    pub lateral: f32,
    /// Current control state.
    pub control: Control,
    /// Seconds remaining before a crashed rider is back up; unused otherwise.
    pub crash_timer: f32,
}

impl BikeSnapshot {
    /// A stationary, upright bike on the centreline.
    pub fn at_rest() -> Self {
        Self::default()
    }

    /// Whether the bike is currently off the tarmac: beyond `half_width`
    /// meters from the road centre.
    pub fn is_off_road(&self, half_width: f32) -> bool {
        self.lateral.abs() > half_width
    }
}

/// Advance one bike by one tick of `dt` seconds under `intent`.
///
/// Pure: no pool access, no RNG, no clock. Crashed riders cannot steer and
/// only scrub speed; a crash timer at or below zero hands control back on
/// coast. Off-road riders accelerate no faster than tarmac but bleed down to
/// [`OFFROAD_MAX_SPEED`]. `corner_curvature` is the road curvature under the
/// bike (1/meters, positive bends left): speed throws the bike outward.
pub fn step_bike(
    mut bike: BikeSnapshot,
    intent: Intent,
    off_road: bool,
    corner_curvature: f32,
    dt: f32,
) -> BikeSnapshot {
    if bike.control == Control::Crashed {
        bike.crash_timer = (bike.crash_timer - dt).max(0.0);
        bike.speed = (bike.speed - CRASH_DRAG * dt).max(0.0);
        if bike.crash_timer == 0.0 {
            bike.control = Control::Coast;
        }
        return bike;
    }

    bike.control = match intent.throttle {
        Throttle::Accelerate => Control::Accelerate,
        Throttle::Brake => Control::Brake,
        Throttle::Coast => Control::Coast,
    };

    let speed_cap = if off_road {
        OFFROAD_MAX_SPEED
    } else {
        MAX_SPEED
    };
    bike.speed = match bike.control {
        Control::Accelerate => {
            let next = bike.speed + ACCEL * dt;
            if off_road && next > speed_cap {
                // Too fast for the dirt: heavy drag pulls the bike back down.
                (bike.speed - OFFROAD_DRAG * dt).max(speed_cap)
            } else {
                next.min(speed_cap)
            }
        }
        Control::Brake => (bike.speed - BRAKE_DECEL * dt).max(0.0),
        _ => (bike.speed - COAST_DRAG * dt).max(0.0),
    };

    // Steering authority scales with speed: the bike must be moving to lean.
    let steer_scale = (bike.speed / MIN_STEER_SPEED).min(1.0);
    let steer_nudge = match intent.steer {
        Steer::Left => -STEER_RATE * dt,
        Steer::Right => STEER_RATE * dt,
        Steer::None => 0.0,
    };
    // Outward cornering force: positive curvature bends left, so the push
    // is to the right (positive lateral) — the road flings you wide.
    let corner_push = bike.speed * bike.speed * corner_curvature * CORNER_FORCE * dt;
    bike.lateral += steer_nudge * steer_scale + corner_push;

    bike
}

/// The crash trigger, evaluated by the race driver after the lateral move:
/// running off the tarmac above [`OFFROAD_CRASH_SPEED`] is a crash.
pub fn should_crash(bike: &BikeSnapshot, half_width: f32) -> bool {
    bike.control != Control::Crashed
        && bike.is_off_road(half_width)
        && bike.speed > OFFROAD_CRASH_SPEED
}

/// Put a bike into a crash.
pub fn crash(bike: &mut BikeSnapshot) {
    bike.control = Control::Crashed;
    bike.crash_timer = CRASH_DURATION_SECS;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::GameInput;

    /// Drain a list of input events into one intent, the way a tick does.
    fn intent_of(events: &[GameInput]) -> Intent {
        let mut intent = Intent::default();
        for event in events {
            intent.absorb(*event);
        }
        intent
    }

    fn stepped(bike: BikeSnapshot, events: &[GameInput], off_road: bool) -> BikeSnapshot {
        step_bike(
            bike,
            intent_of(events),
            off_road,
            0.0,
            sim::clock::TICK_DURATION_SECS as f32,
        )
    }

    #[test]
    fn accelerating_from_rest_gains_speed_and_caps_at_max() {
        let bike = stepped(BikeSnapshot::at_rest(), &[GameInput::Accelerate], false);
        assert!(bike.control == Control::Accelerate);
        assert!(bike.speed > 0.0);

        // Already at the cap: throttle cannot exceed it.
        let capped = stepped(
            BikeSnapshot {
                speed: MAX_SPEED,
                ..BikeSnapshot::at_rest()
            },
            &[GameInput::Accelerate],
            false,
        );
        assert_eq!(capped.speed, MAX_SPEED);
    }

    #[test]
    fn braking_slows_and_never_reverses() {
        let bike = stepped(
            BikeSnapshot {
                speed: 20.0,
                ..BikeSnapshot::at_rest()
            },
            &[GameInput::Brake],
            false,
        );
        assert_eq!(bike.control, Control::Brake);
        assert!(bike.speed < 20.0);

        // A stopped bike under brake stays stopped, never negative.
        let stopped = stepped(BikeSnapshot::at_rest(), &[GameInput::Brake], false);
        assert_eq!(stopped.speed, 0.0);
    }

    #[test]
    fn coasting_bleeds_speed_toward_zero() {
        let bike = stepped(
            BikeSnapshot {
                speed: 30.0,
                ..BikeSnapshot::at_rest()
            },
            &[],
            false,
        );
        assert_eq!(bike.control, Control::Coast);
        assert!(
            (bike.speed - (30.0 - COAST_DRAG * sim::clock::TICK_DURATION_SECS as f32)).abs() < 1e-5
        );

        // Coasting past zero clamps at rest.
        assert_eq!(stepped(BikeSnapshot::at_rest(), &[], false).speed, 0.0);
    }

    #[test]
    fn steering_moves_lateral_both_ways() {
        // Steering needs speed: the bike must be moving to lean.
        let rolling = BikeSnapshot {
            speed: 20.0,
            ..BikeSnapshot::at_rest()
        };
        let left = stepped(rolling, &[GameInput::SteerLeft], false);
        let right = stepped(rolling, &[GameInput::SteerRight], false);
        assert!(left.lateral < 0.0);
        assert!(right.lateral > 0.0);
        assert!(
            (left.lateral + right.lateral).abs() < 1e-6,
            "symmetric rates"
        );
    }

    #[test]
    fn off_road_throttle_bleeds_speed_down_to_the_dirt_cap() {
        // Full speed on tarmac, then off-road with throttle held.
        let mut bike = BikeSnapshot {
            speed: MAX_SPEED,
            ..BikeSnapshot::at_rest()
        };
        let half_width = 6.0;
        // Place the bike off-road without going through the crash check:
        // the state machine itself only reacts to the off_road flag.
        bike.lateral = half_width + 1.0;
        let before = bike.speed;
        bike = stepped(bike, &[GameInput::Accelerate], true);
        assert!(bike.speed < before, "off-road drag scrubs speed");
        assert!(
            bike.speed >= OFFROAD_MAX_SPEED - 0.5,
            "drag stops at the dirt cap"
        );
    }

    #[test]
    fn crashed_bikes_scrub_speed_and_cannot_steer() {
        let mut bike = BikeSnapshot {
            speed: 25.0,
            lateral: 8.0,
            control: Control::Crashed,
            crash_timer: 1.0,
        };
        let speed_before = bike.speed;
        bike = stepped(bike, &[GameInput::SteerRight, GameInput::Accelerate], true);
        assert_eq!(bike.control, Control::Crashed, "still crashed mid-timer");
        assert!(bike.speed < speed_before, "sliding scrubs speed");
        assert!(
            (bike.lateral - 8.0).abs() < 1e-6,
            "no steering while crashed"
        );
    }

    #[test]
    fn crash_timer_expires_back_to_coast() {
        let mut bike = BikeSnapshot {
            speed: 10.0,
            lateral: 0.0,
            control: Control::Crashed,
            crash_timer: sim::clock::TICK_DURATION_SECS as f32,
        };
        bike = stepped(bike, &[], false);
        assert_eq!(bike.control, Control::Coast, "timer expired this tick");
        assert_eq!(bike.crash_timer, 0.0);
    }

    #[test]
    fn cornering_force_pushes_outward_and_grows_with_speed() {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        let at_speed = |speed: f32, curvature: f32| {
            step_bike(
                BikeSnapshot {
                    speed,
                    ..BikeSnapshot::at_rest()
                },
                Intent::default(),
                false,
                curvature,
                dt,
            )
        };
        // Left-hand curve (positive curvature) pushes a fast bike right.
        let fast = at_speed(50.0, 0.005);
        assert!(fast.lateral > 0.0, "left curve throws the bike right");
        // The same curve at walking pace barely moves the bike.
        let slow = at_speed(5.0, 0.005);
        assert!(
            slow.lateral.abs() < fast.lateral.abs() * 0.05,
            "slow is stable"
        );
        // Mirror symmetry: a right-hand curve pushes left.
        let mirrored = at_speed(50.0, -0.005);
        assert!((mirrored.lateral + fast.lateral).abs() < 1e-6);
    }

    #[test]
    fn a_stopped_bike_cannot_steer() {
        let bike = stepped(BikeSnapshot::at_rest(), &[GameInput::SteerRight], false);
        assert!(bike.lateral.abs() < 0.01, "no lean-move at zero speed");
    }

    #[test]
    fn crash_trigger_and_recovery_round_trip() {
        let half_width = 6.0;
        let fast = BikeSnapshot {
            speed: 40.0,
            lateral: half_width + 1.0,
            ..BikeSnapshot::at_rest()
        };
        assert!(should_crash(&fast, half_width));

        let slow = BikeSnapshot {
            speed: 10.0,
            ..fast
        };
        assert!(
            !should_crash(&slow, half_width),
            "low-speed dirt is survivable"
        );

        let mut crashed = fast;
        crash(&mut crashed);
        assert_eq!(crashed.control, Control::Crashed);
        assert_eq!(crashed.crash_timer, CRASH_DURATION_SECS);
    }
}
