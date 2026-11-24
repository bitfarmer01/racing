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
use crate::input::{GameInput, Intent};

/// Bike collision half-length along the road, in meters.
pub const BIKE_HALF_LENGTH: f32 = 1.1;
/// Bike collision half-width across the road, in meters.
pub const BIKE_HALF_WIDTH: f32 = 0.5;
/// Fraction of speed each overlapping bike loses per overlapping tick.
pub const COLLISION_SCRUB: f32 = 0.985;
/// Longitudinal gap between rivals on the starting grid, in meters.
pub const GRID_SPACING: f32 = 6.0;
/// Half the lateral span of lanes the grid spreads across, in meters.
pub const GRID_HALF_SPAN: f32 = 3.0;
/// Closing speed above which running into the bike ahead is a crash, in m/s.
pub const CRASH_CLOSING_SPEED: f32 = 12.0;
/// Speed above which a heavy side-by-side overlap takes both riders down, in
/// m/s.
pub const SIDE_CRASH_SPEED: f32 = 28.0;
/// Lateral gap under which side-by-side contact counts as heavy, in meters.
pub const SIDE_CRASH_GAP: f32 = 0.6;
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
}

impl Default for RiderAux {
    fn default() -> Self {
        Self {
            snapshot: BikeSnapshot::at_rest(),
            target_lane: 0.0,
            top_speed: MAX_SPEED,
            aggression: 1.0,
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
        for rival in 0..rivals {
            let lane = rng.next_f32_in(-GRID_HALF_SPAN, GRID_HALF_SPAN);
            let state = BikeState {
                pos_x: (rival + 1) as f32 * GRID_SPACING,
                pos_y: lane,
                speed: 0.0,
                heading: 0.0,
            };
            let slot = rival + 1;
            handles[slot] = Some(pool.spawn(state).expect("fresh pool has room"));
            aux[slot].target_lane = lane;
            // Rider variety: each rival gets a top speed and an aggression,
            // so the pack spreads out instead of riding as one block.
            aux[slot].top_speed = MAX_SPEED * rng.next_f32_in(0.78, 0.97);
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

        // Player (slot 0) follows the drained intent; rivals follow the AI.
        for slot in 0..self.bike_count {
            let state = self.state(slot);
            let mut snapshot = self.aux[slot].snapshot;
            snapshot.speed = state.speed;
            snapshot.lateral = state.pos_y;

            // Catch-up: rivals far behind get a mild boost, leaders ease off,
            // so the pack stays in the race without teleporting.
            let catch_up = if slot == 0 {
                1.0
            } else if state.pos_x < player_x - 30.0 {
                1.10
            } else if state.pos_x > player_x + 80.0 {
                0.92
            } else {
                1.0
            };

            let intent = if slot == 0 {
                intent
            } else {
                ai::decide(
                    snapshot.speed,
                    snapshot.lateral,
                    state.pos_x,
                    self.aux[slot].target_lane,
                    self.aux[slot].top_speed * catch_up,
                    &self.geometry,
                )
            };

            let curvature = self.geometry.curvature_at(state.pos_x);
            let off_road = snapshot.is_off_road(half_width);
            let mut snapshot = bike::step_bike(snapshot, intent, off_road, curvature, dt);
            if slot != 0 {
                // Rivals never exceed their (possibly catch-up-adjusted) cap.
                snapshot.speed = snapshot.speed.min(self.aux[slot].top_speed * catch_up);
            }
            if bike::should_crash(&snapshot, half_width) {
                bike::crash(&mut snapshot);
            }

            // Integrate forward motion: speed is meters per second.
            let pos_x = state.pos_x + snapshot.speed * dt;
            self.aux[slot].snapshot = snapshot;
            self.pool.set(
                self.handle(slot),
                BikeState {
                    pos_x,
                    pos_y: snapshot.lateral,
                    speed: snapshot.speed,
                    heading: 0.0,
                },
            );
        }

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

                let dy = (b.pos_y - a.pos_y).abs();
                // The rider behind is the rammer when speeds differ.
                let (rear, _front) = if a.pos_x <= b.pos_x { (i, j) } else { (j, i) };
                let closing =
                    self.state(rear).speed - self.state(if rear == i { j } else { i }).speed;

                if closing > CRASH_CLOSING_SPEED {
                    // Rammed from behind at speed: the rammer goes down.
                    self.crash_slot(rear);
                    // The rammed rider barely notices.
                    self.scrub_speed(if rear == i { j } else { i }, 0.97);
                } else if dy < SIDE_CRASH_GAP
                    && self.state(i).speed.min(self.state(j).speed) > SIDE_CRASH_SPEED
                {
                    // Heavy side-by-side lean at speed takes both down.
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

    /// Put one bike (spawn-order slot) into a crash.
    fn crash_slot(&mut self, slot: usize) {
        if self.aux[slot].snapshot.control == Control::Crashed {
            return;
        }
        bike::crash(&mut self.aux[slot].snapshot);
    }

    /// Test seam: removes all traffic so handling tests steer without
    /// running into cars.
    #[cfg(test)]
    pub(crate) fn clear_traffic(&mut self) {
        self.traffic.clear();
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

    /// The player's speed, in meters per second.
    pub fn player_speed(&self) -> f32 {
        self.state(0).speed
    }

    /// The player's lateral position, in meters.
    pub fn player_lateral(&self) -> f32 {
        self.state(0).pos_y
    }

    /// Seconds left on the player's crash timer, or 0 when upright.
    pub fn player_crash_timer(&self) -> f32 {
        if self.aux[0].snapshot.control == Control::Crashed {
            self.aux[0].snapshot.crash_timer
        } else {
            0.0
        }
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
        run(&mut race, 70, &[GameInput::Accelerate]);
        // Stage the rival just ahead, stopped, in the player's line.
        race.place_bike(1, race.player_distance() + 2.0, 0.0);
        let player_speed = race.player_speed();
        assert!(player_speed > 20.0);

        race.tick();

        assert_eq!(race.player_control(), Control::Crashed, "rammer down");
    }

    #[test]
    fn running_into_traffic_at_speed_is_a_crash() {
        let mut race = flat_race(0, 42);
        run(&mut race, 1, &[]);
        // Stage the player 30 m behind a car, in its lane, from a stop: full
        // throttle reaches 40+ m/s before catching the car, way over the
        // 12 m/s closing-speed crash threshold.
        let car = race.traffic_cars()[0].state;
        race.place_bike(0, car.pos_x - 30.0, car.pos_y);
        run(&mut race, 200, &[GameInput::Accelerate]);
        assert_eq!(race.player_control(), Control::Crashed, "traffic bites");
    }

    #[test]
    fn race_position_counts_riders_ahead() {
        let mut race = flat_race(2, 7);
        assert_eq!(race.race_position(), 3, "rivals start ahead on the grid");
        // Push the player just past both rivals.
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
        // Three steering pulses in one tick count once.
        for _ in 0..3 {
            race.push_input(GameInput::SteerRight);
        }
        race.tick();
        let one_pulse = race.player_lateral();
        assert!((one_pulse - bike::STEER_RATE * TICK_DURATION_SECS as f32).abs() < 1e-5);

        // The next tick sees nothing: coast, no further lateral motion.
        race.tick();
        assert_eq!(race.player_lateral(), one_pulse);
        assert_eq!(race.player_control(), Control::Coast);
    }

    #[test]
    fn off_road_at_low_speed_slows_without_crashing() {
        let mut race = flat_race(0, 7);
        race.clear_traffic();
        // Throttle + steer right together: ~9 m/s forward (under the 30 m/s
        // crash threshold) while full steering authority walks the bike off
        // the road. 6 m of road edge at up to 5 m/s lateral takes ~80 ticks;
        // 150 is safe margin.
        for _ in 0..150 {
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
        // Hold right until off-road.
        race.clear_traffic();
        let mut steering = 0;
        while race.player_lateral() <= 6.0 {
            run(&mut race, 1, &[GameInput::SteerRight]);
            steering += 1;
            assert!(steering < 200, "never left the road");
        }
        assert_eq!(race.player_control(), Control::Crashed);

        // After the crash duration the rider is back on coast.
        let recovery_ticks =
            (bike::CRASH_DURATION_SECS / TICK_DURATION_SECS as f32).ceil() as usize + 10;
        run(&mut race, recovery_ticks, &[]);
        assert_eq!(race.player_control(), Control::Coast);
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
        // Rivals start ahead of the player, evenly spaced longitudinally.
        let states: Vec<BikeState> = a.bike_states().collect();
        for (slot, state) in states.iter().enumerate().skip(1) {
            assert!((state.pos_x - slot as f32 * GRID_SPACING).abs() < 1e-5);
        }
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
}
