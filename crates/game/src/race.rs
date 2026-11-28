//! The race: grid, countdown, laps, positions, bike contact, combat, results.

use glam::Vec2;
use sim::rng::XorShift64;

use crate::ai::{Rival, GRUDGE_SECS};
use crate::bike::{wrap_angle, Bike, Controls, Tuning};
use crate::difficulty::Difficulty;
use crate::track::{Track, TrackDef};
use crate::traffic::{circle_vs_car, Traffic, STOP_SECS};

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
/// Kick: a longer wind-up you can see coming, more reach, more damage.
pub const KICK_SECS: f32 = 0.62;
const KICK_HIT_AT: f32 = 0.3;
const KICK_COOLDOWN: f32 = 1.1;
const KICK_DAMAGE: f32 = 52.0;
const BALANCE_REGEN: f32 = 9.0;
/// Guard: how fast it comes up, how much of a hit it soaks, and how soon
/// after raising it a hit is parried instead.
const GUARD_RISE: f32 = 9.0;
const GUARD_SOAK: f32 = 0.25;
pub const PARRY_WINDOW: f32 = 0.22;
const PARRY_PUNISH: f32 = 34.0;
const PARRY_STUN: f32 = 1.0;
/// While the guard is up the rider sits up: less drive, no boost.
const GUARD_THROTTLE: f32 = 0.82;
/// Throttle on the line that gives a perfect start (revs in the green).
pub const LAUNCH_SWEET: (f32, f32) = (0.55, 0.82);
/// Above this the rear tyre lights up at the green.
pub const LAUNCH_SPIN: f32 = 0.92;
/// Boost for gaining a place.
const OVERTAKE_BOOST: f32 = 0.05;
/// Running into a car faster than this puts the rider down.
const TRAFFIC_CRASH_SPEED: f32 = 9.0;
/// A rider shoved by a strike goes down against a car at this.
const SHOVED_CRASH_SPEED: f32 = 3.0;
/// Near miss: passing a car this close (edge to edge) at this relative speed.
const NEAR_MISS_GAP: f32 = 1.5;
const NEAR_MISS_SPEED: f32 = 16.0;
const NEAR_MISS_BOOST: f32 = 0.08;
/// Strikes a weapon survives.
pub const WEAPON_USES: u8 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponKind {
    /// Short and heavy.
    Club,
    /// Long reach, a little lighter.
    Chain,
}

impl WeaponKind {
    /// Lateral reach of a swing, metres from the bike's centreline.
    fn reach(self) -> std::ops::Range<f32> {
        match self {
            WeaponKind::Club => 0.3..3.3,
            WeaponKind::Chain => 0.3..3.9,
        }
    }

    fn damage(self) -> f32 {
        match self {
            WeaponKind::Club => 56.0,
            WeaponKind::Chain => 46.0,
        }
    }

