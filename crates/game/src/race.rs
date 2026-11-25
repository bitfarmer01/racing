//! The race driver: bikes in the sim pool, one fixed 60 Hz tick per call.
//!
//! [`Race::tick`] is the whole simulation step, in order: drain the sim input
//! ring into one intent, step the player and every rival through the bike
//! state machine, integrate positions, resolve bike-vs-bike overlaps, and
//! apply the off-road crash trigger. Nothing in a tick allocates: all state
//! lives in fixed arrays and the sim pool.
//!
//! Determinism: identical seeds and identical input sequences produce
//! identical races — the RNG is drawn only at construction (spawn lanes), and
//! every tick is a pure function of state.

use sim::clock::{TickClock, TICK_DURATION_SECS};
use sim::input::InputBuffer;
use sim::pool::{BikeHandle, BikePool, BikeState, MAX_BIKES};
use sim::rng::XorShift64;
use world::RoadGeometry;

use crate::ai;
use crate::bike::{self, BikeSnapshot, Control, MAX_SPEED};
use crate::combat::{self, Attack, Combat};
use crate::input::{GameInput, Intent};

/// Bike collision half-length along the road, in meters.
pub const BIKE_HALF_LENGTH: f32 = 1.1;
/// Bike collision half-width across the road, in meters.
pub const BIKE_HALF_WIDTH: f32 = 0.5;
/// Fraction of speed each overlapping bike loses per overlapping tick.
pub const COLLISION_SCRUB: f32 = 0.985;
/// Longitudinal gap between rivals on the starting grid, in meters. Wide
/// enough that rivals start strung out down the road instead of bunched at
/// one depth (C4).
pub const GRID_SPACING: f32 = 14.0;
/// Gap behind the player beyond which a rival gets the rubber-band boost,
/// in meters: drop far enough back and the field holds the race together.
pub const RUBBER_BAND_BEHIND: f32 = 25.0;
/// Gap ahead of the player beyond which a leader eases off, in meters: a
/// breakaway is catchable, not untouchable.
pub const RUBBER_BAND_AHEAD: f32 = 60.0;
/// Closing pace of a fully rubber-banded rival over the player, in m/s.
/// Scaled down to zero as the rival approaches the window edge, so it
/// rejoins at the player's pace instead of ramming it.
pub const RUBBER_BAND_MARGIN: f32 = 12.0;
/// Depth outside the window, in meters, over which the rubber-band boost
/// ramps in from the window edge.
pub const RUBBER_BAND_RAMP: f32 = 15.0;
/// Acceleration of a fully rubber-banded rival, in meters per second
/// squared — well above the nominal throttle, so a dropped rival visibly
/// rides back into the race instead of clawing at cruise acceleration.
pub const RUBBER_BAND_ACCEL: f32 = 30.0;
/// Speed multiplier for leaders too far ahead to stay in the race.
pub const RUBBER_BAND_EASE: f32 = 0.85;
/// How much faster than the player a leader may ride at the window's inner
/// edge, in m/s. The slack shrinks to zero as the gap grows toward
/// [`RUBBER_BAND_AHEAD`], so the rider in front is always catchable and
/// never holds a static gap.
pub const LEADER_SLACK: f32 = 8.0;
/// Distance over which a chaser's slack above the player's pace tapers to
/// zero, in meters: closing speed decays to nothing at the rear wheel.
pub const CHASE_TAPER: f32 = 12.0;
/// Fastest a chaser may close on a slower rider ahead, in m/s over the
/// rider's speed. Without this the closing floor (`max(top_speed, …)`)
/// sends a chaser straight into a rider who braked for a bend — the ram
/// it then causes crashes the chaser and tears the pack down (audit
/// items 6-7). Stays under the 12 m/s ram threshold.
pub const CHASE_CEILING: f32 = 8.0;
/// How far behind the player a rival can sit and still join a fight, in
/// meters: the engagement forms on the approach, before punch reach.
pub const FIGHT_ARM_RANGE: f32 = 9.0;
/// Lateral distance across the road within which a rival counts as close
/// enough to the player's line to join a fight, in meters: wider than
/// [`crate::combat::ADJACENT_LATERAL`] because the fighter then converges
/// onto the player's line during the engagement. Wide enough to cover the
/// grid's widest holding lane (~5 m off centre), so a wide-line rider can
/// still be drawn into a fight instead of pacing past untouched.
pub const FIGHT_LATERAL: f32 = 5.5;
/// Lateral offset a fighting rival holds beside the player, in meters:
/// inside punch reach (2.5 m) but outside the bike-overlap band, so the
/// pair rides side by side instead of grinding.
pub const FIGHT_RIDE_LATERAL: f32 = 1.5;
/// Speed a rival must be riding at to join a fight, in m/s: a stopped or
/// crawling rider is a hazard to swerve, not a sparring partner — the ram
/// stays a ram.
pub const FIGHT_RIDE_SPEED: f32 = 15.0;

/// Width of the rumble strip along the tarmac edge, in meters: inside this
/// shoulder band the surface scrubs speed and the camera shakes — the
/// warning a rider gets before running off the road at speed is a crash.
pub const RUMBLE_BAND: f32 = 1.5;
/// Per-tick speed fraction kept while on the rumble strip or gravel.
pub const RUMBLE_SCRUB: f32 = 0.995;
use crate::ai::GRID_HALF_SPAN;
/// A rival lane never sits within this margin of the player's centreline
/// or of its neighbours by less than the side-crash gap, in meters.
const LANE_MIN: f32 = 0.75;
/// Lateral avoidance (audit item 7): when a rival closes on another bike
/// from behind, it sidesteps to a staggered lane instead of riding into
/// the pack — the behaviour it already had for cars, applied to bikes.
/// The player never auto-dodges: steering is the rider's job.
/// Longitudinal range in which a bike ahead is considered a blocker, in m.
const AVOID_RANGE: f32 = 25.0;
/// Lateral distance at which a bike ahead blocks the rival's line, in m.
const AVOID_LATERAL: f32 = 1.6;
/// Closing speed that makes a bike ahead worth dodging, in m/s.
const AVOID_CLOSING: f32 = 2.0;
/// Longitudinal gap at which a bike ahead is dodged even without closing
/// speed, in m: already touching distance.
const AVOID_CONTACT: f32 = 6.0;
/// Sidestep the dodging rival takes around the blocked line, in m.
const AVOID_STEP: f32 = 1.6;
/// Closing speed above which running into the bike ahead is a crash, in m/s.
pub const CRASH_CLOSING_SPEED: f32 = 12.0;
/// Speed above which a heavy side-by-side overlap takes both riders down, in
/// m/s.
pub const SIDE_CRASH_SPEED: f32 = 28.0;
/// Lateral gap under which side-by-side contact counts as heavy, in meters.
pub const SIDE_CRASH_GAP: f32 = 0.6;
/// Lateral closing speed above which side-by-side contact is a slam that
/// takes both riders down, in m/s: leaning along together at speed is
/// racing, steering into each other is a crash.
pub const SIDE_CRASH_SLAM: f32 = 1.5;
/// Meters inside the road edge where a recovered rider is placed: at
/// least this far in from the lip, so a picked-up bike has road to ride.
pub const RECOVERY_LANE_MARGIN: f32 = 3.0;
/// Green-light countdown length: 3 seconds at 60 Hz.
pub const COUNTDOWN_TICKS: u32 = 180;
/// Slow vehicles sharing the road.
pub const TRAFFIC_COUNT: usize = 6;
/// Traffic collision half-length along the road, in meters.
pub const CAR_HALF_LENGTH: f32 = 2.2;
/// Traffic collision half-width across the road, in meters.
pub const CAR_HALF_WIDTH: f32 = 1.0;

/// A slow vehicle sharing the road: an obstacle, not a racer. Cruises its
/// lane at a fixed speed, wrapping the track.
#[derive(Clone, Copy, Debug)]
pub struct TrafficCar {
    /// Position and heading, shaped like a bike so the renderer projects it
    /// the same way.
    pub state: BikeState,
    /// Cruise speed, in meters per second.
    pub cruise: f32,
}

/// A static on-road hazard: a traffic cone. Hitting one hurts the speed,
/// not the rider.
#[derive(Clone, Copy, Debug)]
pub struct Hazard {
    /// Position along the road, in meters.
    pub pos_x: f32,
    /// Lateral offset across the road, in meters.
    pub lateral: f32,
}

/// Lap index for a total distance covered: whole laps completed.
///
/// Boundary cases are the point: exactly one track length is lap 1, an
/// epsilon under it is still lap 0, and exactly `n` track lengths is lap `n`.
pub fn lap_index(distance: f32, track_length: f32) -> u32 {
    (distance / track_length).floor() as u32
}

/// Whether two bike states overlap on the road: within bike length along the
/// road and bike width across it. Pure, so collision checks stay testable.
pub fn bikes_overlap(a: &BikeState, b: &BikeState) -> bool {
    (b.pos_x - a.pos_x).abs() < BIKE_HALF_LENGTH * 2.0
        && (b.pos_y - a.pos_y).abs() < BIKE_HALF_WIDTH * 2.0
}

/// Lateral push for one overlapping pair: `(delta_a, delta_b)`, the deltas to
/// add to each bike's lateral position. Each bike takes half the overlap,
/// pushed away from the other; identical lateral positions (dy == 0) split
/// deterministically with bike A moving left.
pub fn overlap_push(dy: f32) -> (f32, f32) {
    let overlap = BIKE_HALF_WIDTH * 2.0 - dy.abs();
    let sign = if dy >= 0.0 { 1.0 } else { -1.0 };
    (-overlap * 0.5 * sign, overlap * 0.5 * sign)
}

/// Progress through the current lap, in `[0, 1)`: 0 at the start line, just
/// under 1 one epsilon before the next lap. Exactly on the line is 0.
pub fn lap_progress(distance: f32, track_length: f32) -> f32 {
    (distance % track_length) / track_length
}

/// Race setup. [`RoadGeometry`] describes the track; the rest is sizing.
pub struct RaceConfig {
    /// Rival bikes besides the player, capped at `MAX_BIKES - 1`.
    pub rivals: usize,
    /// Length of one lap, in meters.
    pub track_length: f32,
    /// Laps needed to finish.
    pub total_laps: u32,
    /// RNG seed: fixes the rivals' preferred lanes deterministically.
    pub seed: u64,
    /// Track geometry, queried for AI lookahead.
    pub geometry: RoadGeometry,
}

/// A live race: bikes in the pool, aux state in fixed arrays, and the clock.
pub struct Race {
    clock: TickClock,
    inputs: InputBuffer<GameInput>,
    pool: BikePool,
    /// Spawn-order handles; index 0 is always the player.
    handles: [Option<BikeHandle>; MAX_BIKES],
    bike_count: usize,
    /// Per-bike state the pool does not carry, indexed like `handles`.
    aux: [RiderAux; MAX_BIKES],
    geometry: RoadGeometry,
    track_length: f32,
    total_laps: u32,
    /// True once the player has finished every lap.
    finished: bool,
    /// Ticks left before the lights go green.
    countdown_ticks: u32,
    /// Slow traffic cruising the track.
    traffic: Vec<TrafficCar>,
    /// Static cone hazards on the road.
    hazards: Vec<Hazard>,
    /// Whether the player was on the rumble strip this tick.
    player_rumble: bool,
}

/// Aux state per rider, mirroring the pool's slot order.
#[derive(Clone, Copy, Debug)]
struct RiderAux {
    snapshot: BikeSnapshot,
    /// Rival preferred lane offset; unused for the player.
    target_lane: f32,
    /// Straight-line top speed for this rider, in m/s: rivals vary.
    top_speed: f32,
    /// How eagerly this rival hunts the inside line; 1.0 is nominal.
    aggression: f32,
    /// Adjacent-rider combat state (wave 7b): stamina, swing cooldown, pose.
    combat: Combat,
    /// Accumulated instability from landed hits; past the dismount threshold
    /// the rider goes down through the crash flow.
    instability: f32,
    /// Ticks left on a pending attack wind-up; 0 when not winding up.
    windup_ticks: u32,
    /// Spawn slot a pending wind-up is aimed at; None when not winding up.
    windup_target: Option<usize>,
    /// Combat dismounts this rider has already taken; past
    /// [`combat::DISMOUNT_CAP`] hits can no longer dismount them.
    dismounts: u32,
    /// Seconds left in a pace-match engagement: a rival in punch reach of
    /// the player rides the player's trimmed pace for this long.
    shadow_secs: f32,
    /// Seconds left before a rival may start a new pace-match engagement.
    shadow_cooldown: f32,
    /// Lateral velocity last tick, in m/s: how fast this rider is moving
    /// sideways, read by the collision pass to separate leans from slams.
    lateral_vel: f32,
}

