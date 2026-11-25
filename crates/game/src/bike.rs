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
/// bend always beats the road with margin. Small on purpose: a hands-off
/// rider must be able to hold a bend (the audit's recovery probe rides 45 s
/// on throttle alone), so the drift is the residual a rider leans against —
/// not a force field that wins.
pub const CORNER_DRIFT_FRACTION: f32 = 0.05;
/// Visual bank into a corner, as a fraction of [`LEAN_MAX`]: how far the
/// rider leans for the cornering force at full lock, independent of the
/// drift fraction above (the bank is presentation; the drift is physics).
pub const CORNER_LEAN_FRACTION: f32 = 0.6;
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
/// Depth past the road edge, in meters, beyond which an off-road ride is a
/// crash at any speed. Below [`OFFROAD_CRASH_SPEED`] the dirt is rideable,
/// but nothing bounds the excursion: a hairpin overshoot at 18–30 m/s could
/// carry the rider arbitrarily far off-world. Past this limit the sim calls
/// it a crash, and recovery pulls the rider back to the tarmac — braking
/// stays the skill way through a hairpin, but a flat-out overshoot never
/// leaves the player lost off-world.
pub const OFFROAD_EXCURSION_LIMIT_M: f32 = 5.0;

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
/// bike (1/meters; RoadGeometry is authoritative — negative curvature is a
/// left bend and the centreline shifts negative): speed throws the bike
/// outward, capped by the grip limit — steering into a bend always beats
/// the road.
pub fn step_bike(
    bike: BikeSnapshot,
    intent: Intent,
    off_road: bool,
    corner_curvature: f32,
    dt: f32,
) -> BikeSnapshot {
    step_bike_tuned(
        bike,
        intent,
        off_road,
        corner_curvature,
        dt,
        SteerTuning::NOMINAL,
    )
}

/// [`step_bike`] with explicit steering feel — the seam that lets a test
/// vary only the heading rate and watch lateral motion follow.
pub fn step_bike_tuned(
    mut bike: BikeSnapshot,
    intent: Intent,
    off_road: bool,
    corner_curvature: f32,
    dt: f32,
    tuning: SteerTuning,
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

    // Corner drift under the grip limit. RoadGeometry is authoritative: a
    // bend's centreline shifts in the same direction as its curvature sign
    // (negative curvature is a left bend and the centre moves to negative
    // lateral), so the outside of a bend lies OPPOSITE the centreline's
    // change — and that is where speed throws the bike: the drift runs
    // against the centreline's shift, by at most CORNER_DRIFT_FRACTION of
    // what full steering can pull back at this speed. A bend is a line you
    // hold by leaning in earlier or shedding speed, never a force field
    // that wins.
    let drift_authority = CORNER_DRIFT_FRACTION * bike.speed * tuning.full_lock.tan() * steer_scale;
    let corner_drift = -(bike.speed * bike.speed * corner_curvature * CORNER_FORCE)
        .clamp(-drift_authority, drift_authority);

    // Heading drives the line (the Wave 7 model): steering commands a
    // heading up to `full_lock` radians off the centreline, the velocity
    // vector rotates toward it rate-limited at `heading_rate`, and lateral
    // velocity is the across-road component of speed — lateral motion
    // FOLLOWS heading, it is not a parallel sideways shove. With no input
    // the command returns to zero and the bike straightens at the same
    // rate; corner drift yaws the bike outward with it.
    let commanded = steer_dir * tuning.full_lock * steer_ramp * steer_scale;
    bike.heading +=
        (commanded - bike.heading).clamp(-tuning.heading_rate * dt, tuning.heading_rate * dt);
    let lateral_velocity = bike.speed * bike.heading.tan() + corner_drift;
    bike.lateral += lateral_velocity * dt;

    // Visual bank: aim at the steering input plus the corner's inward lean,
    // rate-limited so the bank reads as a motion, not a flip. The steering
    // term is the player's heading change — the rider banks with where the
    // bike is pointed. The corner term comes second, capped at the grip
    // limit: a rider coasting through a bend banks into it by at most the
    // drift fraction. Negative curvature is a left bend (RoadGeometry is
    // authoritative), and into a left bend is a leftward (negative) lean.
    let steer_lean = steer_ramp
        * steer_scale
        * match intent.steer {
            Steer::Left => -1.0,
            Steer::Right => 1.0,
            Steer::None => 0.0,
        };
    let corner_lean = corner_curvature.signum()
        * ((bike.speed * bike.speed * corner_curvature.abs() * CORNER_FORCE)
            / (bike.speed * tuning.full_lock.tan()).max(1.0))
        .clamp(0.0, CORNER_LEAN_FRACTION);
    let target_lean = (steer_lean + corner_lean).clamp(-1.0, 1.0) * LEAN_MAX;
    bike.lean += (target_lean - bike.lean).clamp(-LEAN_RATE * dt, LEAN_RATE * dt);

    bike
}