    fn shove(self) -> f32 {
        match self {
            WeaponKind::Club => 4.4,
            WeaponKind::Chain => 3.6,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            WeaponKind::Club => "Club",
            WeaponKind::Chain => "Chain",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weapon {
    pub kind: WeaponKind,
    /// Strikes left before it breaks.
    pub uses: u8,
}

impl Weapon {
    pub fn new(kind: WeaponKind) -> Self {
        Self {
            kind,
            uses: WEAPON_USES,
        }
    }
}

/// What the player brings to a race from the garage.
#[derive(Debug, Clone, Copy)]
pub struct Loadout {
    pub tuning: Tuning,
    /// Scales balance damage the player takes (1 = stock leathers).
    pub armour: f32,
}

/// Highest level of each garage upgrade.
pub const MAX_UPGRADE: u32 = 3;

impl Loadout {
    /// Garage upgrades, each 0 (stock) to [`MAX_UPGRADE`]: engine adds pull
    /// and top speed, handling adds grip, armour soaks hits.
    pub fn from_levels(engine: u32, handling: u32, armour: u32) -> Self {
        let e = engine.min(MAX_UPGRADE) as f32;
        let h = handling.min(MAX_UPGRADE) as f32;
        let a = armour.min(MAX_UPGRADE) as f32;
        Self {
            tuning: Tuning {
                power: 1.0 + 0.045 * e,
                grip: 1.0 + 0.04 * h,
                top_speed: 1.0 + 0.025 * e,
            },
            armour: 1.0 - 0.13 * a,
        }
    }
}

impl Default for Loadout {
    fn default() -> Self {
        Self {
            tuning: Tuning::default(),
            armour: 1.0,
        }
    }
}

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
    WallHit {
        speed: f32,
    },
    BikeContact {
        speed: f32,
    },
    /// A strike landed clean on this bike.
    Hit {
        attacker: usize,
        kick: bool,
    },
    /// This bike's guard soaked a strike.
    Blocked {
        attacker: usize,
    },
    /// This bike's guard went up just in time and turned the strike back.
    Parried {
        attacker: usize,
    },
    /// This bike's rider put `victim` on the ground.
    KnockOff {
        victim: usize,
    },
    /// This bike ran into a car and went down.
    TrafficCrash {
        speed: f32,
    },
    /// This bike scraped a car and stayed up.
    CarScrape {
        speed: f32,
    },
    /// This bike threaded past a car close and fast.
    NearMiss {
        oncoming: bool,
    },
    /// This bike's rider picked up a weapon, taking it off `from`.
    Armed {
        kind: WeaponKind,
        from: usize,
    },
    /// This bike's weapon broke on its last strike.
    WeaponBroke,
    /// This bike left the line: perfectly, or in a cloud of wheelspin.
    Launch {
        perfect: bool,
    },
    Crash,
    Landed {
        speed: f32,
    },
    LapComplete {
        lap: u32,
        time: f32,
    },
    Finished {
        position: usize,
    },
    Overtake {
        gained: bool,
    },
    CountdownBeep {
        n: u32,
    },
    Go,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    pub bike: usize,
    pub kind: EventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strike {
    Punch,
    Kick,
}

impl Strike {
    pub fn secs(self) -> f32 {
        match self {
            Strike::Punch => SWING_SECS,
            Strike::Kick => KICK_SECS,
        }
    }

    fn hit_at(self) -> f32 {
        match self {
            Strike::Punch => SWING_HIT_AT,
            Strike::Kick => KICK_HIT_AT,
        }
    }
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
    pub swing_kind: Strike,
    pub attack_cooldown: f32,
    /// 0..1 guard; `guard_held` counts seconds since it went up.
    pub guard: f32,
    pub guard_held: f32,
    /// Rival guard: seconds until it goes up, then seconds to hold it.
    pub ai_guard_in: f32,
    pub ai_guard_hold: f32,
    pub wrong_way: f32,
    pub hits_landed: u32,
    pub knockdowns: u32,
    pub knockoffs: u32,
    pub parries: u32,
    pub near_misses: u32,
    pub weapon: Option<Weapon>,
    /// Who struck this rider last, and is still to blame for a fall.
    pub last_hit_by: Option<usize>,
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
            swing_kind: Strike::Punch,
            attack_cooldown: 0.0,
            guard: 0.0,
            guard_held: 0.0,
            ai_guard_in: -1.0,
            ai_guard_hold: 0.0,
            wrong_way: 0.0,
            hits_landed: 0,
            knockdowns: 0,
            knockoffs: 0,
            parries: 0,
            near_misses: 0,
            weapon: None,
            last_hit_by: None,
        }
    }

    /// Lap the racer is on (1-based), clamped to the race length.
    pub fn lap(&self, track_len: f32, laps: u32) -> u32 {
        ((self.progress / track_len).floor() as i64 + 1).clamp(1, laps as i64) as u32
    }

    pub fn swinging(&self) -> bool {
        self.swing > 0.0
    }

    /// 0..1 through the current strike, if any.
    pub fn swing_phase(&self) -> Option<f32> {
        (self.swing > 0.0).then(|| 1.0 - self.swing / self.swing_kind.secs())
    }

    pub fn guarding(&self) -> bool {
        self.guard > 0.5
    }

    /// Reach, damage and shove of the next swing.
    fn strike_stats(&self, kind: Strike) -> (std::ops::Range<f32>, f32, f32) {
        match (kind, self.weapon) {
            (Strike::Kick, _) => (0.3..3.1, KICK_DAMAGE, 5.0),
            (Strike::Punch, Some(w)) => (w.kind.reach(), w.kind.damage(), w.kind.shove()),
            (Strike::Punch, None) => (0.3..2.7, HIT_DAMAGE, 3.2),
        }
    }
}

pub struct Race {
    pub track: Track,
    pub bikes: Vec<Bike>,
    pub racers: Vec<Racer>,
    pub rivals: Vec<Option<Rival>>,
    pub traffic: Traffic,
    pub phase: Phase,
    /// Race clock, seconds since the green light.
    pub clock: f32,
    pub laps: u32,
    /// Bike indices by race position (leader first).
    pub order: Vec<usize>,
    pub events: Vec<Event>,
    pub difficulty: Difficulty,
    /// Chance a rival raises its guard against a strike it sees coming.
    pub guard_chance: f32,
    /// Scales balance damage the player takes, from the garage.
    pub player_armour: f32,
    /// Engine power each bike started with, before catch-up.
    base_power: Vec<f32>,
    rng: XorShift64,
    last_player_place: usize,
    beeps: u32,
}

impl Race {
    pub fn new(def: TrackDef, seed: u64) -> Self {
        Self::with_difficulty(def, seed, Difficulty::Normal)
    }

