//! The race: grid, countdown, laps, positions, bike contact, combat, results.

use glam::Vec2;
use sim::rng::XorShift64;

use crate::ai::Rival;
use crate::bike::{wrap_angle, Bike, Controls, Tuning};
use crate::track::{Track, TrackDef};

pub const DT: f32 = 1.0 / 120.0;
pub const COUNTDOWN_SECS: f32 = 3.0;
pub const PLAYER: usize = 0;
pub const RIVAL_COUNT: usize = 7;
/// Two contact circles per bike (front and rear), this far either side of centre.
const CONTACT_OFFSET: f32 = 0.55;
const CONTACT_RADIUS: f32 = 0.48;
/// Swing timing: the blow lands this far into the swing.
pub const SWING_SECS: f32 = 0.42;
const SWING_HIT_AT: f32 = 0.16;
const ATTACK_COOLDOWN: f32 = 0.55;
const HIT_DAMAGE: f32 = 36.0;
const BALANCE_REGEN: f32 = 9.0;

pub const RIVAL_NAMES: [&str; RIVAL_COUNT] =
    ["Vex", "Kato", "Marlow", "Juno", "Brask", "Silva", "Reyes"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    Countdown {
        t: f32,
    },
    Racing,
    /// The player has finished; the field rides on.
    Finished {
        t: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventKind {
    WallHit { speed: f32 },
    BikeContact { speed: f32 },
    Hit { attacker: usize },
    Crash,
    Landed { speed: f32 },
    LapComplete { lap: u32, time: f32 },
    Finished { position: usize },
    Overtake { gained: bool },
    CountdownBeep { n: u32 },
    Go,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    pub bike: usize,
    pub kind: EventKind,
}

#[derive(Debug, Clone)]
pub struct Racer {
    pub name: &'static str,
    /// Metres covered since the start line (negative on the grid).
    pub progress: f32,
    pub lap_times: Vec<f32>,
    pub lap_started: f32,
    pub finish_time: Option<f32>,
    pub balance: f32,
    pub since_hit: f32,
    pub swing: f32,
    pub swing_side: f32,
    pub swing_landed: bool,
    pub attack_cooldown: f32,
    pub wrong_way: f32,
    pub hits_landed: u32,
    pub knockdowns: u32,
}

impl Racer {
    fn new(name: &'static str, progress: f32) -> Self {
        Self {
            name,
            progress,
            lap_times: Vec::new(),
            lap_started: 0.0,
            finish_time: None,
            balance: 100.0,
            since_hit: 10.0,
            swing: 0.0,
            swing_side: 1.0,
            swing_landed: false,
            attack_cooldown: 0.0,
            wrong_way: 0.0,
            hits_landed: 0,
            knockdowns: 0,
        }
    }

    /// Lap the racer is on (1-based), clamped to the race length.
    pub fn lap(&self, track_len: f32, laps: u32) -> u32 {
        ((self.progress / track_len).floor() as i64 + 1).clamp(1, laps as i64) as u32
    }

    pub fn swinging(&self) -> bool {
        self.swing > 0.0
    }
}

pub struct Race {
    pub track: Track,
    pub bikes: Vec<Bike>,
    pub racers: Vec<Racer>,
    pub rivals: Vec<Option<Rival>>,
    pub phase: Phase,
    /// Race clock, seconds since the green light.
    pub clock: f32,
    pub laps: u32,
    /// Bike indices by race position (leader first).
    pub order: Vec<usize>,
    pub events: Vec<Event>,
    rng: XorShift64,
    last_player_place: usize,
    beeps: u32,
}

impl Race {
    pub fn new(def: TrackDef, seed: u64) -> Self {
        Self::with_track(Track::build(def), seed)
    }

    pub fn with_track(track: Track, seed: u64) -> Self {
        let mut rng = XorShift64::new(seed);
        let n = RIVAL_COUNT + 1;
        let mut bikes = Vec::with_capacity(n);
        let mut racers = Vec::with_capacity(n);
        let mut rivals = Vec::with_capacity(n);
        // Player starts at the back of the grid: something to fight through.
        for slot in 0..n {
            let row = (n - 1 - slot) as f32;
            let back = 12.0 + row * 7.0;
            let lateral = if slot % 2 == 0 { -2.6 } else { 2.6 };
            let s = track.length - back;
            let (tuning, rival) = if slot == PLAYER {
                (Tuning::default(), None)
            } else {
                // Front of the grid is quicker; every rival differs a little.
                let quality = 1.0 - row / n as f32;
                let tuning = Tuning {
                    power: 0.93 + 0.07 * quality + rng.next_f32_in(-0.02, 0.02),
                    grip: 0.95 + 0.05 * quality,
                    top_speed: 0.95 + 0.05 * quality + rng.next_f32_in(-0.01, 0.01),
                };
                let lane = rng.next_f32_in(-0.9, 0.9);
                let commitment = 0.80 + 0.14 * quality + rng.next_f32_in(-0.02, 0.02);
                let aggression = rng.next_f32_in(0.25, 0.95);
                (tuning, Some(Rival::new(lane, commitment, aggression)))
            };
            bikes.push(Bike::new(&track, s, lateral, tuning));
            let name = if slot == PLAYER {
                "You"
            } else {
                RIVAL_NAMES[slot - 1]
            };
            racers.push(Racer::new(name, -back));
            rivals.push(rival);
        }
        let laps = track.def.laps;
        let mut race = Self {
            track,
            bikes,
            racers,
            rivals,
            phase: Phase::Countdown { t: 0.0 },
            clock: 0.0,
            laps,
            order: (0..n).collect(),
            events: Vec::with_capacity(32),
            rng,
            last_player_place: n,
            beeps: 0,
        };
        race.rank();
        race.last_player_place = race.place_of(PLAYER);
        race
    }

    pub fn player(&self) -> &Bike {
        &self.bikes[PLAYER]
    }

    /// 1-based race position of a bike.
    pub fn place_of(&self, bike: usize) -> usize {
        self.order.iter().position(|&b| b == bike).unwrap_or(0) + 1
    }

    pub fn race_distance(&self) -> f32 {
        self.laps as f32 * self.track.length
    }

    /// Advances the race one fixed tick. Events are cleared first.
    pub fn tick(&mut self, player_input: &Controls) {
        self.events.clear();
        match self.phase {
            Phase::Countdown { t } => {
                let t = t + DT;
                // Three beeps a second apart, then GO.
                if self.beeps < 3 && t >= self.beeps as f32 {
                    let n = 3 - self.beeps;
                    self.beeps += 1;
                    self.push(PLAYER, EventKind::CountdownBeep { n });
                }
                if t >= COUNTDOWN_SECS {
                    self.phase = Phase::Racing;
                    self.push(PLAYER, EventKind::Go);
                } else {
                    self.phase = Phase::Countdown { t };
                    // Revving on the line: wheels held.
                    for b in &mut self.bikes {
                        b.throttle = 0.6;
                        b.rpm += (6_500.0 - b.rpm) * 0.05;
                    }
                    self.bikes[PLAYER].throttle = player_input.throttle;
                    self.bikes[PLAYER].rpm = 2_000.0 + player_input.throttle * 9_000.0;
                    return;
                }
            }
            Phase::Racing => {}
            Phase::Finished { t } => self.phase = Phase::Finished { t: t + DT },
        }
        self.clock += DT;
        self.step_bikes(player_input);
        self.contacts();
        self.combat();
        self.progress();
        self.rank();
    }

    fn push(&mut self, bike: usize, kind: EventKind) {
        self.events.push(Event { bike, kind });
    }

    fn step_bikes(&mut self, player_input: &Controls) {
        let n = self.bikes.len();
        let player_finished = self.racers[PLAYER].finish_time.is_some();
        for i in 0..n {
            let controls = if i == PLAYER && !player_finished {
                *player_input
            } else {
                let roll = self.rng.next_f32();
                let others: Vec<&Bike> =
                    (0..n).filter(|&j| j != i).map(|j| &self.bikes[j]).collect();
                let rival = self.rivals[i].get_or_insert_with(|| Rival::new(0.0, 0.88, 0.0));
                let mut c = rival.drive(&self.bikes[i], &others, &self.track, DT, roll);
                // Mild catch-up: rivals far behind the player push harder,
                // rivals far ahead ease off. Never decisive.
                let gap = self.racers[PLAYER].progress - self.racers[i].progress;
                if i != PLAYER {
                    let t = &mut self.bikes[i].tuning;
                    let base = t.power.clamp(0.85, 1.08);
                    t.power = if gap > 120.0 {
                        base.max(1.04)
                    } else if gap < -160.0 {
                        base.min(0.94)
                    } else {
                        base
                    };
                }
                if self.racers[i].finish_time.is_some() {
                    c.throttle *= 0.55;
                    c.attack = false;
                    c.boost = false;
                }
                c
            };
            // Slipstream: tucked in behind another bike.
            let me = &self.bikes[i];
            let mut drafting = false;
            if me.forward_speed() > 25.0 {
                for j in 0..n {
                    if j == i || !self.bikes[j].is_riding() {
                        continue;
                    }
                    let d = self
                        .track
                        .delta_s(me.track_pos.s, self.bikes[j].track_pos.s);
                    let lat = (self.bikes[j].track_pos.lateral - me.track_pos.lateral).abs();
                    if (3.0..22.0).contains(&d) && lat < 1.3 {
                        drafting = true;
                    }
                }
            }
            let bike = &mut self.bikes[i];
            bike.drafting = drafting;
            if drafting {
                bike.boost = (bike.boost + 0.06 * DT).min(1.0);
            }
            if controls.attack && bike.is_riding() {
                self.start_swing(i);
            }
            let bike = &mut self.bikes[i];
            let fx = bike.step(&self.track, &controls, DT);
            if fx.wall_hit > 2.0 {
                self.push(i, EventKind::WallHit { speed: fx.wall_hit });
            }
            if fx.crashed {
                self.racers[i].knockdowns += 1;
                self.push(i, EventKind::Crash);
            }
            if fx.landed > 0.0 {
                self.push(i, EventKind::Landed { speed: fx.landed });
            }
        }
    }

    fn start_swing(&mut self, i: usize) {
        let r = &mut self.racers[i];
        if r.swing > 0.0 || r.attack_cooldown > 0.0 {
            return;
        }
        // Swing towards the nearest bike alongside (default: right).
        let me = &self.bikes[i];
        let mut side = 1.0;
        let mut best = f32::MAX;
        for (j, o) in self.bikes.iter().enumerate() {
            if j == i || !o.is_solid() {
                continue;
            }
            let rel = o.pos2() - me.pos2();
            let d = rel.length();
            if d < best && d < 4.0 {
                best = d;
                side = if rel.dot(me.right()) >= 0.0 {
                    1.0
                } else {
                    -1.0
                };
            }
        }
        let r = &mut self.racers[i];
        r.swing = SWING_SECS;
        r.swing_side = side;
        r.swing_landed = false;
        r.attack_cooldown = ATTACK_COOLDOWN + SWING_SECS;
    }

    fn combat(&mut self) {
        let n = self.bikes.len();
        for i in 0..n {
            let r = &mut self.racers[i];
            r.attack_cooldown = (r.attack_cooldown - DT).max(0.0);
            r.since_hit += DT;
            if r.since_hit > 1.2 {
                r.balance = (r.balance + BALANCE_REGEN * DT).min(100.0);
            }
            if r.swing <= 0.0 {
                continue;
            }
            let before = r.swing;
            r.swing = (r.swing - DT).max(0.0);
            let elapsed = SWING_SECS - r.swing;
            let crossed = SWING_SECS - before < SWING_HIT_AT && elapsed >= SWING_HIT_AT;
            if !crossed || r.swing_landed || !self.bikes[i].is_riding() {
                continue;
            }
            let side = r.swing_side;
            let me = self.bikes[i].clone();
            for j in 0..n {
                if j == i || !self.bikes[j].is_solid() {
                    continue;
                }
                let rel = self.bikes[j].pos2() - me.pos2();
                let along = rel.dot(me.fwd());
                let across = rel.dot(me.right()) * side;
                if along.abs() < 1.9 && (0.3..2.7).contains(&across) {
                    self.racers[i].swing_landed = true;
                    self.racers[i].hits_landed += 1;
                    self.bikes[i].boost = (self.bikes[i].boost + 0.12).min(1.0);
                    let target = &mut self.bikes[j];
                    let shove = me.right() * side * 3.2;
                    target.vel += shove;
                    target.yaw_rate += side * 0.6;
                    let tr = &mut self.racers[j];
                    tr.balance -= HIT_DAMAGE;
                    tr.since_hit = 0.0;
                    self.push(j, EventKind::Hit { attacker: i });
                    if self.racers[j].balance <= 0.0 {
                        self.racers[j].balance = 100.0;
                        self.racers[j].knockdowns += 1;
                        self.bikes[j].crash();
                        self.push(j, EventKind::Crash);
                    }
                    break;
                }
            }
        }
    }

    fn contacts(&mut self) {
        let n = self.bikes.len();
        for i in 0..n {
            for j in (i + 1)..n {
                if !self.bikes[i].is_solid() || !self.bikes[j].is_solid() {
                    continue;
                }
                let (a, b) = (&self.bikes[i], &self.bikes[j]);
                if a.pos2().distance_squared(b.pos2()) > 9.0 || (a.pos.y - b.pos.y).abs() > 1.5 {
                    continue;
                }
                // Deepest overlap between the two bikes' contact circles.
                let mut deepest: Option<(Vec2, f32)> = None;
                for sa in [-1.0, 1.0] {
                    for sb in [-1.0, 1.0] {
                        let pa = a.pos2() + a.fwd() * CONTACT_OFFSET * sa;
                        let pb = b.pos2() + b.fwd() * CONTACT_OFFSET * sb;
                        let d = pb - pa;
                        let dist = d.length();
                        let pen = CONTACT_RADIUS * 2.0 - dist;
                        if pen > 0.0 && deepest.is_none_or(|(_, p)| pen > p) {
                            let nrm = if dist > 1e-4 { d / dist } else { a.right() };
                            deepest = Some((nrm, pen));
                        }
                    }
                }
                let Some((nrm, pen)) = deepest else { continue };
                let rel = (b.vel - a.vel).dot(nrm);
                let (a, b) = split_two(&mut self.bikes, i, j);
                a.pos.x -= nrm.x * pen * 0.5;
                a.pos.z -= nrm.y * pen * 0.5;
                b.pos.x += nrm.x * pen * 0.5;
                b.pos.z += nrm.y * pen * 0.5;
                if rel < 0.0 {
                    let impulse = -rel * 0.6;
                    a.vel -= nrm * impulse;
                    b.vel += nrm * impulse;
                    // Side swipes knock the bikes off line a touch.
                    a.yaw_rate -= nrm.dot(a.right()) * 0.25;
                    b.yaw_rate += nrm.dot(b.right()) * 0.25;
                    if -rel > 1.0 {
                        self.events.push(Event {
                            bike: i,
                            kind: EventKind::BikeContact { speed: -rel },
                        });
                        self.events.push(Event {
                            bike: j,
                            kind: EventKind::BikeContact { speed: -rel },
                        });
                    }
                }
            }
        }
    }

    fn progress(&mut self) {
        let len = self.track.length;
        let total = self.race_distance();
        for i in 0..self.bikes.len() {
            let bike = &self.bikes[i];
            let racer = &self.racers[i];
            let current_s = self.track.wrap_s(racer.progress);
            let ds = self
                .track
                .delta_s(current_s, bike.track_pos.s)
                .clamp(-6.0, 6.0);
            let before = racer.progress;
            let after = before + ds;
            // Facing against the track direction while moving.
            let along = bike.fwd().dot(self.track.frame_at(bike.track_pos.s).fwd);
            let wrong = bike.is_riding() && along < -0.3 && bike.speed() > 3.0;
            let racer = &mut self.racers[i];
            racer.wrong_way = if wrong { racer.wrong_way + DT } else { 0.0 };
            if racer.finish_time.is_some() {
                racer.progress = after.max(total);
                continue;
            }
            racer.progress = after;
            let lap_before = (before / len).floor();
            let lap_after = (after / len).floor();
            if lap_after > lap_before && after > 0.0 && lap_after >= 1.0 {
                let best_done = racer.lap_times.len() as f32;
                // Count each lap once, the first time it is crossed.
                if lap_after > best_done {
                    let time = self.clock - racer.lap_started;
                    racer.lap_times.push(time);
                    racer.lap_started = self.clock;
                    let lap = racer.lap_times.len() as u32;
                    self.events.push(Event {
                        bike: i,
                        kind: EventKind::LapComplete { lap, time },
                    });
                    if after >= total {
                        racer.finish_time = Some(self.clock);
                        racer.progress = total;
                        let position = 1 + self
                            .racers
                            .iter()
                            .enumerate()
                            .filter(|(k, r)| *k != i && r.finish_time.is_some())
                            .count();
                        self.events.push(Event {
                            bike: i,
                            kind: EventKind::Finished { position },
                        });
                        if i == PLAYER {
                            self.phase = Phase::Finished { t: 0.0 };
                        }
                    }
                }
            }
        }
    }

    fn rank(&mut self) {
        let racers = &self.racers;
        self.order.sort_by(|&a, &b| {
            let (ra, rb) = (&racers[a], &racers[b]);
            match (ra.finish_time, rb.finish_time) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => rb.progress.total_cmp(&ra.progress),
            }
        });
        let place = self.place_of(PLAYER);
        if matches!(self.phase, Phase::Racing) && place != self.last_player_place {
            self.push(
                PLAYER,
                EventKind::Overtake {
                    gained: place < self.last_player_place,
                },
            );
        }
        self.last_player_place = place;
    }

    /// Final standings once the player is home: finished riders by time,
    /// the rest projected from their current pace.
    pub fn standings(&self) -> Vec<(usize, f32, bool)> {
        let total = self.race_distance();
        let mut rows: Vec<(usize, f32, bool)> = (0..self.bikes.len())
            .map(|i| {
                let r = &self.racers[i];
                match r.finish_time {
                    Some(t) => (i, t, true),
                    None => {
                        let pace = (r.progress.max(1.0) / self.clock.max(1.0)).max(5.0);
                        (i, self.clock + (total - r.progress).max(0.0) / pace, false)
                    }
                }
            })
            .collect();
        rows.sort_by(|a, b| a.1.total_cmp(&b.1));
        rows
    }

    pub fn is_counting_down(&self) -> bool {
        matches!(self.phase, Phase::Countdown { .. })
    }

    pub fn countdown_remaining(&self) -> f32 {
        match self.phase {
            Phase::Countdown { t } => (COUNTDOWN_SECS - t).max(0.0),
            _ => 0.0,
        }
    }

    /// Heading error of the player relative to the circuit, radians.
    pub fn player_heading_error(&self) -> f32 {
        let b = self.player();
        wrap_angle(b.heading - self.track.frame_at(b.track_pos.s).heading())
    }
}

fn split_two<T>(v: &mut [T], i: usize, j: usize) -> (&mut T, &mut T) {
    debug_assert!(i < j);
    let (lo, hi) = v.split_at_mut(j);
    (&mut lo[i], &mut hi[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::catalog;

    fn throttle() -> Controls {
        Controls {
            throttle: 1.0,
            ..Controls::default()
        }
    }

    #[test]
    fn countdown_holds_the_grid_then_goes() {
        let mut race = Race::new(catalog()[0], 1);
        let start = race.player().pos;
        for _ in 0..((COUNTDOWN_SECS / DT) as usize - 5) {
            race.tick(&throttle());
            assert_eq!(race.player().pos, start);
        }
        let mut went = false;
        for _ in 0..20 {
            race.tick(&throttle());
            went |= race.events.iter().any(|e| e.kind == EventKind::Go);
        }
        assert!(went);
        assert!(matches!(race.phase, Phase::Racing));
    }

    /// The AI field gets round every circuit cleanly and at a sensible pace.
    #[test]
    fn rivals_complete_laps_without_chaos() {
        for def in catalog() {
            let mut race = Race::new(*def, 7);
            let mut crashes = 0;
            let ticks = (200.0 / DT) as usize;
            for _ in 0..ticks {
                // Player idles on the grid out of the way.
                race.tick(&Controls::default());
                crashes += race
                    .events
                    .iter()
                    .filter(|e| e.kind == EventKind::Crash && e.bike != PLAYER)
                    .count();
            }
            let leader = race.order.iter().find(|&&b| b != PLAYER).copied().unwrap();
            let r = &race.racers[leader];
            assert!(
                r.lap_times.len() >= 2,
                "{}: leader only did {} laps",
                def.name,
                r.lap_times.len()
            );
            let lap = r.lap_times[1];
            let avg_kmh = race.track.length / lap * 3.6;
            assert!(
                (110.0..220.0).contains(&avg_kmh),
                "{}: lap {lap:.1}s = {avg_kmh:.0} km/h",
                def.name
            );
            assert!(crashes <= 6, "{}: {crashes} rival crashes", def.name);
            for (i, b) in race.bikes.iter().enumerate().skip(1) {
                assert!(
                    b.track_pos.lateral.abs() < race.track.half_width + 6.0,
                    "{} rider {i} is off in the weeds",
                    def.name
                );
            }
        }
    }

    #[test]
    fn punch_lands_on_a_bike_alongside() {
        let mut race = Race::new(catalog()[0], 3);
        race.phase = Phase::Racing;
        let s = 40.0;
        let track = race.track.clone();
        race.bikes[PLAYER] = Bike::new(&track, s, 0.0, Tuning::default());
        race.bikes[1] = Bike::new(&track, s, 1.4, Tuning::default());
        for k in 2..race.bikes.len() {
            race.bikes[k] = Bike::new(&track, s + 200.0 + k as f32 * 10.0, 0.0, Tuning::default());
        }
        let before = race.racers[1].balance;
        let punch = Controls {
            attack: true,
            ..Controls::default()
        };
        race.tick(&punch);
        for _ in 0..40 {
            race.tick(&Controls::default());
        }
        assert!(race.racers[1].balance < before, "rival took the hit");
        assert_eq!(race.racers[PLAYER].hits_landed, 1);
    }
}