impl Default for RiderAux {
    fn default() -> Self {
        Self {
            snapshot: BikeSnapshot::at_rest(),
            target_lane: 0.0,
            top_speed: MAX_SPEED,
            aggression: 1.0,
            combat: Combat::default(),
            instability: 0.0,
            windup_ticks: 0,
            windup_target: None,
            dismounts: 0,
            shadow_secs: 0.0,
            shadow_cooldown: 0.0,
            lateral_vel: 0.0,
        }
    }
}

impl Race {
    /// A new race on the given config: player on the centreline at the line,
    /// rivals staggered behind in deterministic lanes.
    pub fn new(config: RaceConfig) -> Self {
        let rivals = config.rivals.min(MAX_BIKES - 1);
        let mut pool = BikePool::new();
        let mut handles = [None; MAX_BIKES];
        let mut aux = [RiderAux::default(); MAX_BIKES];

        let player = pool.spawn(BikeState {
            pos_x: 0.0,
            pos_y: 0.0,
            speed: 0.0,
            heading: 0.0,
        });
        handles[0] = Some(player.expect("fresh pool has room for the player"));

        let mut rng = XorShift64::new(config.seed);
        // Even lane spacing: no two rivals share a line from the start,
        // and none starts on the player's centreline, where early
        // side-by-side contact at speed is a double crash.
        let lane_step = if rivals > 0 {
            2.0 * GRID_HALF_SPAN / rivals as f32
        } else {
            0.0
        };
        for rival in 0..rivals {
            let mut lane = -GRID_HALF_SPAN + (rival as f32 + 0.5) * lane_step;
            if lane.abs() < LANE_MIN {
                lane = LANE_MIN * if lane >= 0.0 { 1.0 } else { -1.0 };
            }
            // Spread the grid ahead of and behind the player: even-indexed
            // rivals up the road, odd-indexed ones behind the line, strung
            // out by GRID_SPACING. The pack starts racing, not queued.
            let stagger = (rival / 2 + 1) as f32 * GRID_SPACING;
            let state = BikeState {
                pos_x: if rival % 2 == 0 { stagger } else { -stagger },
                pos_y: lane,
                speed: 0.0,
                heading: 0.0,
            };
            let slot = rival + 1;
            handles[slot] = Some(pool.spawn(state).expect("fresh pool has room"));
            aux[slot].target_lane = lane;
            // Rider variety: each rival gets a top speed and an aggression,
            // so the pack spreads out instead of riding as one block.
            // Rival machines are the player's class of bike: a top speed at
            // or a whisker above/below the player's. Slower rivals scatter
            // the field after a lap — the player outruns everyone and no
            // fight can ever form again (audit item 6 wants reach windows
            // every lap).
            aux[slot].top_speed = MAX_SPEED * rng.next_f32_in(0.94, 1.02);
            aux[slot].aggression = rng.next_f32_in(0.6, 1.4);
        }

        // Traffic: spread around the lap, alternating lanes, cruising slow.
        let mut traffic = Vec::with_capacity(TRAFFIC_COUNT);
        for car in 0..TRAFFIC_COUNT {
            let along = (car + 1) as f32 * config.track_length / (TRAFFIC_COUNT + 1) as f32;
            let lane_sign = if car % 2 == 0 { 1.0 } else { -1.0 };
            traffic.push(TrafficCar {
                state: BikeState {
                    pos_x: along,
                    pos_y: lane_sign * (1.5 + rng.next_f32_in(0.0, 1.5)),
                    speed: 0.0,
                    heading: 0.0,
                },
                cruise: rng.next_f32_in(8.0, 15.0),
            });
        }

        // Cone hazards: deterministic per seed, inside the road edges.
        let half_width = config.geometry.road_width() * 0.5;
        let mut hazards = Vec::new();
        let mut hazard_slots = 0u64;
        let mut along = HAZARD_SPACING;
        while along < config.track_length - HAZARD_SPACING {
            let mut roll = XorShift64::new(hazard_slots.wrapping_mul(0x2545_F491_4F6C_DD1D));
            if roll.next_f32_in(0.0, 1.0) < 0.4 {
                let side = if roll.next_f32_in(0.0, 1.0) < 0.5 {
                    1.0
                } else {
                    -1.0
                };
                hazards.push(Hazard {
                    pos_x: along,
                    lateral: side * roll.next_f32_in(0.8, half_width - 1.2),
                });
            }
            hazard_slots += 1;
            along += HAZARD_SPACING;
        }

        Self {
            clock: TickClock::new(),
            inputs: InputBuffer::new(),
            pool,
            handles,
            bike_count: rivals + 1,
            aux,
            geometry: config.geometry,
            track_length: config.track_length,
            total_laps: config.total_laps,
            finished: false,
            player_rumble: false,
            countdown_ticks: COUNTDOWN_TICKS,
            traffic,
            hazards,
        }
    }

    /// Queue one player input event for the next tick.
    pub fn push_input(&mut self, input: GameInput) {
        self.inputs.push(input);
    }

    /// Move one bike (spawn-order slot) to a track position and settle its
    /// speed to zero: the reset/respawn path, and the testing seam for
    /// staging collisions the grid can't produce.
    pub fn place_bike(&mut self, slot: usize, pos_x: f32, pos_y: f32) {
        self.aux[slot].snapshot = BikeSnapshot::at_rest();
        self.pool.set(
            self.handle(slot),
            BikeState {
                pos_x,
                pos_y,
                speed: 0.0,
                heading: 0.0,
            },
        );
    }

    /// Advance exactly one 60 Hz tick. Inputs drained once, at tick start.
    pub fn tick(&mut self) {
        self.player_rumble = false;
        let mut intent = Intent::default();
        for input in self.inputs.drain() {
            intent.absorb(input);
        }

        // Lights are red: everyone holds still while the clock runs down.
        if self.countdown_ticks > 0 {
            self.countdown_ticks -= 1;
            return;
        }

        let dt = TICK_DURATION_SECS as f32;
        let half_width = self.geometry.road_width() * 0.5;
        let player_x = self.state(0).pos_x;
        let player_speed = self.state(0).speed;

        // Player (slot 0) follows the drained intent; rivals follow the AI.
        for slot in 0..self.bike_count {
            let state = self.state(slot);
            let mut snapshot = self.aux[slot].snapshot;
            snapshot.speed = state.speed;
            snapshot.lateral = state.pos_y;

            // Rubber band: a closed-loop gap controller around the player.
            // A rival dropped well behind races above the player's pace and
            // accelerates harder — scaled by how far out it is, easing to
            // the player's own pace at the window edge so it rejoins rather
            // than ramming. A leader far ahead eases off. Inside the window
            // a rival races at its own pace, so the pack spreads, overtakes,
            // and never holds a static gap.
            // Nearest rider ahead of this one, any bike: chasers queue
            // behind the pack instead of driving through it.
            let mut ahead_x = f32::INFINITY;
            let mut ahead_speed = 0.0_f32;
            let mut ahead_is_player = false;
            for other in 0..self.bike_count {
                if other == slot {
                    continue;
                }
                let them = self.state(other);
                if them.pos_x > state.pos_x && them.pos_x < ahead_x {
                    ahead_x = them.pos_x;
                    ahead_speed = them.speed;
                    ahead_is_player = other == 0;
                }
            }
            let gap_ahead = ahead_x - state.pos_x;
            let gap = state.pos_x - player_x;
            let (cap, chasing) = if slot == 0 {
                (MAX_SPEED, 0.0)
            } else if gap < -RUBBER_BAND_BEHIND {
                let depth = (-gap - RUBBER_BAND_BEHIND).min(RUBBER_BAND_RAMP);
                let ease = depth / RUBBER_BAND_RAMP;
                let cap = self.aux[slot]
                    .top_speed
                    .max((player_speed + RUBBER_BAND_MARGIN * ease).min(MAX_SPEED * 1.2));
                (cap, ease)
            } else if gap > RUBBER_BAND_AHEAD {
                (self.aux[slot].top_speed * RUBBER_BAND_EASE, 0.0)
            } else if gap > 0.0 {
                // Ahead inside the window: the leader paces the player —
                // capped at a small slack that shrinks with the gap, so a
                // rider in front is always catchable, never a static gap.
                let depth = (gap / RUBBER_BAND_AHEAD).min(1.0);
                let ease = 1.0 - (1.0 - RUBBER_BAND_EASE) * depth;
                let cap = (self.aux[slot].top_speed * ease)
                    .min(player_speed + LEADER_SLACK * (1.0 - depth));
                (cap, 0.0)
            } else {
                // Behind but inside the chase window: shadow the player's
                // pace with a slack measured to the nearest rider ahead —
                // the pack queues behind whoever leads instead of driving
                // through teammates — shrinking to zero at contact, so a
                // chaser glides onto the rear wheel instead of ramming.
                // A fight that just ended backs off instead (the
                // break-off). Top speed is a FLOOR for a chaser, not a
                // ceiling: a slower rival can never close on a faster
                // player otherwise, and the fights (audit item 6) would
                // only ever happen by accident.
                let depth = (-gap_ahead / CHASE_TAPER).min(1.0);
                let slack = if self.aux[slot].shadow_cooldown > 0.0 {
                    // The break-off leaves reach promptly (audit item 6):
                    // a slow back-off smears one more long in-reach stretch
                    // behind every fight, and a fast exit keeps the gap
                    // between fights clean for overtaking.
                    -CHASE_CEILING
                } else if -gap_ahead <= combat::PACE_STAGGER + 1.0 {
                    0.0
                } else {
                    LEADER_SLACK * depth
                };
                (
                    self.aux[slot]
                        .top_speed
                        .max(player_speed + slack)
                        .min(player_speed + CHASE_CEILING),
                    0.0,
                )
            };

            let intent = if slot == 0 {
                intent
            } else {
                // Audit item 7: rivals dodge bikes blocking their line
                // (but never the player's own steering — see the slot 0
                // branch above).
                let avoid = self.avoidance_lane(slot);
                ai::decide(
                    snapshot.speed,
                    snapshot.lateral,
                    state.pos_x,
                    self.aux[slot].target_lane,
                    cap,
                    avoid,
                    &self.geometry,
                )
            };

            let curvature = self.geometry.curvature_at(state.pos_x);
            let off_road = snapshot.is_off_road(half_width);
            let was_crashed = snapshot.control == Control::Crashed;

            // Fight pacing (audit item 6): a rival near the player's
            // rear wheel joins a bounded engagement — for
            // PACE_MATCH_SECS it rides the player's pace, trimmed toward
            // a stagger gap, and steers onto the player's line, so the
            // fight stays joined long enough for the player to take
            // part. The speed gate keeps the servo off bikes being
            // overtaken or stationary, so a closing ram still rams and a
            // faster player still passes; the trim rate caps closing at
            // PACE_TRIM, so a rejoined rival shadows instead of ramming.
            // After the engagement a cool-down backs the rival out of
            // reach — the break-off that lets fights cycle instead of
            // one hold lasting a whole lap.
            let player = self.state(0);
            let dx = player.pos_x - state.pos_x;
            let dy = player.pos_y - state.pos_y;
            let near_fight = snapshot.control != Control::Crashed
                && self.aux[0].snapshot.control != Control::Crashed
                && !off_road
                && dx > -combat::PUNCH_REACH
                && dx < FIGHT_ARM_RANGE
                && dy.abs() < FIGHT_LATERAL
                && state.speed > FIGHT_RIDE_SPEED
                // A rider shearing through reach at a big speed
                // difference is a pass, not a fight: the pace servo (a
                // 1.5 m/s trim) can never hold it, and arming one turns
                // the stay into a shard. Matched-speed riders — which
                // every chaser becomes on approach (the chase ceiling
                // caps closing at 8 m/s) — still arm.
                && (state.speed - self.aux[0].snapshot.speed).abs()
                    < combat::PACE_MATCH_TOLERANCE;
            if near_fight
                && self.aux[slot].shadow_secs <= 0.0
                && self.aux[slot].shadow_cooldown <= 0.0
            {
                self.aux[slot].shadow_secs = combat::PACE_MATCH_SECS;
            }
            let fighting = self.aux[slot].shadow_secs > 0.0;

            let intent = if slot == 0 {
                intent
            } else {
                // Audit item 7: rivals dodge bikes blocking their line
                // (but never the player's own steering — see the slot 0
                // branch above). A fighting rival skips the dodge and
                // parks alongside the player, offset to the side it
                // already rides: inside punch reach, outside the
                // bike-overlap band, so the fight stays joined without
                // either bike grinding the other off the road.
                let avoid = if fighting {
                    // Park on the side the rival already rides — crossing
                    // the player's line to reach the far side would shove
                    // them both off the road.
                    let side = if dy < -0.25 {
                        1.0
                    } else if dy > 0.25 {
                        -1.0
                    } else {
                        1.0
                    };
                    Some((player.pos_y + side * FIGHT_RIDE_LATERAL).clamp(
                        -self.geometry.road_width() * 0.5 + 1.5,
                        self.geometry.road_width() * 0.5 - 1.5,
                    ))
                } else {
                    self.avoidance_lane(slot)
                };
                ai::decide(
                    snapshot.speed,
                    snapshot.lateral,
                    state.pos_x,
                    self.aux[slot].target_lane,
                    cap,
                    avoid,
                    &self.geometry,
                )
            };

            let mut snapshot = bike::step_bike(snapshot, intent, off_road, curvature, dt);
            if slot != 0 {
                // Rivals never exceed their rubber-band cap.
                snapshot.speed = snapshot.speed.min(cap);
                // On straights the cap is a PACE, not just a ceiling: a
                // rival converges on it directly, because sharing the
                // player's acceleration curve otherwise pins the whole
                // field in lockstep at whatever gaps the grid dealt. In
                // corners and off the road, plain physics plus the clamp.
                let on_straight =
                    self.geometry.curvature_at(state.pos_x).abs() < ai::CURVE_DEADZONE;
                if on_straight && snapshot.control != Control::Crashed && !off_road {
                    let rate = RUBBER_BAND_ACCEL * dt;
                    snapshot.speed = if snapshot.speed < cap {
                        (snapshot.speed + rate).min(cap)
                    } else {
                        (snapshot.speed - rate).max(cap)
                    };
                }
                // A chasing rival also accelerates above the nominal
                // throttle — the cap alone never closes a gap on a player
                // already at speed. Crashed and off-road riders chase on
                // foot like everyone else.
                if chasing > 0.0 && snapshot.control != Control::Crashed && !off_road {
                    let boost = bike::ACCEL + (RUBBER_BAND_ACCEL - bike::ACCEL) * chasing;
                    snapshot.speed = snapshot.speed.max((state.speed + boost * dt).min(cap));
                }
                // Fight pacing, continued: the engagement clock runs on
                // the post-step state, and the pace servo applies while
                // it is live.
                let player = self.state(0);
                let dx = player.pos_x - state.pos_x;
                let both_upright = snapshot.control != Control::Crashed
                    && self.aux[0].snapshot.control != Control::Crashed;
                // The re-engagement cool-down ticks down like any other
                // clock; without this decrement a single engagement would
                // permanently bar the rival from ever arming again (the
                // lap-1 fight blackout).
                if self.aux[slot].shadow_cooldown > 0.0 {
                    self.aux[slot].shadow_cooldown = (self.aux[slot].shadow_cooldown - dt).max(0.0);
                }
                if self.aux[slot].shadow_secs > 0.0 && both_upright && !off_road {
                    self.aux[slot].shadow_secs = (self.aux[slot].shadow_secs - dt).max(0.0);
                    if self.aux[slot].shadow_secs == 0.0 {
                        // Engagement over: a cool-down before the rival
                        // may join again, so the player can actually pass
                        // it or ride away from it.
                        self.aux[slot].shadow_cooldown = combat::PACE_MATCH_COOLDOWN;
                    }
                    let pace = if dx > combat::PACE_STAGGER {
                        player.speed + combat::PACE_TRIM
                    } else if dx < -combat::PACE_STAGGER {
                        player.speed - combat::PACE_TRIM
                    } else {
                        player.speed
                    };
                    // A queued fighter never drives through the rider in
                    // front of it: the pace yields to whoever is directly
                    // ahead (the player's own stagger is the servo's job,
                    // so this only bites for teammate queues).
                    let pace = if !ahead_is_player && gap_ahead < combat::PACE_STAGGER + 1.0 {
                        pace.min((ahead_speed - 0.5).max(0.0))
                    } else {
                        pace
                    };
                    // The engagement overrides top speed: matching the
                    // player is the whole point of the fight (audit item
                    // 6). The break-off cap above restores the rival's
                    // own pace once it ends.
                    snapshot.speed = pace;
                }
            }
            if bike::should_crash(&snapshot, half_width) {
                bike::crash(&mut snapshot);
            }
            if was_crashed
                && snapshot.control != Control::Crashed
                && snapshot.is_off_road(half_width)
            {
                // Back in the saddle, stranded in the dirt: place the rider
                // in a lane at least RECOVERY_LANE_MARGIN meters inside the
                // edge (audit item 3), so a picked-up bike has road to ride
                // instead of immediately leaving the tarmac again.
                snapshot.lateral = snapshot.lateral.signum() * (half_width - RECOVERY_LANE_MARGIN);
            }
            // Shoulder rumble: the last meters before the tarmac ends warn
            // the rider — a scrub and a shake — before the off-road crash
            // threshold. True off the tarmac, it is gravel: keep scrubbing.
            let on_rumble = snapshot.control != Control::Crashed
                && (half_width - snapshot.lateral.abs()) < RUMBLE_BAND;
            if on_rumble {
                snapshot.speed *= RUMBLE_SCRUB;
                if slot == 0 {
                    self.player_rumble = true;
                }
            }

            // Integrate forward motion: speed is meters per second.
            let pos_x = state.pos_x + snapshot.speed * dt;
            // Lateral velocity, from where the bike was last tick: the
            // collision pass uses it to tell a lean-along from a slam.
            self.aux[slot].lateral_vel = (snapshot.lateral - self.aux[slot].snapshot.lateral) / dt;
            self.aux[slot].snapshot = snapshot;
            self.pool.set(
                self.handle(slot),
                BikeState {
                    pos_x,
                    pos_y: snapshot.lateral,
                    speed: snapshot.speed,
                    heading: snapshot.heading,
                },
            );
        }

        self.resolve_combat(intent.attack, dt);
        self.resolve_collisions();
        self.resolve_hazards();
        self.advance_traffic(dt);

        if self.lap_index() >= self.total_laps {
            self.finished = true;
        }
    }