    pub fn with_difficulty(def: TrackDef, seed: u64, difficulty: Difficulty) -> Self {
        Self::with_track_and_difficulty(Track::build(def), seed, difficulty)
    }

    pub fn with_track(track: Track, seed: u64) -> Self {
        Self::with_track_and_difficulty(track, seed, Difficulty::Normal)
    }

    pub fn with_track_and_difficulty(track: Track, seed: u64, difficulty: Difficulty) -> Self {
        Self::with_loadout(track, seed, difficulty, Loadout::default())
    }

    pub fn with_loadout(track: Track, seed: u64, difficulty: Difficulty, loadout: Loadout) -> Self {
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
                (loadout.tuning, None)
            } else {
                // Front of the grid is quicker; every rival differs a little.
                let quality = 1.0 - row / n as f32;
                let pace = difficulty.rival_pace();
                let tuning = Tuning {
                    power: (0.93 + 0.07 * quality + rng.next_f32_in(-0.02, 0.02)) * pace,
                    grip: 0.95 + 0.05 * quality,
                    top_speed: (0.95 + 0.05 * quality + rng.next_f32_in(-0.01, 0.01)) * pace,
                };
                let lane = rng.next_f32_in(-0.9, 0.9);
                let commitment = (0.80
                    + 0.14 * quality
                    + rng.next_f32_in(-0.02, 0.02)
                    + difficulty.rival_nerve())
                .clamp(0.7, 0.97);
                let aggression =
                    (rng.next_f32_in(0.25, 0.95) * difficulty.rival_aggression()).min(1.0);
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
        // The angriest riders come looking for you; some bring weapons.
        let mut by_anger: Vec<usize> = (1..n).collect();
        by_anger.sort_by(|&a, &b| {
            let ag = |i: usize| rivals[i].as_ref().map_or(0.0, |r: &Rival| r.aggression);
            ag(b).total_cmp(&ag(a))
        });
        for &i in by_anger.iter().take(difficulty.brawlers()) {
            if let Some(r) = rivals[i].as_mut() {
                r.brawler = true;
            }
        }
        for (k, &i) in by_anger.iter().take(difficulty.armed_rivals()).enumerate() {
            let kind = if k % 2 == 0 {
                WeaponKind::Club
            } else {
                WeaponKind::Chain
            };
            racers[i].weapon = Some(Weapon::new(kind));
        }
        let traffic = Traffic::spawn(&track, difficulty.traffic_density(), &mut rng);
        let laps = track.def.laps;
        let base_power = bikes.iter().map(|b| b.tuning.power).collect();
        let mut race = Self {
            track,
            bikes,
            racers,
            rivals,
            traffic,
            phase: Phase::Countdown { t: 0.0 },
            clock: 0.0,
            laps,
            order: (0..n).collect(),
            events: Vec::with_capacity(32),
            difficulty,
            guard_chance: difficulty.rival_guard(),
            player_armour: loadout.armour,
            base_power,
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
                    self.launch(player_input.throttle);
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
        self.traffic.step(&self.track, DT, &mut self.rng);
        self.step_bikes(player_input);
        self.contacts();
        self.traffic_contacts();
        self.near_misses();
        self.combat();
        self.progress();
        self.rank();
    }

    /// The green light: a throttle held in the sweet spot fires the bike off
    /// the line; a pinned throttle spins the rear tyre. Rivals launch well
    /// more often on harder levels.
    fn launch(&mut self, player_throttle: f32) {
        let good_odds = match self.difficulty {
            Difficulty::Easy => 0.2,
            Difficulty::Normal => 0.35,
            Difficulty::Hard => 0.55,
        };
        for i in 0..self.bikes.len() {
            let perfect = if i == PLAYER {
                let t = player_throttle;
                if t > LAUNCH_SPIN {
                    Some(false)
                } else if (LAUNCH_SWEET.0..=LAUNCH_SWEET.1).contains(&t) {
                    Some(true)
                } else {
                    None
                }
            } else {
                let roll = self.rng.next_f32();
                if roll < good_odds {
                    Some(true)
                } else if roll > 0.93 {
                    Some(false)
                } else {
                    None
                }
            };
            let b = &mut self.bikes[i];
            match perfect {
                Some(true) => {
                    b.vel = b.fwd() * 7.0;
                    b.boost = (b.boost + 0.2).min(1.0);
                }
                Some(false) => b.wheelspin = 0.9,
                None => {}
            }
            if let (true, Some(p)) = (i == PLAYER, perfect) {
                self.push(PLAYER, EventKind::Launch { perfect: p });
            }
        }
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
                let prey = (i != PLAYER && !player_finished).then(|| &self.bikes[PLAYER]);
                let rival = self.rivals[i].get_or_insert_with(|| Rival::new(0.0, 0.88, 0.0));
                let mut c = rival.drive(
                    &self.bikes[i],
                    &others,
                    &self.traffic.cars,
                    prey,
                    &self.track,
                    DT,
                    roll,
                );
                // Catch-up, set by the difficulty: never decisive.
                let gap = self.racers[PLAYER].progress - self.racers[i].progress;
                if i != PLAYER {
                    self.bikes[i].tuning.power = self.base_power[i] * self.difficulty.catch_up(gap);
                }
                // A rival's guard goes up when it saw a strike coming.
                let r = &mut self.racers[i];
                if r.ai_guard_in >= 0.0 {
                    r.ai_guard_in -= DT;
                    if r.ai_guard_in < 0.0 {
                        r.ai_guard_hold = 0.6;
                    }
                }
                r.ai_guard_hold = (r.ai_guard_hold - DT).max(0.0);
                c.block = r.ai_guard_hold > 0.0;
                if self.racers[i].finish_time.is_some() {
                    c.throttle *= 0.55;
                    c.attack = false;
                    c.kick = false;
                    c.boost = false;
                }
                c
            };
            // Guard: comes up fast, and while it's up the rider sits up.
            let mut controls = controls;
            let r = &mut self.racers[i];
            let want_guard = controls.block && self.bikes[i].is_riding();
            if want_guard {
                if r.guard <= 0.0 {
                    r.guard_held = 0.0;
                }
                r.guard = (r.guard + GUARD_RISE * DT).min(1.0);
                r.guard_held += DT;
            } else {
                r.guard = (r.guard - GUARD_RISE * DT).max(0.0);
            }
            if r.guarding() {
                controls.throttle *= GUARD_THROTTLE;
                controls.boost = false;
                controls.attack = false;
                controls.kick = false;
            }
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
            if controls.kick && bike.is_riding() {
                self.start_swing(i, Strike::Kick);
            } else if controls.attack && bike.is_riding() {
                self.start_swing(i, Strike::Punch);
            }
            let bike = &mut self.bikes[i];
            let fx = bike.step(&self.track, &controls, DT);
            if fx.wall_hit > 2.0 {
                self.push(i, EventKind::WallHit { speed: fx.wall_hit });
            }
            if fx.crashed {
                self.racers[i].knockdowns += 1;
                self.racers[i].weapon = None;
                self.push(i, EventKind::Crash);
            }
            if fx.landed > 0.0 {
                self.push(i, EventKind::Landed { speed: fx.landed });
            }
        }
    }

