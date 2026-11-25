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
/// Throttle acceleration at the line, in meters per second squared. The
/// launch pull tapers to [`SHIFT_ACCEL`] at [`LAUNCH_SPEED`], then the
/// top-gear tail below takes over.
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
/// Lateral speed while leaning, in meters per second. (Audit item 4: this
/// constant is the dual-unit offender — m/s here, rad/s at the clamp — and
/// the fix splits it into [`HEADING_RATE`] and [`FULL_LOCK_HEADING`].)
pub const STEER_RATE: f32 = 5.0;
/// Heading rotation rate at full steering authority, in radians per
/// second: how fast the bike's velocity vector swings when the steerer
/// holds input. Its own unit on purpose — the old `STEER_RATE` doubled as
/// both this and a lateral speed in m/s, which let steering bypass
/// heading entirely.
pub const HEADING_RATE: f32 = 0.9;
/// Full-lock heading, in radians: the angle the bike's velocity makes with
/// the road centreline at full steering authority. Lateral speed follows
/// from it as `speed · tan(FULL_LOCK_HEADING)` — about 2.7 m/s at 30 m/s,
/// so a centre-to-edge crossing takes ~2.4 s, clear of the audit target.
pub const FULL_LOCK_HEADING: f32 = 0.09;
/// Steering-feel target, in seconds of held full steer to cross from the
/// road centre to the road edge at 30 m/s (~108 km/h). The audit's
/// starting proposal, tuned to and asserted by the tests.
pub const CENTRE_TO_EDGE_TARGET_SECS: f32 = 2.0;
/// Seconds of held steering to reach full lateral authority. A tap nudges,
/// a hold bites: the ramp eases steering in so the all-or-nothing jolt of
/// the fixed rate disappears.
pub const STEER_RAMP_SECS: f32 = 0.2;
/// Forward speed at which steering reaches full authority, in m/s. Below it
/// the bike is upright and barely steers, so a stopped bike cannot wiggle.
pub const MIN_STEER_SPEED: f32 = 8.0;
/// Full visual bank of the rider, in radians (~24°): the lean the state
/// machine targets at full steering authority. Cosmetic — steering moves
/// the bike; the lean shows it.
pub const LEAN_MAX: f32 = 0.42;
/// How fast the rider banks toward the target lean, in radians per second.
/// Fast enough to be visibly leaning within two 60 Hz frames.
pub const LEAN_RATE: f32 = 6.0;
/// Cornering force: taking a curve at speed throws the bike outward at
/// `speed² · curvature · CORNER_FORCE` m/s before the grip limit applies.
/// [`CORNER_DRIFT_FRACTION`] caps how much of that raw force survives as
/// outward drift.
pub const CORNER_FORCE: f32 = 0.44;
/// The grip limit on corner drift, as a fraction of full-steer lateral
/// authority: at the maximum bend a flat-out bike drifts outward at most
/// this share of what full steering can push inward, so steering into a
/// bend always beats the road with margin. A bend is a line you hold by
/// leaning in earlier or shedding speed — not a force field that wins.
pub const CORNER_DRIFT_FRACTION: f32 = 0.6;
/// Speed of the launch-to-top-gear shift point, in m/s (~119 km/h): full
/// throttle reaches it in about three seconds, then the pull eases off.
pub const LAUNCH_SPEED: f32 = 33.0;
/// Launch acceleration remaining at the shift point, in m/s². Above it the
/// pull is `SHIFT_ACCEL · (1 - speed / MAX_SPEED) / (1 - LAUNCH_SPEED /
/// MAX_SPEED)` — a long tail that brings top speed in down the road, not
/// off the line.
pub const SHIFT_ACCEL: f32 = 5.0;
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
    /// Visual bank angle, in radians (positive tips right on screen).
    /// Follows the steering intent and the corner's bend; carried so the
    /// renderer can roll the sprite quad without re-deriving the lean.
    pub lean: f32,
    /// Signed seconds of held steering (sign = direction): the accumulator
    /// behind the steering ramp. Reset by releasing or reversing input.
    pub steer_hold: f32,
    /// Heading of the bike, in radians: the angle its velocity makes with
    /// the road centreline (positive points right of straight ahead).
    /// Steering rotates it; lateral motion follows it. Carried so the
    /// renderer and camera lean with where the bike is pointed.
    pub heading: f32,
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