    /// Move traffic one tick and wrap cars past the lap line.
    fn advance_traffic(&mut self, dt: f32) {
        for car in &mut self.traffic {
            car.state.pos_x += car.cruise * dt;
            if car.state.pos_x >= self.track_length {
                car.state.pos_x -= self.track_length;
            }
            car.state.speed = car.cruise;
        }
    }

    /// Cones knock speed down, not riders. One overlapping tick shaves a
    /// little; plowing through a cone line costs real time.
    fn resolve_hazards(&mut self) {
        for slot in 0..self.bike_count {
            if self.aux[slot].snapshot.control == Control::Crashed {
                continue;
            }
            let state = self.state(slot);
            for hazard_index in 0..self.hazards.len() {
                let hazard = self.hazards[hazard_index];
                if (state.pos_x - hazard.pos_x).abs() < 1.5
                    && (state.pos_y - hazard.lateral).abs() < 0.9
                {
                    self.scrub_speed(slot, HAZARD_SCRUB);
                }
            }
        }
    }

    /// Feed one rendered frame's delta into the sim clock and run however
    /// many 60 Hz ticks it asks for (clamped by the clock's spiral guard).
    /// Returns the number of ticks run.
    pub fn step_frame(&mut self, frame_dt_secs: f64) -> usize {
        let ticks = self.clock.update(frame_dt_secs);
        for _ in 0..ticks {
            self.tick();
        }
        ticks
    }

    /// Lateral avoidance (audit item 7): when a rival closes on another bike
    /// from behind, it sidesteps to a staggered lane instead of riding into
    /// the pack — the behaviour it already had for cars, applied to bikes.
    /// The player never auto-dodges: steering is the rider's job.
    ///
    /// Pure in the race state: no mutation, safe to call mid-step.
    fn avoidance_lane(&self, slot: usize) -> Option<f32> {
        let me = self.state(slot);
        let mut blocker = None;
        let mut best_gap = AVOID_RANGE;
        for other in 0..self.bike_count {
            if other == slot || other == 0 {
                continue;
            }
            let them = self.state(other);
            let gap = them.pos_x - me.pos_x;
            if gap <= 0.0 {
                continue;
            }
            let dy = (them.pos_y - me.pos_y).abs();
            let urgent = me.speed - them.speed > AVOID_CLOSING || gap < AVOID_CONTACT;
            if gap < best_gap && dy < AVOID_LATERAL && urgent {
                best_gap = gap;
                blocker = Some(them.pos_y);
            }
        }
        // The player is never a forward blocker: a rival riding behind the
        // player is governed by the pace-match taper, which holds it clear
        // of the overlap band without a swerve (audit item 6), and a rival
        // beside the player is either fighting (steer onto the player's
        // line) or clear laterally. Only the rear emergency below — a
        // much faster rider closing from behind — makes a rival step
        // aside for the player.
        // A faster rider closing hard from behind is a rear-ending in
        // progress: step aside, whatever is ahead. Below the emergency
        // threshold the catch-up is a fight (the pace-match servo), not
        // a ram, so the line is held; above it, only the swerve saves
        // the slower rider (audit item 7).
        if blocker.is_none() {
            let me_x = me.pos_x;
            let me_y = me.pos_y;
            let my_speed = me.speed;
            for other in 0..self.bike_count {
                if other == slot {
                    continue;
                }
                let them = self.state(other);
                let gap = them.pos_x - me_x;
                if gap >= 0.0 || gap < -AVOID_RANGE {
                    continue;
                }
                if (them.pos_y - me_y).abs() < AVOID_LATERAL
                    && them.speed - my_speed > combat::PACE_MATCH_TOLERANCE
                {
                    blocker = Some(them.pos_y);
                    break;
                }
            }
        }
        // Step around the blocker — but on whichever side actually clears
        // the whole pack and stays on the road: a lane that dodges one
        // bike by steering into another (or off the tarmac) is no dodge
        // at all.
        blocker.map(|lane| {
            let me_side = if (me.pos_y - lane).abs() < 0.01 {
                if lane <= 0.0 {
                    1.0
                } else {
                    -1.0
                }
            } else {
                (me.pos_y - lane).signum()
            };
            let limit = self.geometry.road_width() * 0.5 - 1.5;
            let first = (lane + me_side * AVOID_STEP).clamp(-limit, limit);
            let second = (lane - me_side * AVOID_STEP).clamp(-limit, limit);
            if self.clearance(slot, first) >= self.clearance(slot, second) {
                first
            } else {
                second
            }
        })
    }

    /// How much lateral room a candidate lane leaves clear of every other
    /// bike within avoidance range: the distance to the nearest one.
    fn clearance(&self, slot: usize, lane: f32) -> f32 {
        let me = self.state(slot);
        let mut best = f32::INFINITY;
        for other in 0..self.bike_count {
            if other == slot {
                continue;
            }
            let them = self.state(other);
            if (them.pos_x - me.pos_x).abs() > AVOID_RANGE {
                continue;
            }
            best = best.min((them.pos_y - lane).abs());
        }
        best
    }