    fn start_swing(&mut self, i: usize, kind: Strike) {
        let r = &mut self.racers[i];
        if r.swing > 0.0 || r.attack_cooldown > 0.0 {
            return;
        }
        let mut target = None;
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
                target = Some(j);
                side = if rel.dot(me.right()) >= 0.0 {
                    1.0
                } else {
                    -1.0
                };
            }
        }
        let r = &mut self.racers[i];
        r.swing = kind.secs();
        r.swing_kind = kind;
        r.swing_side = side;
        r.swing_landed = false;
        r.attack_cooldown = kind.secs()
            + match kind {
                Strike::Punch => ATTACK_COOLDOWN,
                Strike::Kick => KICK_COOLDOWN,
            };
        // A rival that sees it coming may get its guard up, early enough
        // to parry or a beat too late.
        if let Some(j) = target {
            let roll = self.rng.next_f32();
            let t = &mut self.racers[j];
            if j != PLAYER
                && roll < self.guard_chance
                && t.ai_guard_hold <= 0.0
                && t.ai_guard_in < 0.0
            {
                t.ai_guard_in = self.rng.next_f32_in(0.0, kind.hit_at() + 0.05);
            }
        }
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
            let kind = r.swing_kind;
            let before = r.swing;
            r.swing = (r.swing - DT).max(0.0);
            let total = kind.secs();
            let crossed = total - before < kind.hit_at() && total - r.swing >= kind.hit_at();
            if !crossed || r.swing_landed || !self.bikes[i].is_riding() {
                continue;
            }
            let side = r.swing_side;
            let reach = r.strike_stats(kind).0;
            let me = self.bikes[i].clone();
            for j in 0..n {
                if j == i || !self.bikes[j].is_solid() {
                    continue;
                }
                let rel = self.bikes[j].pos2() - me.pos2();
                let along = rel.dot(me.fwd());
                let across = rel.dot(me.right()) * side;
                if along.abs() < 1.9 && reach.contains(&across) {
                    self.strike(i, j, kind, side);
                    break;
                }
            }
        }
    }

    /// Strike `i` reaches bike `j`: parried, blocked, or clean.
    fn strike(&mut self, i: usize, j: usize, kind: Strike, side: f32) {
        self.racers[i].swing_landed = true;
        let (guard, held) = (self.racers[j].guarding(), self.racers[j].guard_held);
        if guard && held <= PARRY_WINDOW {
            // Parry: the strike bounces back on the attacker.
            self.racers[j].parries += 1;
            self.bikes[j].boost = (self.bikes[j].boost + 0.1).min(1.0);
            let a = &mut self.racers[i];
            a.attack_cooldown = a.attack_cooldown.max(PARRY_STUN);
            a.swing = 0.0;
            self.push(j, EventKind::Parried { attacker: i });
            let back = -self.bikes[i].right() * side * 1.6;
            self.bikes[i].vel += back;
            // Turning a weapon back takes it off them.
            if kind == Strike::Punch && self.racers[j].weapon.is_none() {
                if let Some(w) = self.racers[i].weapon.take() {
                    self.racers[j].weapon = Some(w);
                    self.push(
                        j,
                        EventKind::Armed {
                            kind: w.kind,
                            from: i,
                        },
                    );
                }
            }
            self.damage(i, PARRY_PUNISH, j);
            return;
        }
        let (_, damage, shove) = self.racers[i].strike_stats(kind);
        // Each clean or blocked weapon blow wears it down.
        if kind == Strike::Punch {
            if let Some(w) = self.racers[i].weapon.as_mut() {
                w.uses = w.uses.saturating_sub(1);
                if w.uses == 0 {
                    self.racers[i].weapon = None;
                    self.push(i, EventKind::WeaponBroke);
                }
            }
        }
        let soak = if guard { GUARD_SOAK } else { 1.0 };
        let right = self.bikes[i].right();
        let target = &mut self.bikes[j];
        target.vel += right * side * shove * if guard { 0.4 } else { 1.0 };
        target.yaw_rate += side * 0.6 * soak;
        if guard {
            self.push(j, EventKind::Blocked { attacker: i });
        } else {
            self.racers[i].hits_landed += 1;
            // Hit a rival and it remembers.
            if i == PLAYER {
                if let Some(r) = self.rivals[j].as_mut() {
                    r.grudge = GRUDGE_SECS;
                }
            }
            self.bikes[i].boost = (self.bikes[i].boost + 0.12).min(1.0);
            self.push(
                j,
                EventKind::Hit {
                    attacker: i,
                    kick: kind == Strike::Kick,
                },
            );
        }
        self.damage(j, damage * soak, i);
    }

    /// Takes balance off `j`; at zero the rider goes down and `by` gets the
    /// knock-off: a full boost meter.
    fn damage(&mut self, j: usize, amount: f32, by: usize) {
        let scale = if j == PLAYER {
            self.difficulty.damage_to_player() * self.player_armour
        } else {
            1.0
        };
        let tr = &mut self.racers[j];
        tr.balance -= amount * scale;
        tr.since_hit = 0.0;
        tr.last_hit_by = Some(by);
        if tr.balance <= 0.0 {
            tr.balance = 100.0;
            tr.guard = 0.0;
            self.bikes[j].crash();
            self.knocked_off(j, by);
        }
    }

    /// `j` is on the ground and `by` put it there: the knock-off scores,
    /// fills `by`'s boost, and a weapon changes hands.
    fn knocked_off(&mut self, j: usize, by: usize) {
        self.racers[j].knockdowns += 1;
        self.push(j, EventKind::Crash);
        self.racers[by].knockoffs += 1;
        self.bikes[by].boost = 1.0;
        self.push(by, EventKind::KnockOff { victim: j });
        if let Some(w) = self.racers[j].weapon.take() {
            if self.racers[by].weapon.is_none() {
                self.racers[by].weapon = Some(Weapon::new(w.kind));
                self.push(
                    by,
                    EventKind::Armed {
                        kind: w.kind,
                        from: j,
                    },
                );
            }
        }
    }

    /// Bikes against road traffic: a scrape bounces off, a real hit puts the
    /// rider over the bonnet and stops the car. A rider shoved into a car by
    /// a strike goes down far more easily, and the striker gets the credit.
    fn traffic_contacts(&mut self) {
        for i in 0..self.bikes.len() {
            if !self.bikes[i].is_solid() {
                continue;
            }
            for ci in 0..self.traffic.cars.len() {
                let car = &self.traffic.cars[ci];
                let b = &self.bikes[i];
                if b.pos2().distance_squared(car.pos2()) > 36.0 || (b.pos.y - car.pos.y).abs() > 2.0
                {
                    continue;
                }
                let mut deepest: Option<(Vec2, f32)> = None;
                for sa in [-1.0, 1.0] {
                    let p = b.pos2() + b.fwd() * CONTACT_OFFSET * sa;
                    if let Some((nrm, pen)) = circle_vs_car(car, p, CONTACT_RADIUS) {
                        if deepest.is_none_or(|(_, d)| pen > d) {
                            deepest = Some((nrm, pen));
                        }
                    }
                }
                let Some((nrm, pen)) = deepest else { continue };
                let car_vel = car.vel();
                let closing = -(b.vel - car_vel).dot(nrm);
                let r = &self.racers[i];
                let shoved_by = r.last_hit_by.filter(|_| r.since_hit < 0.8);
                let b = &mut self.bikes[i];
                b.pos.x += nrm.x * pen;
                b.pos.z += nrm.y * pen;
                let goes_down = closing > TRAFFIC_CRASH_SPEED
                    || (shoved_by.is_some() && closing > SHOVED_CRASH_SPEED);
                if goes_down {
                    let v = b.vel;
                    b.crash();
                    // Over the bonnet: the rider keeps going, the bike stops.
                    let up = 4.0 + closing * 0.09;
                    b.rider_vel = glam::Vec3::new(v.x * 0.7, up, v.y * 0.7);
                    b.vel = car_vel * 0.6 + nrm * 2.0;
                    let car = &mut self.traffic.cars[ci];
                    car.stopped = STOP_SECS;
                    car.speed *= 0.3;
                    self.racers[i].weapon = None;
                    self.push(i, EventKind::TrafficCrash { speed: closing });
                    match shoved_by {
                        Some(by) if by != i => self.knocked_off(i, by),
                        _ => {
                            self.racers[i].knockdowns += 1;
                            self.push(i, EventKind::Crash);
                        }
                    }
                    break;
                } else if closing > 0.0 {
                    b.vel += nrm * closing * 1.3;
                    b.yaw_rate += nrm.dot(b.right()) * -0.4;
                    if closing > 1.5 {
                        self.push(i, EventKind::CarScrape { speed: closing });
                    }
                }
            }
        }
    }

    /// The player threading past a car close and fast tops up the boost.
    fn near_misses(&mut self) {
        let p = &self.bikes[PLAYER];
        let riding = p.is_riding();
        let (ps, plat, pvel) = (p.track_pos.s, p.track_pos.lateral, p.vel);
        let mut scored = Vec::new();
        for (ci, car) in self.traffic.cars.iter_mut().enumerate() {
            let d = self.track.delta_s(car.s, ps);
            if d.abs() > 30.0 {
                car.player_side = 0.0;
                continue;
            }
            let side = if d >= 0.0 { 1.0 } else { -1.0 };
            let passed = car.player_side != 0.0 && side != car.player_side;
            car.player_side = side;
            if !passed || !riding || car.near_miss_cd > 0.0 {
                continue;
            }
            let gap = (plat - car.lateral).abs() - car.half_extents().x;
            let rel = (pvel - car.vel()).length();
            if (0.0..NEAR_MISS_GAP).contains(&gap) && rel > NEAR_MISS_SPEED {
                car.near_miss_cd = 3.0;
                scored.push((ci, car.dir < 0.0));
            }
        }
        for (_, oncoming) in scored {
            self.racers[PLAYER].near_misses += 1;
            let b = &mut self.bikes[PLAYER];
            b.boost = (b.boost + NEAR_MISS_BOOST).min(1.0);
            self.push(PLAYER, EventKind::NearMiss { oncoming });
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
            let gained = place < self.last_player_place;
            if gained {
                let b = &mut self.bikes[PLAYER];
                b.boost = (b.boost + OVERTAKE_BOOST).min(1.0);
            }
            self.push(PLAYER, EventKind::Overtake { gained });
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
    fn garage_upgrades_make_the_bike_quicker_and_tougher() {
        let stock = Loadout::from_levels(0, 0, 0);
        let maxed = Loadout::from_levels(MAX_UPGRADE, MAX_UPGRADE, 9);
        assert_eq!(stock.tuning.power, 1.0);
        assert_eq!(stock.armour, 1.0);
        assert!(maxed.tuning.power > 1.1 && maxed.tuning.top_speed > 1.05);
        assert!(maxed.tuning.grip > 1.1);
        assert!(maxed.armour > 0.5 && maxed.armour < 0.7, "{}", maxed.armour);
        let race = Race::with_loadout(Track::build(catalog()[0]), 1, Difficulty::Normal, maxed);
        assert_eq!(race.player_armour, maxed.armour);
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
            // Traffic and rival-on-rival fights put a few down; not a pile-up.
            assert!(crashes <= 9, "{}: {crashes} rival crashes", def.name);
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

    /// Player at `s` on the centre line, rival 1 alongside on the right, the
    /// rest of the field far up the road; racing, no rival guards.
    fn duel() -> Race {
        let mut race = Race::new(catalog()[0], 3);
        race.phase = Phase::Racing;
        race.guard_chance = 0.0;
        let s = 40.0;
        let track = race.track.clone();
        race.bikes[PLAYER] = Bike::new(&track, s, 0.0, Tuning::default());
        race.bikes[1] = Bike::new(&track, s, 1.4, Tuning::default());
        for k in 2..race.bikes.len() {
            race.bikes[k] = Bike::new(&track, s + 200.0 + k as f32 * 10.0, 0.0, Tuning::default());
        }
        race
    }

    fn run(race: &mut Race, first: Controls, ticks: usize) -> Vec<Event> {
        let mut seen = Vec::new();
        race.tick(&first);
        seen.extend(race.events.iter().copied());
        for _ in 0..ticks {
            race.tick(&Controls::default());
            seen.extend(race.events.iter().copied());
        }
        seen
    }

    #[test]
    fn kick_hits_harder_than_a_punch() {
        let mut a = duel();
        let mut b = duel();
        let punch = Controls {
            attack: true,
            ..Controls::default()
        };
        let kick = Controls {
            kick: true,
            ..Controls::default()
        };
        run(&mut a, punch, 90);
        let seen = run(&mut b, kick, 90);
        assert!(seen.iter().any(|e| e.kind
            == EventKind::Hit {
                attacker: PLAYER,
                kick: true
            }));
        assert!(b.racers[1].balance < a.racers[1].balance);
    }

    #[test]
    fn guard_soaks_a_strike_and_an_early_guard_parries() {
        // Rival 1 swings at the player.
        let mut race = duel();
        race.racers[PLAYER].guard = 1.0;
        race.racers[PLAYER].guard_held = 1.0; // up for a while: block, not parry
        race.start_swing(1, Strike::Punch);
        let block = Controls {
            block: true,
            ..Controls::default()
        };
        let mut seen = Vec::new();
        for _ in 0..80 {
            race.tick(&block);
            seen.extend(race.events.iter().copied());
        }
        assert!(seen
            .iter()
            .any(|e| e.kind == EventKind::Blocked { attacker: 1 }));
        let soaked = 100.0 - race.racers[PLAYER].balance;
        assert!(soaked > 0.0 && soaked < HIT_DAMAGE * 0.5, "{soaked}");

        // Guard raised just before the blow lands: parried, attacker hurt.
        let mut race = duel();
        race.start_swing(1, Strike::Punch);
        let mut seen = Vec::new();
        for k in 0..80 {
            let c = if k >= 8 { block } else { Controls::default() };
            race.tick(&c);
            seen.extend(race.events.iter().copied());
        }
        assert!(
            seen.iter()
                .any(|e| e.kind == EventKind::Parried { attacker: 1 }),
            "{seen:?}"
        );
        assert_eq!(race.racers[PLAYER].balance, 100.0);
        assert!(race.racers[1].balance < 100.0);
    }

    #[test]
    fn knocking_a_rider_off_fills_the_boost() {
        let mut race = duel();
        race.racers[1].balance = 10.0;
        race.bikes[PLAYER].boost = 0.1;
        let seen = run(
            &mut race,
            Controls {
                attack: true,
                ..Controls::default()
            },
            60,
        );
        assert!(seen
            .iter()
            .any(|e| e.kind == EventKind::KnockOff { victim: 1 }));
        assert!(!race.bikes[1].is_riding());
        assert!(race.bikes[PLAYER].boost > 0.95);
        assert_eq!(race.racers[PLAYER].knockoffs, 1);
    }

    fn launch_with(throttle: f32) -> (Race, Vec<Event>) {
        let mut race = Race::new(catalog()[0], 1);
        let c = Controls {
            throttle,
            ..Controls::default()
        };
        let mut seen = Vec::new();
        for _ in 0..((COUNTDOWN_SECS / DT) as usize + 3) {
            race.tick(&c);
            seen.extend(race.events.iter().copied());
        }
        (race, seen)
    }

    #[test]
    fn launch_rewards_revs_in_the_sweet_spot() {
        let (good, seen) = launch_with(0.7);
        assert!(seen
            .iter()
            .any(|e| e.kind == EventKind::Launch { perfect: true }));
        let (spun, seen) = launch_with(1.0);
        assert!(seen
            .iter()
            .any(|e| e.kind == EventKind::Launch { perfect: false }));
        assert!(spun.player().wheelspin > 0.0);
        assert!(good.player().speed() > spun.player().speed() + 3.0);
    }

    #[test]
    fn harder_levels_field_quicker_rivals() {
        let lap_pace = |d: Difficulty| {
            let race = Race::with_difficulty(catalog()[0], 7, d);
            let n = race.bikes.len() as f32 - 1.0;
            race.bikes
                .iter()
                .skip(1)
                .map(|b| b.tuning.power)
                .sum::<f32>()
                / n
        };
        let [e, n, h] = Difficulty::ALL.map(lap_pace);
        assert!(e < n && n < h, "{e} {n} {h}");
        assert_eq!(Race::new(catalog()[0], 7).difficulty, Difficulty::Normal);
    }
}