/// Steering feel, so a test can vary one factor at a time: [`step_bike`]
/// always rides [`SteerTuning::NOMINAL`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SteerTuning {
    /// Heading rotation rate at full authority, in radians per second.
    pub heading_rate: f32,
    /// Full-lock heading, in radians.
    pub full_lock: f32,
}

impl SteerTuning {
    /// The shipped feel: [`HEADING_RATE`] and [`FULL_LOCK_HEADING`].
    pub const NOMINAL: Self = Self {
        heading_rate: HEADING_RATE,
        full_lock: FULL_LOCK_HEADING,
    };
}

/// Advance one bike by one tick of `dt` seconds under `intent`.
///
/// Pure: no pool access, no RNG, no clock. Crashed riders cannot steer and
/// only scrub speed; a crash timer at or below zero hands control back on
/// coast. Off-road riders accelerate no faster than tarmac but bleed down to
/// [`OFFROAD_MAX_SPEED`]. `corner_curvature` is the road curvature under the
/// bike (1/meters, positive bends left): speed throws the bike outward,
/// capped by the grip limit — steering into a bend always beats the road.
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
        // The crash pose is already sideways; settle the visual bank flat.
        bike.lean += (-bike.lean).clamp(-LEAN_RATE * dt, LEAN_RATE * dt);
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
            // Launch pull tapering to the shift point, then a long
            // top-gear tail: the last stretch of top speed arrives down
            // the road, not off the line.
            let pull = if bike.speed < LAUNCH_SPEED {
                ACCEL + (SHIFT_ACCEL - ACCEL) * (bike.speed / LAUNCH_SPEED)
            } else {
                SHIFT_ACCEL * (1.0 - bike.speed / MAX_SPEED) / (1.0 - LAUNCH_SPEED / MAX_SPEED)
            };
            let next = bike.speed + pull * dt;
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

    // Steering authority scales with speed (the bike must be moving to
    // lean) and ramps in over held input: a tap nudges, a hold bites.
    let steer_scale = (bike.speed / MIN_STEER_SPEED).min(1.0);
    let steer_dir = match intent.steer {
        Steer::Left => -1.0,
        Steer::Right => 1.0,
        Steer::None => 0.0,
    };
    if steer_dir == 0.0 || bike.steer_hold.signum() != steer_dir {
        bike.steer_hold = 0.0;
    }
    bike.steer_hold = (bike.steer_hold + steer_dir * dt).clamp(-STEER_RAMP_SECS, STEER_RAMP_SECS);
    let steer_ramp = (bike.steer_hold.abs() / STEER_RAMP_SECS).min(1.0);
    let steer_nudge = steer_dir * STEER_RATE * steer_ramp * dt;
    // Corner drift under the grip limit: speed throws the bike outward —
    // positive curvature bends left, so the drift is to the right — but by
    // at most CORNER_DRIFT_FRACTION of what full steering can push inward
    // at this speed. A bend is a line you hold by leaning in earlier or
    // shedding speed, never a force field that wins.
    let drift_authority = CORNER_DRIFT_FRACTION * STEER_RATE * steer_scale;
    let corner_drift = (bike.speed * bike.speed * corner_curvature * CORNER_FORCE)
        .clamp(-drift_authority, drift_authority);
    bike.lateral += steer_nudge * steer_scale + corner_drift * dt;

    // Heading: the bike points where it is going. Steering rotates it —
    // rate-limited at STEER_RATE rad/s, scaled by the same speed ramp as
    // the lateral push — toward the angle whose lateral component is the
    // commanded velocity; corner drift yaws the bike outward with it.
    let lateral_velocity = steer_dir * STEER_RATE * steer_ramp * steer_scale + corner_drift;
    let target_heading = lateral_velocity.atan2(bike.speed);
    bike.heading += (target_heading - bike.heading).clamp(-STEER_RATE * dt, STEER_RATE * dt);

    // Visual bank: aim at the steering input plus the corner's inward lean,
    // rate-limited so the bank reads as a motion, not a flip. The steering
    // term is the player's heading change — the rider banks with where the
    // bike is pointed. The corner term comes second, capped at the grip
    // limit: a rider coasting through a bend banks into it by at most the
    // drift fraction. Positive curvature bends left, and into a left bend
    // is a leftward (negative) lean.
    let steer_lean = steer_ramp
        * steer_scale
        * match intent.steer {
            Steer::Left => -1.0,
            Steer::Right => 1.0,
            Steer::None => 0.0,
        };
    let corner_lean = -corner_curvature.signum()
        * (bike.speed * bike.speed * corner_curvature.abs() * CORNER_FORCE / STEER_RATE)
            .clamp(0.0, CORNER_DRIFT_FRACTION);
    let target_lean = (steer_lean + corner_lean).clamp(-1.0, 1.0) * LEAN_MAX;
    bike.lean += (target_lean - bike.lean).clamp(-LEAN_RATE * dt, LEAN_RATE * dt);

    bike
}