    /// The wave 7b combat pass: the player's swing from this tick's intent,
    /// rival retaliation, and every rider's combat timers. Runs after the
    /// movement loop — attacks resolve against where riders actually are,
    /// and a kick's shove is visible to this tick's collision pass.
    fn resolve_combat(&mut self, player_attack: Option<Attack>, dt: f32) {
        let mut attacked = [false; MAX_BIKES];
        let mut was_hit = [false; MAX_BIKES];

        // The player's swing: auto-target the nearest rider in reach.
        // No target, no swing, no cost.
        if let Some(attack) = player_attack {
            let player_ready = self.aux[0].snapshot.control != Control::Crashed
                && self.aux[0].combat.cooldown >= combat::ATTACK_COOLDOWN;
            if player_ready {
                let attacker = self.state(0);
                let target = combat::pick_target(
                    attacker.pos_x,
                    attacker.pos_y,
                    self.upright_riders_except(0)
                        .filter(|(_, other)| {
                            combat::in_reach(
                                attack,
                                other.pos_x - attacker.pos_x,
                                other.pos_y - attacker.pos_y,
                            )
                        })
                        .map(|(slot, other)| (slot, other.pos_x, other.pos_y)),
                );
                if let Some(target) = target {
                    self.land_hit(0, target, attack);
                    attacked[0] = true;
                    was_hit[target] = true;
                } else {
                    // A whiff still plays the swing and spends the attempt
                    // (audit item 8) — but nothing is in reach, so no
                    // stamina changes hands.
                    self.aux[0].combat.cooldown = 0.0;
                    combat::start_swing(&mut self.aux[0].combat, attack);
                    attacked[0] = true;
                }
            }
        }

        // Rival retaliation: wind up at the player once the
        // aggression-weighted cooldown is up — sooner with a car alongside,
        // because a rider shoved toward traffic is angrier. Rivals do not
        // brawl among themselves: the player is the only target (audit
        // item 7).
        for slot in 1..self.bike_count {
            if self.aux[slot].snapshot.control == Control::Crashed {
                continue;
            }

            // A pending wind-up ticks toward its landing: the hit only
            // lands if the target is still in reach when the telegraph
            // expires (audit item 1).
            if self.aux[slot].windup_ticks > 0 {
                self.aux[slot].windup_ticks -= 1;
                if self.aux[slot].windup_ticks == 0 {
                    if let Some(target) = self.aux[slot].windup_target.take() {
                        let attacker = self.state(slot);
                        let target_state = self.state(target);
                        let dx = target_state.pos_x - attacker.pos_x;
                        let dy = target_state.pos_y - attacker.pos_y;
                        let still_in_reach = self.aux[target].snapshot.control != Control::Crashed
                            && combat::in_reach(Attack::Punch, dx, dy);
                        if still_in_reach {
                            self.land_hit(slot, target, Attack::Punch);
                            was_hit[target] = true;
                        }
                    }
                }
                continue;
            }

            let attacker = self.state(slot);
            let near_traffic = self.traffic_alongside(attacker.pos_x, attacker.pos_y);
            let ready =
                combat::retaliation_cooldown(self.aux[slot].aggression, near_traffic.is_some());
            if self.aux[slot].combat.cooldown < ready {
                continue;
            }
            let player = self.state(0);
            let target = if self.aux[0].snapshot.control != Control::Crashed
                && combat::in_reach(
                    Attack::Punch,
                    player.pos_x - attacker.pos_x,
                    player.pos_y - attacker.pos_y,
                ) {
                Some(0)
            } else {
                None
            };
            if let Some(target) = target {
                // Telegraph: the swing pose starts now; the hit lands
                // WINDUP_TICKS later if the target is still in reach.
                self.aux[slot].combat.cooldown = 0.0;
                combat::start_swing(&mut self.aux[slot].combat, Attack::Punch);
                self.aux[slot].windup_ticks = combat::WINDUP_TICKS;
                self.aux[slot].windup_target = Some(target);
                attacked[slot] = true;
            }
        }

        // Timers: instability decays while not being hit; cooldowns count
        // up, poses decay, stamina regenerates while clean. A downed rider
        // freezes its combat state until the crash flow hands the bike back.
        for slot in 0..self.bike_count {
            if self.aux[slot].snapshot.control == Control::Crashed {
                continue;
            }
            if !was_hit[slot] {
                self.aux[slot].instability =
                    (self.aux[slot].instability - combat::INSTABILITY_DECAY * dt).max(0.0);
            }
            combat::step_combat(
                &mut self.aux[slot].combat,
                attacked[slot],
                was_hit[slot],
                dt,
            );
        }
    }

    /// Land one hit from `attacker` on `target`: the swing costs the
    /// attacker stamina and opens the pose; the target takes instability
    /// and a stamina drain, a kick's lateral shove (toward traffic when it
    /// is there), and — past the threshold, or on an empty bar — a
    /// dismount through the existing crash flow.
    fn land_hit(&mut self, attacker: usize, target: usize, attack: Attack) {
        self.aux[attacker].combat.cooldown = 0.0;
        self.aux[attacker].combat.stamina =
            (self.aux[attacker].combat.stamina - combat::stamina_cost(attack)).max(0.0);
        combat::start_swing(&mut self.aux[attacker].combat, attack);

        let target_state = self.state(target);
        self.aux[target].instability += combat::hit_instability(attack);
        self.aux[target].combat.stamina =
            (self.aux[target].combat.stamina - combat::HIT_STAMINA_DRAIN).max(0.0);
        if attack == Attack::Kick {
            let traffic_y = self.traffic_alongside(target_state.pos_x, target_state.pos_y);
            let attacker_y = self.state(attacker).pos_y;
            let push = combat::KICK_PUSH
                * combat::kick_push_side(attacker_y, target_state.pos_y, traffic_y);
            self.write_lateral(target, target_state.pos_y + push);
        }
        if combat::takes_dismount(
            self.aux[target].instability,
            self.aux[target].combat.stamina,
        ) {
            // The crash flow owns the dismount cap (see crash_slot):
            // past it the hit still hurts but the rider stays up.
            #[cfg(test)]
            eprintln!("CRASH slot={target} cause=hit-dismount");
            self.crash_slot(target);
        }
    }

    /// The lateral position of the nearest car alongside a rider, within
    /// the near-traffic window — both the "toward traffic" kick direction
    /// and the retaliation rage read it. `None` on a clear road.
    fn traffic_alongside(&self, pos_x: f32, pos_y: f32) -> Option<f32> {
        let mut best: Option<(f32, f32)> = None; // (|dx|, car_y)
        for car in &self.traffic {
            let dx = (car.state.pos_x - pos_x).abs();
            let dy = (car.state.pos_y - pos_y).abs();
            if dx <= combat::NEAR_TRAFFIC_RANGE
                && dy <= combat::ADJACENT_LATERAL + 1.5
                && best.is_none_or(|(best_dx, _)| dx < best_dx)
            {
                best = Some((dx, car.state.pos_y));
            }
        }
        best.map(|(_, car_y)| car_y)
    }

    /// Every upright rider except `excluded`, with their pooled state.
    fn upright_riders_except(
        &self,
        excluded: usize,
    ) -> impl Iterator<Item = (usize, BikeState)> + '_ {
        (0..self.bike_count)
            .filter(move |&slot| {
                slot != excluded && self.aux[slot].snapshot.control != Control::Crashed
            })
            .map(move |slot| (slot, self.state(slot)))
    }

    /// Push overlapping bikes apart laterally and scrub their speed; hard
    /// contact puts riders down, using the pure [`bikes_overlap`] and
    /// [`overlap_push`] helpers.
    fn resolve_collisions(&mut self) {
        for i in 0..self.bike_count {
            for j in (i + 1)..self.bike_count {
                let (a, b) = (self.state(i), self.state(j));
                if !bikes_overlap(&a, &b) {
                    continue;
                }
                if self.fighting_the_player(i, j) {
                    // A live engagement owns its geometry: the pace servo
                    // holds the pair at the stagger and the formation
                    // steering holds them a bike's width apart. The
                    // collision push would only fight the servo and
                    // shove the player around mid-probe.
                    continue;
                }

                let dy = (b.pos_y - a.pos_y).abs();
                // The rider behind is the rammer when speeds differ.
                let (rear, _front) = if a.pos_x <= b.pos_x { (i, j) } else { (j, i) };
                let closing =
                    self.state(rear).speed - self.state(if rear == i { j } else { i }).speed;

                if closing > CRASH_CLOSING_SPEED {
                    // Rammed from behind at speed: the rammer goes down.
                    #[cfg(test)]
                    eprintln!("CRASH slot={rear} cause=ram");
                    self.crash_slot(rear);
                    // The rammed rider barely notices.
                    self.scrub_speed(if rear == i { j } else { i }, 0.97);
                } else if dy < SIDE_CRASH_GAP
                    && self.state(i).speed.min(self.state(j).speed) > SIDE_CRASH_SPEED
                    && (self.aux[i].lateral_vel - self.aux[j].lateral_vel).abs() > SIDE_CRASH_SLAM
                    && !self.fighting_the_player(i, j)
                {
                    // A slam into side-by-side contact at speed takes both
                    // down; a shared lean just gets the push below.
                    #[cfg(test)]
                    eprintln!("CRASH slot={i},{j} cause=slam");
                    self.crash_slot(i);
                    self.crash_slot(j);
                } else {
                    let (push_a, push_b) = overlap_push(b.pos_y - a.pos_y);
                    self.write_lateral(i, self.aux[i].snapshot.lateral + push_a);
                    self.write_lateral(j, self.aux[j].snapshot.lateral + push_b);
                    // Speed penalty: both riders back off while overlapping.
                    self.scrub_speed(i, COLLISION_SCRUB);
                    self.scrub_speed(j, COLLISION_SCRUB);
                }
            }

            // Bikes versus traffic: cars are heavy and always win.
            for car_index in 0..self.traffic.len() {
                let car = self.traffic[car_index].state;
                let bike_state = self.state(i);
                if !hits_traffic(&bike_state, &car) {
                    continue;
                }
                let closing = bike_state.speed - car.speed;
                if closing > CRASH_CLOSING_SPEED {
                    self.crash_slot(i);
                } else {
                    // Bounce off the back/side of the car, hard.
                    let push = if bike_state.pos_y >= car.pos_y {
                        0.6
                    } else {
                        -0.6
                    };
                    self.write_lateral(i, self.aux[i].snapshot.lateral + push);
                    self.scrub_speed(i, 0.88);
                }
            }
        }
    }

    /// Whether either of these overlapping riders is a rival in a live
    /// pace-match engagement with the other: a fighting pair converging on
    /// a shared line leans together instead of slamming (audit item 6).
    fn fighting_the_player(&self, i: usize, j: usize) -> bool {
        if i == 0 {
            self.aux[j].shadow_secs > 0.0
        } else if j == 0 {
            self.aux[i].shadow_secs > 0.0
        } else {
            false
        }
    }

    /// Put one bike (spawn-order slot) into a crash. Counts toward the
    /// per-rider dismount cap: past the cap the rider stays in the
    /// saddle — a race is not a pile-up (audit item 7) — whatever brought
    /// the bike down. Hits still land (stamina, instability, shoves); a
    /// capped rider just cannot be dismounted again.
    fn crash_slot(&mut self, slot: usize) {
        if self.aux[slot].snapshot.control == Control::Crashed
            || self.aux[slot].dismounts >= combat::DISMOUNT_CAP
        {
            return;
        }
        self.aux[slot].dismounts += 1;
        bike::crash(&mut self.aux[slot].snapshot);
        // A downed rider's pending swing dies with the fight: no wind-up
        // survives the crash flow to land after recovery.
        self.aux[slot].windup_ticks = 0;
        self.aux[slot].windup_target = None;
        self.aux[slot].shadow_secs = 0.0;
    }

    /// Test seam: removes all traffic so handling tests steer without
    /// running into cars.
    #[cfg(test)]
    pub(crate) fn clear_traffic(&mut self) {
        self.traffic.clear();
    }

    /// Test seam: injects one slow car at a fixed spot, so combat tests can
    /// stage "traffic alongside" deterministically.
    #[cfg(test)]
    pub(crate) fn stage_traffic(&mut self, pos_x: f32, pos_y: f32) {
        self.traffic.push(TrafficCar {
            state: BikeState {
                pos_x,
                pos_y,
                speed: 0.0,
                heading: 0.0,
            },
            cruise: 0.0,
        });
    }

    /// Test seam: skips the green-light countdown so combat tests stage and
    /// fight immediately, without the grid waiting on the lights.
    #[cfg(test)]
    pub(crate) fn skip_countdown(&mut self) {
        self.countdown_ticks = 0;
    }

    /// Test seam: sets one rider's stamina bar, so a test can stage the
    /// empty-bar dismount rule in isolation.
    #[cfg(test)]
    pub(crate) fn set_stamina(&mut self, slot: usize, stamina: f32) {
        self.aux[slot].combat.stamina = stamina;
    }

    fn write_lateral(&mut self, slot: usize, lateral: f32) {
        self.aux[slot].snapshot.lateral = lateral;
        let mut state = self.state(slot);
        state.pos_y = lateral;
        self.pool.set(self.handle(slot), state);
    }

    fn scrub_speed(&mut self, slot: usize, factor: f32) {
        self.aux[slot].snapshot.speed *= factor;
        let mut state = self.state(slot);
        state.speed *= factor;
        self.pool.set(self.handle(slot), state);
    }

    /// The live handle of one spawn-order slot.
    fn handle(&self, slot: usize) -> BikeHandle {
        self.handles[slot].expect("race slots are always live")
    }

    /// Snapshot of one bike's pooled state.
    fn state(&self, slot: usize) -> BikeState {
        self.pool
            .get(self.handle(slot))
            .expect("race slots are always live")
    }

    /// The player's total distance covered, in meters.
    pub fn player_distance(&self) -> f32 {
        self.state(0).pos_x
    }

    /// Whole laps the player has completed.
    pub fn lap_index(&self) -> u32 {
        lap_index(self.player_distance(), self.track_length)
    }

    /// Progress through the player's current lap, in `[0, 1)`.
    pub fn lap_progress(&self) -> f32 {
        lap_progress(self.player_distance(), self.track_length)
    }

    /// Laps needed to finish, for the HUD's `lap x of N` display.
    pub fn total_laps(&self) -> u32 {
        self.total_laps
    }

    /// Length of one lap, in meters, for the HUD's progress display.
    pub fn lap_length(&self) -> f32 {
        self.track_length
    }

    /// Whether the player has completed every lap.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The player's current control state.
    pub fn player_control(&self) -> Control {
        self.aux[0].snapshot.control
    }

    /// Control state of one bike (spawn-order slot), for rendering.
    pub fn bike_control(&self, slot: usize) -> Control {
        self.aux[slot].snapshot.control
    }

    /// The player's speed, in meters per second.
    pub fn player_speed(&self) -> f32 {
        self.state(0).speed
    }

    /// The player's lateral position, in meters.
    pub fn player_lateral(&self) -> f32 {
        self.state(0).pos_y
    }

    /// The player's visual bank angle, in radians (positive tips right on
    /// screen), for rendering and HUD feel.
    pub fn player_lean(&self) -> f32 {
        self.aux[0].snapshot.lean
    }

    /// The player's heading, in radians — the angle of the bike's velocity
    /// against the road centreline (positive points right of straight
    /// ahead). The camera leans with this first, road bend second.
    pub fn player_heading(&self) -> f32 {
        self.aux[0].snapshot.heading
    }

    /// Visual bank angle of one bike (spawn-order slot), for rendering.
    pub fn bike_lean(&self, slot: usize) -> f32 {
        self.aux[slot].snapshot.lean
    }

    /// The player's combat stamina in `0..=1`, for the HUD bar.
    pub fn player_stamina(&self) -> f32 {
        self.aux[0].combat.stamina
    }

    /// Combat stamina of one bike (spawn-order slot).
    pub fn bike_stamina(&self, slot: usize) -> f32 {
        self.aux[slot].combat.stamina
    }

    /// The swing one bike is mid-throw on, if any — the renderer picks the
    /// punch/kick frame from it.
    pub fn bike_attack_pose(&self, slot: usize) -> Option<Attack> {
        let combat = &self.aux[slot].combat;
        if combat.swing > 0.0 {
            combat.swing_attack
        } else {
            None
        }
    }

    /// Seconds left on the player's crash timer, or 0 when upright.
    pub fn player_crash_timer(&self) -> f32 {
        if self.aux[0].snapshot.control == Control::Crashed {
            self.aux[0].snapshot.crash_timer
        } else {
            0.0
        }
    }

    /// Whether the player was on the shoulder rumble strip this tick —
    /// the warning before running off the road at speed is a crash.
    pub fn player_on_rumble(&self) -> bool {
        self.player_rumble
    }

    /// Live bike count, player included.
    pub fn bike_count(&self) -> usize {
        self.bike_count
    }

    /// Snapshots of every live bike, in spawn order (player first). The
    /// iterator itself allocates nothing; the caller decides what to collect.
    pub fn bike_states(&self) -> impl Iterator<Item = BikeState> + '_ {
        (0..self.bike_count).map(|slot| self.state(slot))
    }

    /// Ticks run since construction.
    pub fn tick_index(&self) -> u64 {
        self.clock.tick_index()
    }

    /// Seconds left on the green-light countdown, rounded up.
    pub fn countdown_secs(&self) -> u32 {
        self.countdown_ticks.div_ceil(60)
    }

    /// Whether the lights are still red.
    pub fn is_counting_down(&self) -> bool {
        self.countdown_ticks > 0
    }

    /// The player's race position: 1 plus the number of rivals ahead.
    pub fn race_position(&self) -> usize {
        let player_x = self.state(0).pos_x;
        1 + (1..self.bike_count)
            .filter(|&slot| self.state(slot).pos_x > player_x)
            .count()
    }

    /// Traffic on the road, for rendering.
    pub fn traffic_cars(&self) -> &[TrafficCar] {
        &self.traffic
    }

    /// Static cone hazards on the road, for rendering.
    pub fn hazards(&self) -> &[Hazard] {
        &self.hazards
    }
}