/// The crash trigger, evaluated by the race driver after the lateral move:
/// running off the tarmac above [`OFFROAD_CRASH_SPEED`] is a crash, and so
/// is riding deeper than [`OFFROAD_EXCURSION_LIMIT_M`] past the edge at any
/// speed — the depth bound that keeps every excursion recoverable.
pub fn should_crash(bike: &BikeSnapshot, half_width: f32) -> bool {
    if bike.control == Control::Crashed || !bike.is_off_road(half_width) {
        return false;
    }
    bike.speed > OFFROAD_CRASH_SPEED || bike.lateral.abs() > half_width + OFFROAD_EXCURSION_LIMIT_M
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
        // Right-hand curve (positive curvature; RoadGeometry shifts the
        // centre positive) throws a fast bike left — outward, opposite the
        // centreline's change.
        let fast = at_speed(50.0, 0.005);
        assert!(fast.lateral < 0.0, "right curve throws the bike left");
        // The same curve at walking pace barely moves the bike.
        let slow = at_speed(5.0, 0.005);
        assert!(
            slow.lateral.abs() < fast.lateral.abs() * 0.1,
            "slow is stable"
        );
        // Mirror symmetry: a left-hand curve pushes right.
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
        // Maximum-curvature bend (k = -0.005, a left-hand bend: the inside
        // is negative lateral). Full left steer held through it must keep
        // the bike moving toward the inside at every speed — the road never
        // wins.
        for speed in [5.0, 8.0, 16.0, 30.0, 45.0, MAX_SPEED] {
            let bike = BikeSnapshot {
                speed,
                ..BikeSnapshot::at_rest()
            };
            let held = (0..14).fold(bike, |bike, _| {
                step_bike(bike, intent_of(&[GameInput::SteerLeft]), false, -0.005, dt)
            });
            let next = step_bike(held, intent_of(&[GameInput::SteerLeft]), false, -0.005, dt);
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
        // Flat out through the maximum bend (k = +0.005, a right-hand
        // bend), hands off: the grip limit caps the outward drift —
        // negative lateral, opposite the centreline's positive shift — at
        // 60% of what full steering could pull back.
        let bike = BikeSnapshot {
            speed: MAX_SPEED,
            ..BikeSnapshot::at_rest()
        };
        let drifted = step_bike(bike, Intent::default(), false, 0.005, dt);
        let drift = drifted.lateral - bike.lateral;
        let authority = CORNER_DRIFT_FRACTION * MAX_SPEED * FULL_LOCK_HEADING.tan() * dt;
        assert!(
            drift < 0.0 && drift >= -authority - 1e-6,
            "grip limit caps drift at the fraction of steer authority ({drift})"
        );
    }

    #[test]
    fn steering_ramps_in_and_bites_when_held() {
        let rolling = BikeSnapshot {
            speed: 30.0,
            ..BikeSnapshot::at_rest()
        };
        // The first tick's lateral motion is the ramped heading command
        // turned into across-road speed — a tap nudges, it does not snap.
        // The coast tick scrubs a little speed first; match it.
        let first = stepped(rolling, &[GameInput::SteerRight], false);
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        let ramp = (dt / STEER_RAMP_SECS).min(1.0);
        let coasted = 30.0 - COAST_DRAG * dt;
        let expected = coasted * (FULL_LOCK_HEADING * ramp).tan() * dt;
        assert!((first.lateral - expected).abs() < 1e-6, "ramped first tick");

        // Held long enough, the ramp is full: per-tick motion reaches the
        // full-lock across-road speed, speed · tan(full lock).
        let held = (0..12).fold(rolling, |bike, _| {
            stepped(bike, &[GameInput::SteerRight], false)
        });
        let next = stepped(held, &[GameInput::SteerRight], false);
        let coasted = held.speed - COAST_DRAG * dt;
        let expected = coasted * FULL_LOCK_HEADING.tan() * dt;
        assert!(
            (next.lateral - held.lateral - expected).abs() < 1e-4,
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
        // Positive curvature is a right bend (RoadGeometry: the centre
        // shifts positive); the rider banks right — into it — with no input.
        let banked = step_bike(fast, Intent::default(), false, 0.005, dt);
        assert!(banked.lean > 0.05, "corner leans the rider inward");
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

    #[test]
    fn a_deep_excursion_crashes_at_dirt_speeds_where_the_speed_cap_left_it_unbounded() {
        let half_width = 6.0;
        // The 18-30 m/s window: the dirt is rideable, so a shallow
        // excursion survives...
        let shallow = BikeSnapshot {
            speed: 25.0,
            lateral: half_width + 2.0,
            ..BikeSnapshot::at_rest()
        };
        assert!(
            !should_crash(&shallow, half_width),
            "a shallow dirt ride below the crash speed stays legal"
        );
        // ...but the excursion is bounded: deeper than the limit at any
        // speed is a crash, so recovery pulls the rider back to the tarmac
        // instead of leaving them lost off-world (the hairpin overshoot
        // used to run the lateral unbounded through this window).
        let deep = BikeSnapshot {
            speed: 25.0,
            lateral: half_width + OFFROAD_EXCURSION_LIMIT_M + 0.5,
            ..BikeSnapshot::at_rest()
        };
        assert!(
            should_crash(&deep, half_width),
            "a deep excursion must crash at dirt speed"
        );
        // The depth bound is not a speed rule: it applies at walking pace.
        let slow_deep = BikeSnapshot { speed: 5.0, ..deep };
        assert!(should_crash(&slow_deep, half_width));
        // And an already-crashed bike does not re-trigger.
        let mut down = deep;
        crash(&mut down);
        assert!(
            !should_crash(&down, half_width),
            "a crashed bike cannot re-crash"
        );
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
        // Two rides identical but for the heading-rate constant, already
        // holding full ramp: during the approach to full lock the lateral
        // motion must differ — lateral velocity follows heading, it is not
        // a direct push.
        let run = |heading_rate: f32| {
            let tuning = SteerTuning {
                heading_rate,
                ..SteerTuning::NOMINAL
            };
            let bike = BikeSnapshot {
                speed: 30.0,
                steer_hold: STEER_RAMP_SECS,
                ..BikeSnapshot::at_rest()
            };
            (0..20).fold(bike, |b, _| {
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

    // ------------------------------------------------------------------
    // Wave 8.2 spec — drivetrain (.obvious/specs/wave8-drivetrain.md).
    // Red-first: these describe the gearbox the tick must gain; they
    // fail on the current single-ratio thrust until it ships.
    // ------------------------------------------------------------------

    /// Full-throttle run from rest: speed after each of `ticks` ticks at
    /// the 60 Hz sim rate.
    fn full_throttle_run(ticks: usize) -> Vec<f32> {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        let mut bike = BikeSnapshot::at_rest();
        let mut speeds = vec![bike.speed];
        for _ in 0..ticks {
            bike = step_bike(bike, intent_of(&[GameInput::Accelerate]), false, 0.0, dt);
            speeds.push(bike.speed);
        }
        speeds
    }

    /// Per-tick acceleration along a run, in m/s².
    fn accel_profile(speeds: &[f32]) -> Vec<f32> {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        speeds.windows(2).map(|w| (w[1] - w[0]) / dt).collect()
    }

    /// Speed at each gear shift the profile shows. A shift is a sharp
    /// recovery: the pull sags at least 25% off its running peak through
    /// the top of a rev band, then jumps at least 25% above the sag
    /// floor. The single-ratio thrust only ever tapers, so it detects
    /// nothing.
    fn shift_speeds(speeds: &[f32], accel: &[f32]) -> Vec<f32> {
        const SAG_FRACTION: f32 = 0.25;
        const RECOVERY_FRACTION: f32 = 1.25;
        let mut shifts = Vec::new();
        let mut band_peak = accel.first().copied().unwrap_or(0.0);
        let mut sag_floor = f32::INFINITY;
        let mut sagged = false;
        for (i, &a) in accel.iter().enumerate() {
            band_peak = band_peak.max(a);
            if !sagged {
                if a < band_peak * (1.0 - SAG_FRACTION) {
                    sagged = true;
                    sag_floor = a;
                }
            } else {
                sag_floor = sag_floor.min(a);
                if a > sag_floor * RECOVERY_FRACTION {
                    shifts.push(speeds[i + 1]);
                    band_peak = a;
                    sagged = false;
                }
            }
        }
        shifts
    }

    #[test]
    fn full_throttle_acceleration_rises_through_distinct_gear_bands() {
        // 20 s of held throttle: past every shift and into the tail.
        let speeds = full_throttle_run(1_200);
        let shifts = shift_speeds(&speeds, &accel_profile(&speeds));
        assert!(
            shifts.len() >= 2,
            "a multi-gear drivetrain recovers the pull at each shift; the \
             single-ratio thrust only tapers (shifts at {shifts:?})"
        );
    }

    #[test]
    fn gear_shifts_occur_at_speed_boundaries_not_at_the_extremes() {
        let speeds = full_throttle_run(1_200);
        let shifts = shift_speeds(&speeds, &accel_profile(&speeds));
        assert!(
            !shifts.is_empty(),
            "the gearbox shifts up through its rev bands; nothing shifts \
             on the single-ratio thrust"
        );
        for &speed in &shifts {
            assert!(
                (12.0..=50.0).contains(&speed),
                "shift at {speed} m/s is outside the road range: no shift \
                 at a crawl, and top gear owns the top end"
            );
        }
    }

    #[test]
    fn full_throttle_reaches_100_kmh_within_the_gearbox_launch_target() {
        let dt = sim::clock::TICK_DURATION_SECS as f32;
        let speeds = full_throttle_run(600);
        let hit = speeds
            .iter()
            .position(|&s| s >= 100.0 / 3.6)
            .expect("a full-throttle run reaches 100 km/h");
        let secs = hit as f32 * dt;
        assert!(
            secs < 2.2,
            "0-100 km/h in {secs:.2} s misses the 2.2 s gearbox launch \
             target — first gear must out-pull the single-ratio launch"
        );
    }
}