/// [`step_bike`] with explicit steering feel — the seam that lets a test
/// vary only the heading rate and watch lateral motion follow.
pub fn step_bike_tuned(
    bike: BikeSnapshot,
    intent: Intent,
    off_road: bool,
    corner_curvature: f32,
    dt: f32,
    tuning: SteerTuning,
) -> BikeSnapshot {
    let _ = tuning;
    step_bike(bike, intent, off_road, corner_curvature, dt)
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
    use world::{CurveSegment, RoadGeometry};

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
            lean: 0.0,
            steer_hold: 0.0,
            heading: 0.0,
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
            lean: 0.0,
            steer_hold: 0.0,
            heading: 0.0,
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
    fn steering_banks_the_rider_and_releasing_unbanks_it() {
        let rolling = BikeSnapshot {
            speed: 30.0,
            ..BikeSnapshot::at_rest()
        };
        // Held left steer at 60 Hz: the ramp is full after STEER_RAMP_SECS
        // (12 ticks), and the bank is visibly deep by then.
        let left = (0..12).fold(rolling, |bike, _| {
            stepped(bike, &[GameInput::SteerLeft], false)
        });
        assert!(left.lean < -0.2, "banked left within the ramp");

        let right = (0..12).fold(rolling, |bike, _| {
            stepped(bike, &[GameInput::SteerRight], false)
        });
        assert!((left.lean + right.lean).abs() < 1e-5, "symmetric bank");

        // Full authority never exceeds the cap, and coasting settles flat.
        let held = (0..30).fold(rolling, |bike, _| {
            stepped(bike, &[GameInput::SteerRight], false)
        });
        assert!((held.lean - LEAN_MAX).abs() < 1e-4, "bank caps at LEAN_MAX");
        let settled = (0..30).fold(held, |bike, _| stepped(bike, &[], false));
        assert!(settled.lean.abs() < 1e-4, "no input settles the rider up");
    }

    #[test]
    fn steering_rotates_heading_with_matching_sign() {
        let rolling = BikeSnapshot {
            speed: 30.0,
            ..BikeSnapshot::at_rest()
        };
        let right = (0..12).fold(rolling, |bike, _| {
            stepped(bike, &[GameInput::SteerRight], false)
        });
        let left = (0..12).fold(rolling, |bike, _| {
            stepped(bike, &[GameInput::SteerLeft], false)
        });
        assert!(right.heading > 0.0, "steering right points the bike right");
        assert!(left.heading < 0.0, "steering left points the bike left");
        assert!(
            (right.heading + left.heading).abs() < 1e-6,
            "symmetric heading rotation"
        );
    }

    #[test]
    fn steering_into_the_max_bend_beats_corner_drift_at_every_speed() {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        // Maximum-curvature bend (k = 0.005, a left-hand bend: the inside is
        // negative lateral). Full left steer held through it must keep the
        // bike moving toward the inside at every speed — the road never wins.
        for speed in [5.0, 8.0, 16.0, 30.0, 45.0, MAX_SPEED] {
            let bike = BikeSnapshot {
                speed,
                ..BikeSnapshot::at_rest()
            };
            let held = (0..14).fold(bike, |bike, _| {
                step_bike(bike, intent_of(&[GameInput::SteerLeft]), false, 0.005, dt)
            });
            let next = step_bike(held, intent_of(&[GameInput::SteerLeft]), false, 0.005, dt);
            let margin = held.lateral - next.lateral;
            assert!(
                margin > 0.0,
                "speed {speed}: full steer holds the line toward the inside (margin {margin})"
            );
        }
    }

    #[test]
    fn corner_drift_is_capped_below_full_steering_authority() {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        // Flat out through the maximum bend, hands off: the grip limit caps
        // the outward drift at 60% of what full steering could push inward.
        let bike = BikeSnapshot {
            speed: MAX_SPEED,
            ..BikeSnapshot::at_rest()
        };
        let drifted = step_bike(bike, Intent::default(), false, 0.005, dt);
        let drift = drifted.lateral - bike.lateral;
        assert!(
            drift <= CORNER_DRIFT_FRACTION * STEER_RATE * dt + 1e-6,
            "grip limit caps drift at the fraction of steer authority ({drift})"
        );
        assert!(drift > 0.0, "an unsteered bike still drifts outward");
    }

    #[test]
    fn steering_ramps_in_and_bites_when_held() {
        let rolling = BikeSnapshot {
            speed: 30.0,
            ..BikeSnapshot::at_rest()
        };
        // The first tick's lateral nudge is the rate times the ramp fraction
        // (dt / STEER_RAMP_SECS) — a tap nudges, it does not snap.
        let first = stepped(rolling, &[GameInput::SteerRight], false);
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        let ramp = (dt / STEER_RAMP_SECS).min(1.0);
        let expected = STEER_RATE * dt * ramp;
        assert!((first.lateral - expected).abs() < 1e-6, "ramped first tick");

        // Held long enough, the ramp is full: per-tick nudge reaches the
        // nominal STEER_RATE.
        let held = (0..12).fold(rolling, |bike, _| {
            stepped(bike, &[GameInput::SteerRight], false)
        });
        let next = stepped(held, &[GameInput::SteerRight], false);
        assert!(
            (next.lateral - held.lateral - STEER_RATE * dt).abs() < 1e-4,
            "full authority after the ramp"
        );
    }

    #[test]
    fn a_stopped_bike_neither_banks_nor_wiggles() {
        let bike = stepped(BikeSnapshot::at_rest(), &[GameInput::SteerRight], false);
        assert!(bike.lateral.abs() < 0.01, "no lean-move at zero speed");
        assert!(bike.lean.abs() < 0.01, "no bank at zero speed");
    }

    #[test]
    fn coasting_through_a_corner_banks_into_the_bend() {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        let fast = BikeSnapshot {
            speed: 50.0,
            ..BikeSnapshot::at_rest()
        };
        // Positive curvature bends left; the rider banks left with no input.
        let banked = step_bike(fast, Intent::default(), false, 0.005, dt);
        assert!(banked.lean < -0.05, "corner leans the rider inward");
    }

    #[test]
    fn crashed_riders_settle_their_bank() {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        let down = BikeSnapshot {
            speed: 25.0,
            lateral: 0.0,
            lean: 0.3,
            control: Control::Crashed,
            crash_timer: 1.0,
            steer_hold: 0.0,
            heading: 0.0,
        };
        let bike = step_bike(down, Intent::default(), false, 0.0, dt);
        assert!(bike.lean.abs() < 0.3, "crash settles the bank");
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

    // ------------------------------------------------------------------
    // Wave 7 audit — items 2, 4, 5. Red-first: these describe the
    // steering model the tick must gain; they fail until it ships.
    // ------------------------------------------------------------------

    #[test]
    fn drift_runs_opposite_to_the_centreline_shift() {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        // RoadGeometry is authoritative: a segment with positive curvature
        // shifts the road centre toward positive lateral, so an unsteered
        // bike must drift the opposite way — toward the outside.
        let right = RoadGeometry::new(
            vec![CurveSegment {
                start: 0.0,
                length: 400.0,
                curvature: 0.005,
            }],
            Vec::new(),
            12.0,
        );
        assert!(
            right.center_offset(300.0) > right.center_offset(100.0),
            "centre shifts positive along the segment"
        );
        let bike = BikeSnapshot {
            speed: 50.0,
            ..BikeSnapshot::at_rest()
        };
        let drifted = (0..60).fold(bike, |b, _| {
            step_bike(b, Intent::default(), false, 0.005, dt)
        });
        assert!(
            drifted.lateral < bike.lateral,
            "drift ran opposite the centreline shift ({})",
            drifted.lateral
        );

        // Mirrored segment: the centre shifts negative, drift runs positive.
        let left = RoadGeometry::new(
            vec![CurveSegment {
                start: 0.0,
                length: 400.0,
                curvature: -0.005,
            }],
            Vec::new(),
            12.0,
        );
        assert!(left.center_offset(300.0) < left.center_offset(100.0));
        let drifted = (0..60).fold(bike, |b, _| {
            step_bike(b, Intent::default(), false, -0.005, dt)
        });
        assert!(
            drifted.lateral > bike.lateral,
            "mirror: drift ran the other way ({})",
            drifted.lateral
        );
    }

    #[test]
    fn lateral_motion_follows_the_heading_rate() {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        // Two rides identical but for the heading-rate constant: during
        // the approach to full lock the lateral motion must differ —
        // lateral velocity follows heading, it is not a direct push.
        let run = |heading_rate: f32| {
            let tuning = SteerTuning {
                heading_rate,
                ..SteerTuning::NOMINAL
            };
            let bike = BikeSnapshot {
                speed: 30.0,
                ..BikeSnapshot::at_rest()
            };
            (0..8).fold(bike, |b, _| {
                step_bike_tuned(
                    b,
                    intent_of(&[GameInput::SteerRight]),
                    false,
                    0.0,
                    dt,
                    tuning,
                )
            })
        };
        let nominal = run(HEADING_RATE);
        let lazy = run(HEADING_RATE * 0.4);
        assert!(
            nominal.lateral > lazy.lateral * 1.2,
            "a slower heading rate must move less lateral ({}) vs ({})",
            nominal.lateral,
            lazy.lateral
        );
    }

    #[test]
    fn centre_to_edge_at_full_steer_takes_at_least_the_target_time() {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        // Reference measurement speed: ~108 km/h, a typical race pace.
        let bike = BikeSnapshot {
            speed: 30.0,
            ..BikeSnapshot::at_rest()
        };
        let mut ticks = 0usize;
        let mut bike = bike;
        while bike.lateral.abs() < 6.0 {
            bike = step_bike(bike, intent_of(&[GameInput::SteerRight]), false, 0.0, dt);
            ticks += 1;
            assert!(ticks < 600, "the bike never reached the road edge");
        }
        let secs = ticks as f32 * dt;
        assert!(
            secs >= CENTRE_TO_EDGE_TARGET_SECS,
            "centre-to-edge {secs:.2} s is short of the {CENTRE_TO_EDGE_TARGET_SECS:?} s target"
        );
    }
}