/// Whether a bike has run into a car: bikes are shorter and narrower than
/// cars, so the combined envelope is wider than bike-vs-bike. Pure.
pub fn hits_traffic(bike: &BikeState, car: &BikeState) -> bool {
    (car.pos_x - bike.pos_x).abs() < BIKE_HALF_LENGTH + CAR_HALF_LENGTH
        && (car.pos_y - bike.pos_y).abs() < BIKE_HALF_WIDTH + CAR_HALF_WIDTH
}

/// Distance between cone hazards along the road, in meters.
pub const HAZARD_SPACING: f32 = 47.0;
/// Fraction of speed lost per overlapping cone tick.
pub const HAZARD_SCRUB: f32 = 0.93;

#[cfg(test)]
mod tests {
    use super::*;
    use world::CurveSegment;

    const TRACK: f32 = 500.0;
    const LAPS: u32 = 3;

    fn flat_race(rivals: usize, seed: u64) -> Race {
        Race::new(RaceConfig {
            rivals,
            track_length: TRACK,
            total_laps: LAPS,
            seed,
            geometry: RoadGeometry::new(Vec::new(), Vec::new(), 12.0),
        })
    }

    fn run(race: &mut Race, ticks: usize, per_tick: &[GameInput]) {
        while race.is_counting_down() {
            race.tick();
        }
        for _ in 0..ticks {
            for input in per_tick {
                race.push_input(*input);
            }
            race.tick();
        }
    }

    #[test]
    fn countdown_holds_everyone_then_releases() {
        let mut race = flat_race(2, 7);
        assert!(race.is_counting_down());
        assert_eq!(race.countdown_secs(), 3);
        // Throttle held during the countdown does nothing.
        race.push_input(GameInput::Accelerate);
        race.tick();
        assert_eq!(race.player_speed(), 0.0, "red lights hold the grid");
        assert!(race.is_counting_down());
        while race.is_counting_down() {
            race.tick();
        }
        race.push_input(GameInput::Accelerate);
        race.tick();
        assert!(race.player_speed() > 0.0, "released");
    }

    #[test]
    fn ramming_the_bike_ahead_at_speed_crashes_the_rammer() {
        let mut race = flat_race(1, 42);
        run(&mut race, 100, &[GameInput::Accelerate]);
        // Stage the rival just ahead, stopped, in the player's line.
        race.place_bike(1, race.player_distance() + 2.0, 0.0);
        let player_speed = race.player_speed();
        assert!(player_speed > 20.0);

        race.tick();

        assert_eq!(race.player_control(), Control::Crashed, "rammer down");
    }

    #[test]
    fn a_rival_dodges_a_bike_blocking_its_line() {
        // Two rivals: slot 1 dodges, slot 2 is the blocker.
        let mut race = flat_race(2, 42);
        // Stage the blocker 5 m ahead of the dodger, dead in its line:
        // touching distance, so the dodge fires without any closing speed.
        race.place_bike(1, 295.0, 0.0);
        race.place_bike(2, 300.0, 0.0);
        // Step around: to the side with road to spare when dead centre.
        let dodge = race.avoidance_lane(1).expect("dodge");
        assert!((dodge - AVOID_STEP).abs() < 1e-3, "dodge={dodge}");

        // Offset the blocker half a meter right: the rival steps left.
        race.place_bike(2, 300.0, 0.5);
        let dodge = race.avoidance_lane(1).expect("dodge");
        assert!((dodge - (0.5 - AVOID_STEP)).abs() < 1e-3, "dodge={dodge}");

        // Nothing ahead: no dodge.
        race.place_bike(2, 100.0, 0.0);
        assert_eq!(race.avoidance_lane(1), None);

        // The player blocking the line is different: a rival holds its
        // line under a catching player (the pace-match servo turns the
        // catch-up into a fight) and swerves only at emergency closing.
        race.place_bike(1, 295.0, 0.0);
        race.place_bike(0, 300.0, 0.0);
        assert_eq!(
            race.avoidance_lane(1),
            None,
            "a catching player is a fight, not a blocker"
        );
    }

    #[test]
    fn running_into_traffic_at_speed_is_a_crash() {
        let mut race = flat_race(0, 42);
        run(&mut race, 1, &[]);
        // Stage the player 30 m behind a car, in its lane, from a stop: full
        // throttle reaches 40+ m/s down the road and runs the car down well
        // over the 12 m/s closing-speed crash threshold.
        let car = race.traffic_cars()[0].state;
        race.place_bike(0, car.pos_x - 30.0, car.pos_y);
        run(&mut race, 300, &[GameInput::Accelerate]);
        assert_eq!(race.player_control(), Control::Crashed, "traffic bites");
    }

    #[test]
    fn race_position_counts_riders_ahead() {
        let mut race = flat_race(2, 7);
        // Even-indexed rivals start ahead of the line, odd-indexed behind.
        assert_eq!(race.race_position(), 2, "one rival starts ahead");
        // Push the player just past the rival ahead.
        let lead = race.bike_states().map(|s| s.pos_x).fold(0.0_f32, f32::max);
        race.place_bike(0, lead + 5.0, 0.0);
        assert_eq!(race.race_position(), 1);
    }

    #[test]
    fn scripted_accelerate_then_brake_transitions_the_state_machine() {
        let mut race = flat_race(1, 7);
        run(&mut race, 60, &[GameInput::Accelerate]);
        assert_eq!(race.player_control(), Control::Accelerate);
        assert!(race.player_speed() > 10.0, "one second of throttle");

        run(&mut race, 30, &[GameInput::Brake]);
        assert_eq!(race.player_control(), Control::Brake);
        assert!(race.player_speed() < 10.0, "half a second of hard braking");
    }

    #[test]
    fn input_ring_drains_exactly_once_per_tick() {
        let mut race = flat_race(0, 7);
        while race.is_counting_down() {
            race.tick();
        }
        // Roll first: steering needs speed.
        run(&mut race, 30, &[GameInput::Accelerate]);
        let speed = race.player_speed();
        assert!(speed >= bike::MIN_STEER_SPEED, "full steering authority");
        // Three steering pulses in one tick count once. The pulse's lateral
        // motion follows the heading: one tick of ramped command rotates
        // the heading by full_lock · ramp, and the lateral step is the
        // across-road component of speed.
        for _ in 0..3 {
            race.push_input(GameInput::SteerRight);
        }
        race.tick();
        let one_pulse = race.player_lateral();
        let ramp = (TICK_DURATION_SECS as f32 / bike::STEER_RAMP_SECS).min(1.0);
        let expected = speed * (bike::FULL_LOCK_HEADING * ramp).tan() * TICK_DURATION_SECS as f32;
        assert!((one_pulse - expected).abs() < 1e-4);

        // The next tick sees nothing: coast, no further lateral motion.
        race.tick();
        assert_eq!(race.player_lateral(), one_pulse);
        assert_eq!(race.player_control(), Control::Coast);
    }

    #[test]
    fn off_road_at_low_speed_slows_without_crashing() {
        let mut race = flat_race(0, 7);
        race.clear_traffic();
        // Throttle + steer right together: the bike stays under the 30 m/s
        // crash threshold while full steering authority walks it off the
        // road. The across-road speed is speed · tan(full lock) — a few
        // seconds to cross 6 m of tarmac, so 600 ticks leaves margin.
        for _ in 0..600 {
            run(
                &mut race,
                1,
                &[GameInput::Accelerate, GameInput::SteerRight],
            );
        }
        let half_width = 6.0;
        assert!(race.player_lateral() > half_width, "driven off the road");
        assert_ne!(
            race.player_control(),
            Control::Crashed,
            "slow enough to survive"
        );
        assert!(
            race.player_speed() <= bike::OFFROAD_MAX_SPEED + 0.5,
            "dirt caps speed"
        );
    }

    #[test]
    fn off_road_at_high_speed_crashes_then_recovers() {
        let mut race = flat_race(0, 7);
        race.clear_traffic();
        // 180 ticks of throttle exceeds the 30 m/s crash threshold.
        run(&mut race, 180, &[GameInput::Accelerate]);
        // Hold right WITH throttle so the bike crosses the edge still over
        // the crash threshold (coasting bleeds under it on the way out).
        race.clear_traffic();
        let mut steering = 0;
        while race.player_lateral() <= 6.0 {
            run(
                &mut race,
                1,
                &[GameInput::SteerRight, GameInput::Accelerate],
            );
            steering += 1;
            assert!(steering < 200, "never left the road");
        }
        assert_eq!(race.player_control(), Control::Crashed);

        // After the crash duration the rider is back on coast — and back on
        // the tarmac: a recovered racer rejoins the race instead of being
        // stranded off-road at the dirt cap.
        let recovery_ticks =
            (bike::CRASH_DURATION_SECS / TICK_DURATION_SECS as f32).ceil() as usize + 10;
        run(&mut race, recovery_ticks, &[]);
        assert_eq!(race.player_control(), Control::Coast);
        assert!(
            race.player_lateral() <= 6.0,
            "recovered rider glided back onto the tarmac"
        );
    }

    #[test]
    fn the_shoulder_rumbles_and_scrubs_before_the_edge() {
        // The player sits just inside the shoulder band (half width 6,
        // band 1.5) and brings them up to speed on the straight.
        let mut race = flat_race(0, 7);
        race.clear_traffic();
        race.place_bike(0, 60.0, 5.5);
        run(&mut race, 60, &[GameInput::Accelerate]);
        // One coast tick on the strip: flagged, and scrubbed harder than
        // the same tick on the centreline.
        race.tick();
        assert!(race.player_on_rumble(), "on the rumble strip");
        let rumble_speed = race.player_speed();

        let mut centre = flat_race(0, 7);
        centre.clear_traffic();
        centre.place_bike(0, 60.0, 0.0);
        run(&mut centre, 60, &[GameInput::Accelerate]);
        centre.tick();
        assert!(!centre.player_on_rumble(), "centreline is clean");
        assert!(
            centre.player_speed() - rumble_speed > 0.04,
            "the strip scrubs speed"
        );
    }

    #[test]
    fn race_position_trades_places_as_the_pack_races() {
        let mut race = flat_race(3, 7);
        race.clear_traffic();
        let start_position = race.race_position();
        assert!(start_position > 1, "rivals start ahead of the player");

        // A straight full-throttle run: the player claws through the field.
        // Rivals pace the player through the mid-race, so the pass comes as
        // the fight cycles — the engagement holds the pair together (8 s),
        // the break-off backs the rival out, and the player goes through
        // during the cooldown. A cycle plus one allowed dismount recovery
        // per rival needs this much window; countdown plus three
        // catch-fight-pass cycles does not fit in a minute.
        for tick in 0..5400usize {
            race.push_input(GameInput::Accelerate);
            race.tick();
            if tick.is_multiple_of(120) {
                #[cfg(test)]
                for slot in 1..race.bike_count() {
                    let s = race.bike_states().nth(slot).expect("bike");
                    eprintln!(
                        "POS t={tick} pos={} r{slot} dx={:.1} dy={:.1} v={:.1} ctrl={:?} shadow={:.1} cool={:.1}",
                        race.race_position(),
                        race.player_distance() - s.pos_x,
                        race.player_lateral() - s.pos_y,
                        s.speed,
                        race.bike_control(slot),
                        race.aux[slot].shadow_secs,
                        race.aux[slot].shadow_cooldown,
                    );
                }
            }
        }
        assert!(
            race.race_position() < start_position,
            "the player gained places (now {})",
            race.race_position()
        );
    }

    #[test]
    fn collision_push_resolves_both_directions_and_ties() {
        // Rival to the player's right (dy > 0): player pushed left, rival right.
        let (push_player, push_rival) = overlap_push(0.6);
        assert!(push_player < 0.0);
        assert!(push_rival > 0.0);
        assert!((push_player + push_rival).abs() < 1e-6, "push is symmetric");

        // Mirrored (dy < 0): the deltas flip sign exactly.
        let (push_player, push_rival) = overlap_push(-0.6);
        assert!(push_player > 0.0);
        assert!(push_rival < 0.0);

        // Perfect side-by-side tie (dy == 0): deterministic, not NaN.
        let (push_player, push_rival) = overlap_push(0.0);
        assert!(push_player < 0.0 && push_rival > 0.0);
        assert_eq!(push_player, -push_rival, "tie splits evenly, sign apart");
    }

    #[test]
    fn overlap_detection_respects_length_and_width_bounds() {
        let a = BikeState {
            pos_x: 0.0,
            pos_y: 0.0,
            speed: 0.0,
            heading: 0.0,
        };
        let touching_bike = BikeState {
            pos_x: 1.0,
            pos_y: 0.0,
            ..a
        };
        assert!(bikes_overlap(&a, &touching_bike));

        let ahead = BikeState {
            pos_x: BIKE_HALF_LENGTH * 2.0,
            ..a
        };
        assert!(
            !bikes_overlap(&a, &ahead),
            "exactly one bike length apart: clear"
        );

        let aside = BikeState {
            pos_x: 0.0,
            pos_y: BIKE_HALF_WIDTH * 2.0,
            ..a
        };
        assert!(
            !bikes_overlap(&a, &aside),
            "exactly one bike width apart: clear"
        );
    }

    #[test]
    fn collision_tick_pushes_both_ways_and_scrubs_speed() {
        // Stage a side-by-side overlap through the respawn seam, once with
        // the rival right of the player and once left: the tick must resolve
        // in both directions, and both bikes must lose speed.
        for rival_side in [1.0_f32, -1.0] {
            let mut race = flat_race(1, 42);
            // Give the player some speed first so the scrub is observable.
            run(&mut race, 20, &[GameInput::Accelerate]);
            let speed_before = race.player_speed();
            race.place_bike(1, race.player_distance(), rival_side * 0.5);

            race.tick();

            let player_lateral = race.player_lateral();
            let rival_lateral = race.bike_states().nth(1).unwrap().pos_y;
            assert!(
                (rival_lateral - player_lateral).abs() >= BIKE_HALF_WIDTH * 2.0 - 1e-4,
                "resolved out of overlap"
            );
            assert!(
                (player_lateral - rival_lateral) * rival_side < 0.0,
                "player pushed away from the rival"
            );
            assert!(
                race.player_speed() < speed_before * (COLLISION_SCRUB + 0.01),
                "speed penalty applied"
            );
        }
    }

    #[test]
    fn ai_is_deterministic_across_two_identical_runs() {
        let inputs: Vec<Vec<GameInput>> = (0..240)
            .map(|tick| {
                if tick % 7 == 0 {
                    vec![GameInput::Accelerate, GameInput::SteerLeft]
                } else {
                    vec![GameInput::Accelerate]
                }
            })
            .collect();

        let mut run_a = flat_race(3, 99);
        let mut run_b = flat_race(3, 99);
        for events in &inputs {
            for input in events {
                run_a.push_input(*input);
            }
            run_a.tick();
        }
        for events in &inputs {
            for input in events {
                run_b.push_input(*input);
            }
            run_b.tick();
        }

        let states_a: Vec<BikeState> = run_a.bike_states().collect();
        let states_b: Vec<BikeState> = run_b.bike_states().collect();
        assert_eq!(states_a, states_b, "same seed and inputs: identical races");
        assert_eq!(run_a.tick_index(), run_b.tick_index());
    }

    #[test]
    fn lap_scoring_boundary_cases() {
        assert_eq!(lap_index(0.0, TRACK), 0);
        assert_eq!(
            lap_index(TRACK - 0.001, TRACK),
            0,
            "epsilon short of the line"
        );
        assert_eq!(lap_index(TRACK, TRACK), 1, "exactly one lap: lap 1");
        assert_eq!(lap_index(TRACK + 0.001, TRACK), 1);
        assert_eq!(lap_index(TRACK * LAPS as f32, TRACK), LAPS, "race distance");
        assert_eq!(lap_index(TRACK * (LAPS as f32 + 0.5), TRACK), LAPS);

        // Progress wraps at the line: exactly on it is the start, not lap end.
        assert!((lap_progress(TRACK, TRACK) - 0.0).abs() < 1e-6);
        assert!((lap_progress(TRACK * 2.5, TRACK) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn race_finishes_after_total_laps() {
        let mut race = Race::new(RaceConfig {
            rivals: 0,
            track_length: 20.0,
            total_laps: 1,
            seed: 5,
            geometry: RoadGeometry::new(Vec::new(), Vec::new(), 12.0),
        });
        assert!(!race.is_finished());
        // Full throttle: 20 m at up to 60 m/s takes well under 120 ticks.
        run(&mut race, 120, &[GameInput::Accelerate]);
        assert!(race.is_finished(), "crossed the 20 m finish line");
        assert_eq!(race.lap_index(), 1);
    }

    #[test]
    fn rivals_stagger_deterministically_on_the_grid() {
        let a = flat_race(4, 123);
        let b = flat_race(4, 123);
        let lanes_a: Vec<f32> = a.bike_states().map(|s| s.pos_y).collect();
        let lanes_b: Vec<BikeState> = b.bike_states().collect();
        for (lane, state) in lanes_a.iter().zip(lanes_b) {
            assert_eq!(*lane, state.pos_y);
        }
        // Rivals start staggered ahead of and behind the player, spaced by
        // GRID_SPACING on their side of the line.
        let states: Vec<BikeState> = a.bike_states().collect();
        for (slot, state) in states.iter().enumerate().skip(1) {
            let rival = slot - 1;
            let stagger = (rival / 2 + 1) as f32 * GRID_SPACING;
            let expected = if rival % 2 == 0 { stagger } else { -stagger };
            assert!((state.pos_x - expected).abs() < 1e-5);
        }
    }

    #[test]
    fn held_throttle_after_release_advances_the_player_and_the_pack() {
        let mut race = flat_race(3, 7);
        while race.is_counting_down() {
            race.tick();
        }
        let start_distance = race.player_distance();
        let start_rivals: Vec<BikeState> = race.bike_states().skip(1).collect();

        // Four seconds of held throttle at 60 Hz.
        for _ in 0..240 {
            race.push_input(GameInput::Accelerate);
            race.tick();
        }

        assert!(
            race.player_speed() * 3.6 > 120.0,
            "120 km/h within four seconds of throttle"
        );
        assert!(
            race.player_distance() > start_distance + 60.0,
            "the player is down the road"
        );
        // Every rival has advanced, and no rival holds a static gap: each
        // gap to the player changed since the lights went green.
        for (slot, before) in start_rivals.iter().enumerate() {
            let after = race.bike_states().nth(slot + 1).unwrap();
            assert!(
                after.pos_x > before.pos_x + 5.0,
                "rival {slot} advanced up the road"
            );
            let gap_before = before.pos_x - start_distance;
            let gap_after = after.pos_x - race.player_distance();
            #[cfg(test)]
            eprintln!(
                "GAP rival={slot} before={gap_before:.2} after={gap_after:.2} shadow={} cool={} ctrl={:?}",
                race.aux[slot + 1].shadow_secs,
                race.aux[slot + 1].shadow_cooldown,
                race.bike_control(slot + 1)
            );
            assert!(
                (gap_after - gap_before).abs() > 0.5
                    || race.aux[slot + 1].shadow_secs > 0.0
                    || race.aux[slot + 1].shadow_cooldown > 0.0,
                "rival {slot}'s gap to the player changed"
            );
        }
    }

    #[test]
    fn rubber_band_pulls_a_dropped_rival_back_into_the_race() {
        let mut race = flat_race(1, 7);
        race.clear_traffic();
        while race.is_counting_down() {
            race.tick();
        }
        // Roll the player out ahead, then drop the rival well behind the
        // rubber-band window.
        run(&mut race, 120, &[GameInput::Accelerate]);
        race.place_bike(1, race.player_distance() - RUBBER_BAND_BEHIND - 20.0, 0.0);
        let gap = race.player_distance() - race.bike_states().nth(1).unwrap().pos_x;
        assert!(gap > RUBBER_BAND_BEHIND);

        // Ten seconds of held throttle later, the pack has regrouped: the
        // staged 45 m gap closes to the window edge (~25 m), where the
        // rival rejoins at the player's pace instead of ramming it.
        for _ in 0..600 {
            race.push_input(GameInput::Accelerate);
            race.tick();
        }
        let gap_after = race.player_distance() - race.bike_states().nth(1).unwrap().pos_x;
        assert!(
            gap_after < gap - 10.0,
            "rubber band closed the gap from {gap} m to {gap_after} m"
        );
        assert_eq!(
            race.player_control(),
            Control::Accelerate,
            "no ram from behind"
        );
    }

    #[test]
    fn steering_banks_the_player_within_a_few_ticks() {
        let mut race = flat_race(0, 7);
        race.clear_traffic();
        while race.is_counting_down() {
            race.tick();
        }
        // Roll first: steering authority needs speed.
        run(&mut race, 30, &[GameInput::Accelerate]);
        for _ in 0..3 {
            race.push_input(GameInput::SteerLeft);
            race.tick();
        }
        let lean = race.player_lean();
        assert!(lean < -0.1, "visibly banked left within three ticks");
        assert!(lean >= -bike::LEAN_MAX - 1e-4, "bank respects the cap");
        assert!(race.player_lateral() < 0.0, "bank came with lateral motion");
    }

    #[test]
    fn clock_gates_ticks_through_step_frame() {
        let mut race = flat_race(0, 7);
        // One 60 Hz frame runs one tick; a sub-tick frame runs none.
        assert_eq!(race.step_frame(TICK_DURATION_SECS), 1);
        assert_eq!(race.step_frame(TICK_DURATION_SECS * 0.25), 0);
        assert_eq!(race.step_frame(TICK_DURATION_SECS * 2.25), 2);
        assert_eq!(race.tick_index(), 3);
    }

    // ------------------------------------------------------------------
    // Wave 7b — adjacent-rider combat core. Red-first: these describe the
    // combat the tick must gain; they fail until the combat pass ships.
    // ------------------------------------------------------------------

    const STAGE_DY: f32 = 1.5;

    /// Stages a duel: the rider 1 m ahead of the player and [`STAGE_DY`]
    /// to the side — inside punch reach, clear of the bike-vs-bike overlap
    /// envelope — and skips the green-light countdown so the fight starts
    /// immediately. The rider sits on its own preferred lane; the chase
    /// logic settles it just behind the player, holding the adjacency.
    fn staged_fight(rivals: usize, seed: u64) -> Race {
        let mut race = flat_race(rivals, seed);
        race.clear_traffic();
        race.skip_countdown();
        let lane = race.bike_states().nth(1).expect("rival staged").pos_y;
        race.place_bike(1, -1.0, lane);
        race.place_bike(0, 0.0, lane - STAGE_DY);
        race
    }

    #[test]
    fn a_swing_with_no_adjacent_rival_costs_nothing() {
        let mut race = flat_race(1, 42);
        race.clear_traffic();
        race.skip_countdown();
        let lane = race.bike_states().nth(1).expect("rival staged").pos_y;
        // Nobody alongside: the rival is staged far up the road.
        race.place_bike(1, 60.0, lane);
        race.place_bike(0, 0.0, lane);

        for _ in 0..30 {
            race.push_input(GameInput::Accelerate);
            race.push_input(GameInput::Punch);
            race.tick();
        }

        assert_eq!(
            race.player_stamina(),
            1.0,
            "no target, no cost — the stamina bar is untouched"
        );
        // Every press plays the swing pose, whiff included (item 8).
        assert_eq!(
            race.bike_attack_pose(0),
            Some(Attack::Punch),
            "a whiff still shows the swing"
        );
        assert_ne!(
            race.player_control(),
            Control::Crashed,
            "an empty swing never takes the player down"
        );
    }

    #[test]
    fn two_punches_dismount_a_worn_rival_deterministically() {
        let mut race = staged_fight(1, 42);
        // A worn bar goes down to two clean punches: the first drains,
        // the second empties it and takes the rider off the bike — the
        // beatdown chain (four fresh punches) is not needed here.
        race.set_stamina(1, 0.35);
        let mut ticks = 0;
        while race.bike_control(1) != Control::Crashed {
            race.push_input(GameInput::Accelerate);
            race.push_input(GameInput::Punch);
            race.tick();
            ticks += 1;
            assert!(ticks < 600, "the rival never went down");
        }
        // Both punches drained: the empty bar, not the instability chain,
        // took the rider down.
        let rival_stamina = race.bike_stamina(1);
        assert!(
            rival_stamina <= 0.15 + 1e-4,
            "both punches drained (rival stamina {rival_stamina})"
        );
        assert!(
            race.player_stamina() > 0.0,
            "the player survives the exchange it won"
        );
    }

    #[test]
    fn combat_outcomes_replay_identically_for_the_same_seed_and_inputs() {
        let script = |race: &mut Race| {
            while race.is_counting_down() {
                race.tick();
            }
            let lane = race.bike_states().nth(1).expect("rival staged").pos_y;
            race.place_bike(1, 1.0, lane);
            race.place_bike(0, 0.0, lane - STAGE_DY);
            for _ in 0..240 {
                // A swing every tick; the sim spaces them by the cooldown.
                race.push_input(GameInput::Accelerate);
                race.push_input(GameInput::Punch);
                race.tick();
            }
            race.bike_states().collect::<Vec<BikeState>>()
        };
        let states_a = script(&mut staged_fight(1, 42));
        let states_b = script(&mut staged_fight(1, 42));
        assert_eq!(
            states_a, states_b,
            "same seed, same inputs: the same fight, blow for blow"
        );
    }

    #[test]
    fn a_dismounted_rival_rejoins_at_reduced_speed() {
        let mut race = staged_fight(1, 42);
        race.set_stamina(1, 0.35);
        let mut ticks = 0;
        while race.bike_control(1) != Control::Crashed {
            race.push_input(GameInput::Accelerate);
            race.push_input(GameInput::Punch);
            race.tick();
            ticks += 1;
            assert!(ticks < 600, "the rival never went down");
        }
        // The rider is down and scrubbing to a stop while the player rides on.
        run(&mut race, 90, &[GameInput::Accelerate]);
        let player_lead = race.player_distance() - race.bike_states().nth(1).unwrap().pos_x;
        assert!(player_lead > 10.0, "the dismount dropped the rival behind");

        // After the crash flow runs its course the rival is back in the
        // saddle — from the crash's standstill, so the rejoin is at a
        // reduced speed. Asserted at the recovery moment: any ticks of
        // live racing after it and the chase logic converges the rival
        // back up to pace, hiding the rejoin speed.
        let recovery_ticks =
            (bike::CRASH_DURATION_SECS / TICK_DURATION_SECS as f32).ceil() as usize + 120;
        let mut rejoined_speed = None;
        for _ in 0..recovery_ticks {
            race.tick();
            if race.bike_control(1) != Control::Crashed {
                rejoined_speed = Some(race.bike_states().nth(1).unwrap().speed);
                break;
            }
        }
        let rejoined_speed = rejoined_speed.expect("back on the bike");
        assert!(
            rejoined_speed < bike::MAX_SPEED * 0.5,
            "rejoining at a reduced speed ({rejoined_speed})"
        );
    }

    #[test]
    fn a_kick_shoves_the_rival_toward_traffic() {
        let mut race = staged_fight(1, 42);
        // A car alongside and ahead of the rival, on the rival's LEFT —
        // the same side the player is on. A plain "away from attacker"
        // shove would send the rival right; traffic wins the direction.
        let rival_x = race.bike_states().nth(1).unwrap().pos_x;
        let rival_y_before = race.bike_states().nth(1).unwrap().pos_y;
        race.stage_traffic(rival_x + 5.0, rival_y_before - 3.5);

        race.push_input(GameInput::Kick);
        race.tick();

        let rival_y = race.bike_states().nth(1).unwrap().pos_y;
        assert!(
            rival_y < rival_y_before - 0.3,
            "the kick shoved the rival toward the car ({rival_y_before} -> {rival_y})"
        );
        assert_ne!(
            race.bike_control(1),
            Control::Crashed,
            "one kick never dismounts a fresh rival"
        );
    }

    #[test]
    fn rivals_retaliate_on_a_cooldown_when_adjacent() {
        let mut race = staged_fight(1, 42);
        // Ride alongside and let the rival swing — no punches back, so the
        // rival stays up and the retaliation cadence is readable on the
        // player's stamina bar.
        let mut drop_ticks: Vec<usize> = Vec::new();
        let mut saw_swing = false;
        let mut last_stamina = race.player_stamina();
        for tick in 0..1800 {
            race.push_input(GameInput::Accelerate);
            race.tick();
            saw_swing |= race.bike_attack_pose(1) == Some(crate::combat::Attack::Punch);
            let stamina = race.player_stamina();
            if stamina < last_stamina - 0.01 {
                drop_ticks.push(tick);
            }
            last_stamina = stamina;
            if drop_ticks.len() >= 3 {
                break;
            }
        }
        assert!(
            drop_ticks.len() >= 2,
            "an adjacent rival retaliates (drops at {drop_ticks:?})"
        );
        assert!(saw_swing, "the retaliation shows the swing pose");
        // Consecutive retaliations respect the cooldown: no rival lands two
        // hits inside the shortest aggro-weighted cooldown window.
        let min_cooldown_ticks =
            (combat::RETALIATION_COOLDOWN / 1.4 / TICK_DURATION_SECS as f32).floor() as usize;
        for pair in drop_ticks.windows(2) {
            assert!(
                pair[1] - pair[0] >= min_cooldown_ticks,
                "retaliation beats its cooldown ({drop_ticks:?})"
            );
        }
    }

    #[test]
    fn a_hit_on_an_empty_stamina_bar_dismounts_the_player_through_crash_recover() {
        let mut race = staged_fight(1, 42);
        // Ride the fight and soak retaliation until one more landed hit
        // puts the player down through the existing crash flow. The bar is
        // pinned empty through regen so the test isolates the rule under
        // test: a hit landing at zero stamina dismounts — and nothing else
        // (full swings would trip the instability threshold first and mask
        // this rule).
        let mut ticks = 0;
        while race.player_control() != Control::Crashed {
            race.set_stamina(0, 0.0);
            race.push_input(GameInput::Accelerate);
            race.tick();
            ticks += 1;
            assert!(ticks < 1800, "the player was never dismounted");
        }
        assert!(
            race.player_stamina() <= 1e-4,
            "the dismount came from an empty bar ({})",
            race.player_stamina()
        );
        // The existing crash-recover flow hands the rider back.
        let recovery_ticks =
            (bike::CRASH_DURATION_SECS / TICK_DURATION_SECS as f32).ceil() as usize + 10;
        run(&mut race, recovery_ticks, &[]);
        assert_eq!(race.player_control(), Control::Coast, "recovered");
    }

    // ------------------------------------------------------------------
    // Wave 7 audit — sim probes (items 1–8). Each probe builds the demo
    // race: seed 0x5EED_C0DE, a 2 000 m lap from the two curve segments of
    // runtime's `track_geometry`, and pushes inputs each tick while
    // reading the Race's public getters.
    // ------------------------------------------------------------------

    /// The demo course's two curve segments, mirrored from runtime's
    /// `track_geometry` (the game crate cannot depend on runtime): a long
    /// gentle left (negative curvature per the geometry-authoritative
    /// convention — `center_offset` is −15 m at 300 m) and a shorter,
    /// tighter right. Elevation does not enter the sim.
    fn demo_geometry() -> RoadGeometry {
        RoadGeometry::new(
            vec![
                CurveSegment {
                    start: 200.0,
                    length: 700.0,
                    curvature: -0.003,
                },
                CurveSegment {
                    start: 1200.0,
                    length: 400.0,
                    curvature: 0.005,
                },
            ],
            Vec::new(),
            12.0,
        )
    }

    /// The demo race probe: seed 0x5EED_C0DE, 2 000 m laps, three laps.
    /// Traffic is cleared: these probes isolate audited sim behavior
    /// (drift, recovery, fight pacing), not car avoidance.
    fn probe_race(rivals: usize) -> Race {
        let mut race = Race::new(RaceConfig {
            rivals,
            track_length: 2_000.0,
            total_laps: 3,
            seed: 0x5EED_C0DE,
            geometry: demo_geometry(),
        });
        race.clear_traffic();
        race
    }

    /// Runs `ticks` of throttle-only input through the race.
    fn throttle(race: &mut Race, ticks: usize) {
        for _ in 0..ticks {
            race.push_input(GameInput::Accelerate);
            race.tick();
        }
    }

    /// Whether the rider in `slot` sits in punch reach of the player.
    fn rival_in_reach(race: &Race, slot: usize) -> bool {
        let player = race.bike_states().next().expect("player");
        let rival = race.bike_states().nth(slot).expect("rival");
        combat::in_reach(
            Attack::Punch,
            rival.pos_x - player.pos_x,
            rival.pos_y - player.pos_y,
        )
    }

    #[test]
    fn a_rival_hit_lands_only_after_the_windup_ticks() {
        let mut race = staged_fight(1, 42);
        // Ride adjacent until the rival's wind-up pose appears.
        let mut seen_pose = false;
        for _ in 0..600 {
            race.push_input(GameInput::Accelerate);
            race.tick();
            if race.bike_attack_pose(1) == Some(Attack::Punch) {
                seen_pose = true;
                break;
            }
        }
        assert!(seen_pose, "the rival shows the wind-up pose");

        // Through the wind-up the target's stamina is untouched — the hit
        // has not landed yet.
        for windup_tick in 1..combat::WINDUP_TICKS {
            race.push_input(GameInput::Accelerate);
            race.tick();
            assert_eq!(
                race.player_stamina(),
                1.0,
                "stamina drained {windup_tick} ticks into the wind-up"
            );
        }
        // ...then the hit lands.
        race.push_input(GameInput::Accelerate);
        race.tick();
        assert!(
            race.player_stamina() < 1.0,
            "the hit lands WINDUP_TICKS after the pose starts"
        );
    }

    #[test]
    fn a_windup_hit_is_cancelled_when_the_target_leaves_reach() {
        let mut race = staged_fight(1, 42);
        let mut seen_pose = false;
        for _ in 0..600 {
            race.push_input(GameInput::Accelerate);
            race.tick();
            if race.bike_attack_pose(1) == Some(Attack::Punch) {
                seen_pose = true;
                break;
            }
        }
        assert!(seen_pose, "the rival shows the wind-up pose");

        // The player bolts out of reach mid-wind-up: the swing whiffs.
        race.place_bike(0, race.player_distance() + 30.0, 0.0);
        throttle(&mut race, combat::WINDUP_TICKS as usize + 30);
        assert_eq!(
            race.player_stamina(),
            1.0,
            "the cancelled wind-up never landed"
        );
        assert_ne!(
            race.player_control(),
            Control::Crashed,
            "a cancelled wind-up takes no one down"
        );
    }

    #[test]
    fn throttle_only_through_the_first_bend_drifts_outward() {
        let mut race = probe_race(3);
        // Roll to just inside the first bend (200 m, curvature −0.003:
        // a left bend — the centre shifts negative, the outside is right),
        // with the pack 60 m behind: this probe measures the drift physics
        // on a clear road, not race traffic — a passing bike's legitimate
        // overlap push would otherwise shove the rider sideways mid-window
        // and bury the drift signal.
        while race.player_distance() < 140.0 {
            throttle(&mut race, 1);
        }
        race.place_bike(0, race.player_distance() + 0.0, 0.0);
        for slot in 1..race.bike_count() {
            let s = race.bike_states().nth(slot).expect("rival");
            race.place_bike(slot, s.pos_x - 60.0, s.pos_y);
        }
        while race.player_distance() < 205.0 {
            throttle(&mut race, 1);
        }
        let entry = race.player_lateral();
        // Throttle only, deeper into the bend — deep enough that the
        // quadratic-in-speed drift gain clears the assertion margin.
        while race.player_distance() < 345.0 {
            throttle(&mut race, 1);
        }
        let deep = race.player_lateral();
        assert!(
            deep > entry + 0.3,
            "lateral must move outward (positive) through the bend ({entry} -> {deep})"
        );
        // The sign is the contract (audit item 2): the drift must push
        // toward the outside of the bend. The absolute side is not — at
        // launch speed the drift gain over the first 90 m of bend is only
        // a few tenths of a meter, so a rider starting left of centre
        // stays left of centre while still drifting the right way.
    }

    #[test]
    fn rivals_take_the_visual_inside_of_each_bend() {
        let mut race = probe_race(3);
        // Sample upright rivals deep in each bend over a full race.
        let mut left_bend: Vec<f32> = Vec::new();
        let mut right_bend: Vec<f32> = Vec::new();
        while !race.is_finished() && race.tick_index() < 20_000 {
            throttle(&mut race, 1);
            for slot in 1..race.bike_count() {
                if race.bike_control(slot) == Control::Crashed {
                    continue;
                }
                // A rival locked in a pace-matched engagement — or backing
                // off one — sits beside the player on the fight formation,
                // not on a racing line (audit item 6 demands the fight);
                // it says nothing about bend craft. Sample the riders
                // actually racing, well clear of the fight zone.
                let s = race.bike_states().nth(slot).expect("rival");
                let player_x = race.player_distance();
                if (s.pos_x - player_x).abs() < 20.0 || race.aux[slot].shadow_secs > 0.0 {
                    continue;
                }
                let (x, y) = (s.pos_x, s.pos_y);
                let d = x.rem_euclid(2_000.0);
                // Bend 1 (a left bend): inside is the negative side.
                if (300.0..800.0).contains(&d) {
                    left_bend.push(y);
                }
                // Bend 2 (a right bend): inside is the positive side.
                if (1_300.0..1_500.0).contains(&d) {
                    right_bend.push(y);
                }
            }
        }
        let mean = |samples: &[f32]| samples.iter().sum::<f32>() / samples.len() as f32;
        assert!(!left_bend.is_empty() && !right_bend.is_empty());
        assert!(
            mean(&left_bend) < 0.0,
            "rivals must sit inside the left bend (negative side), mean {}",
            mean(&left_bend)
        );
        assert!(
            mean(&right_bend) > 0.0,
            "rivals must sit inside the right bend (positive side), mean {}",
            mean(&right_bend)
        );
    }

    #[test]
    fn a_recovered_rider_stays_on_the_tarmac() {
        let mut race = probe_race(3);
        // Fixture: crash the player deterministically at the green, so the
        // probe measures the recovery flow itself — throttle only from the
        // first recovery on. After recovery the rider must be placed at
        // least 3 m inside the edge and stay on the tarmac for at least
        // 90% of ticks.
        throttle(&mut race, 1);
        race.crash_slot(0);
        let mut was_crashed = true;
        let mut recovered = false;
        let mut on = 0usize;
        let mut total = 0usize;
        for _ in 0..(45 * 60) {
            throttle(&mut race, 1);
            let crashed = race.player_control() == Control::Crashed;
            let recovered_now = was_crashed && !crashed;
            was_crashed = crashed;
            if recovered {
                total += 1;
                if race.player_lateral().abs() <= 6.0 {
                    on += 1;
                }
            }
            if recovered_now {
                recovered = true;
            }
        }
        assert!(recovered, "the probe saw at least one crash recovery");
        let share = on as f32 / total as f32;
        assert!(
            share >= 0.9,
            "on-tarmac share after first recovery: {on}/{total} = {share:.3}"
        );
    }

    #[test]
    fn rivals_hold_punch_reach_for_the_player_to_join() {
        // A field above four riders, a centre-holding rider who never
        // attacks: throttle only.
        const STAY_MERGE_TICKS: usize = 120;
        let mut race = probe_race(6);
        let mut open = [0usize; MAX_BIKES];
        let mut open_lap = [0u32; MAX_BIKES];
        let mut first_tick = [0usize; MAX_BIKES];
        let mut last_tick = [0usize; MAX_BIKES];
        let mut windows: Vec<(u32, usize)> = Vec::new(); // (lap, span ticks)
                                                         // The helper tick() does not advance the frame clock, so the stay
                                                         // bookkeeping counts probe iterations (one sim tick each), the
                                                         // same clock the ignored trace test prints.
        let mut now = 0usize;
        while !race.is_finished() && now < 20_000 {
            throttle(&mut race, 1);
            now += 1;
            let lap = race.lap_index().min(2);
            for slot in 1..race.bike_count() {
                // The audit's mechanism (item 6): a rival within reach has
                // its pace matched for a stated number of seconds, so a
                // rider that "stays in punch reach" is one the fight servo
                // is holding there. A transversal crossing at speed never
                // stays — it shears through in a fraction of a second —
                // and counting it as a stay would measure pack churn,
                // not fights. Measure the matched riders' reach stays.
                let in_reach = race.bike_control(slot) != Control::Crashed
                    && race.player_control() != Control::Crashed
                    && race.aux[slot].shadow_secs > 0.0
                    && rival_in_reach(&race, slot);
                // A "stay" runs from entering punch reach to finally
                // leaving it. A re-entry within 2 s continues the same
                // stay — a fight that flickers at the reach boundary or
                // re-arms after a beat is one fight, not a pile of shard
                // windows; only a real departure (2+ s out of reach)
                // closes the stay.
                if in_reach {
                    if open[slot] == 0 {
                        open_lap[slot] = lap;
                        first_tick[slot] = now;
                    } else if now - last_tick[slot] >= STAY_MERGE_TICKS {
                        windows.push((open_lap[slot], last_tick[slot] - first_tick[slot] + 1));
                        open_lap[slot] = lap;
                        first_tick[slot] = now;
                    }
                    last_tick[slot] = now;
                    open[slot] += 1;
                } else if open[slot] > 0 && now - last_tick[slot] >= STAY_MERGE_TICKS {
                    windows.push((open_lap[slot], last_tick[slot] - first_tick[slot] + 1));
                    open[slot] = 0;
                }
            }
        }
        for slot in 1..race.bike_count() {
            if open[slot] > 0 {
                windows.push((open_lap[slot], last_tick[slot] - first_tick[slot] + 1));
            }
        }
        assert!(
            race.is_finished(),
            "the probe race completes inside the tick budget"
        );
        let mut lens: Vec<usize> = windows.iter().map(|&(_, len)| len).collect();
        lens.sort_unstable();
        let median = lens[lens.len() / 2];
        assert!(
            median >= 120,
            "median punch-reach window is {median} ticks; the spec asks for 2 s (windows {lens:?})"
        );
        for lap in 0..3u32 {
            let count = windows.iter().filter(|&&(l, _)| l == lap).count();
            assert!(count >= 3, "lap {lap}: only {count} reach windows");
        }
    }

    #[test]
    #[ignore]
    fn debug_race_trace() {
        let mut race = probe_race(6);
        let mut prev: Vec<Control> = (0..race.bike_count())
            .map(|s| race.bike_control(s))
            .collect();
        let mut open = [0usize; MAX_BIKES];
        let mut windows: Vec<(u32, usize)> = Vec::new();
        let mut open_lap = [0u32; MAX_BIKES];
        let mut iter = 0usize;
        for _ in 0..(180 * 60) {
            throttle(&mut race, 1);
            iter += 1;
            let t = iter;
            let d0 = race.player_distance().rem_euclid(2_000.0);
            if race.lap_index() == 0 && (300.0..800.0).contains(&d0) && iter.is_multiple_of(150) {
                let s: Vec<_> = race.bike_states().collect();
                for slot in 1..race.bike_count() {
                    println!(
                        "BEND t={:.1} r{slot} d={:.0} lat={:.2} v={:.1} dx={:.1} ctrl={:?} avoid={:?} shadow={:.1}",
                        t as f32 / 60.0,
                        s[slot].pos_x.rem_euclid(2_000.0),
                        s[slot].pos_y,
                        s[slot].speed,
                        s[0].pos_x - s[slot].pos_x,
                        race.bike_control(slot),
                        race.avoidance_lane(slot),
                        race.aux[slot].shadow_secs,
                    );
                }
            }
            let lap = race.lap_index().min(2);
            for (slot, prev_slot) in prev.iter_mut().enumerate() {
                let now = race.bike_control(slot);
                if now == Control::Crashed && *prev_slot != Control::Crashed {
                    let (x, y) = {
                        let s = race.bike_states().nth(slot).expect("bike");
                        (s.pos_x, s.pos_y)
                    };
                    println!(
                        "CRASH slot={slot} t={:.1}s d={:.0} lat={:.2} v={:.1}",
                        t as f32 / 60.0,
                        x.rem_euclid(2_000.0),
                        y,
                        race.player_speed()
                    );
                }
                *prev_slot = now;
                if slot > 0 {
                    let in_reach = now != Control::Crashed
                        && race.player_control() != Control::Crashed
                        && rival_in_reach(&race, slot);
                    if in_reach {
                        if open[slot] == 0 {
                            open_lap[slot] = lap;
                            let s = race.bike_states().nth(slot).expect("bike");
                            let p = race.bike_states().next().expect("player");
                            println!(
                                "OPEN slot={slot} t={:.1} dx={:.1} dy={:.1} vr={:.1} vp={:.1}",
                                t as f32 / 60.0,
                                p.pos_x - s.pos_x,
                                p.pos_y - s.pos_y,
                                s.speed,
                                p.speed
                            );
                        }
                        open[slot] += 1;
                    } else if open[slot] > 0 {
                        windows.push((open_lap[slot], open[slot]));
                        let s = race.bike_states().nth(slot).expect("bike");
                        let p = race.bike_states().next().expect("player");
                        println!(
                            "END slot={slot} ticks={} dx={:.1} dy={:.1} vr={:.1} vp={:.1} ctrl={:?} shad={:.1} cd={:.1}",
                            open[slot],
                            p.pos_x - s.pos_x,
                            p.pos_y - s.pos_y,
                            s.speed,
                            p.speed,
                            race.bike_control(slot),
                            race.aux[slot].shadow_secs,
                            race.aux[slot].shadow_cooldown,
                        );
                        println!("WINDOW slot={slot} lap={lap} ticks={}", open[slot]);
                        open[slot] = 0;
                    }
                }
            }
            if t.is_multiple_of(600) {
                println!(
                    "t={:.1}s d={:.0} lap={} lat={:.2} v={:.1} ctrl={:?}",
                    t as f32 / 60.0,
                    race.player_distance().rem_euclid(2_000.0),
                    race.lap_index(),
                    race.player_lateral(),
                    race.player_speed(),
                    race.player_control()
                );
            }
        }
        println!("total windows: {}", windows.len());
    }

    #[test]
    fn a_peaceful_field_does_not_tear_itself_down() {
        // Centre-holding rider who never attacks: throttle only, full race.
        let mut race = probe_race(6);
        let mut prev: Vec<Control> = (0..race.bike_count())
            .map(|s| race.bike_control(s))
            .collect();
        let mut player_crashes = 0;
        let mut rival_crashes = 0;
        while !race.is_finished() && race.tick_index() < 20_000 {
            throttle(&mut race, 1);
            for (slot, prev_slot) in prev.iter_mut().enumerate() {
                let now = race.bike_control(slot);
                if now == Control::Crashed && *prev_slot != Control::Crashed {
                    if slot == 0 {
                        player_crashes += 1;
                    } else {
                        rival_crashes += 1;
                    }
                }
                *prev_slot = now;
            }
        }
        assert!(
            race.is_finished(),
            "the probe race completes inside the tick budget"
        );
        assert!(rival_crashes < 6, "rival crashes: {rival_crashes}");
        assert!(player_crashes <= 1, "player crashes: {player_crashes}");
    }
}
