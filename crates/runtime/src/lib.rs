//! Game session glue between the deterministic race and the renderer:
//! fixed-step simulation with render interpolation, the chase camera,
//! particle effects and the HUD snapshot. Platform-free; the web crate and
//! the native tools drive it.

use glam::{Mat4, Quat, Vec2, Vec3};

use game::ai::Rival;
use game::bike::{Bike, Controls, RiderState, Surface, BOOST_TOP_SPEED};
use game::race::{EventKind, Phase, Race, DT, PLAYER, RIVAL_NAMES, SWING_SECS};
use game::track::TrackDef;
use render::{BikeDraw, Camera, FrameInput, Particle, ParticleKind};
use sim::rng::XorShift64;

/// Raw player input for one frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    pub throttle: f32,
    pub brake: f32,
    /// -1..1; digital keys are eased here, analog sticks pass through.
    pub steer: f32,
    pub analog_steer: bool,
    pub boost: bool,
    pub attack: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CamMode {
    Chase,
    Far,
    Hood,
}

/// Sound and HUD cues raised since the last frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cue {
    Beep(u32),
    Go,
    WallHit(f32),
    Contact(f32),
    HitLanded,
    GotHit,
    Crash,
    Lap(u32, f32),
    Finished(usize),
    Overtake(bool),
}

#[derive(Debug, Clone, Copy)]
struct Snap {
    pos: Vec3,
    heading: f32,
    lean: f32,
    pitch: f32,
    wheel: f32,
    steer: f32,
    rider: Vec3,
    spin: f32,
}

impl Snap {
    fn of(b: &Bike) -> Self {
        Self {
            pos: b.pos,
            heading: b.heading,
            lean: b.lean,
            pitch: b.pitch,
            wheel: b.wheel_angle,
            steer: b.steer,
            rider: b.rider_offset,
            spin: b.rider_spin,
        }
    }

    fn lerp(a: &Snap, b: &Snap, t: f32) -> Snap {
        let ang = |x: f32, y: f32| x + game::bike::wrap_angle(y - x) * t;
        Snap {
            pos: a.pos.lerp(b.pos, t),
            heading: ang(a.heading, b.heading),
            lean: a.lean + (b.lean - a.lean) * t,
            pitch: a.pitch + (b.pitch - a.pitch) * t,
            wheel: ang(a.wheel, b.wheel),
            steer: a.steer + (b.steer - a.steer) * t,
            rider: a.rider.lerp(b.rider, t),
            spin: a.spin + (b.spin - a.spin) * t,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Live {
    p: Particle,
    life: f32,
    age: f32,
    grow: f32,
    gravity: f32,
    drag: f32,
    alpha0: f32,
}

pub const PAINTS: [([f32; 3], [f32; 3]); 8] = [
    ([0.80, 0.04, 0.03], [0.85, 0.85, 0.86]),
    ([0.03, 0.18, 0.75], [0.05, 0.05, 0.08]),
    ([0.95, 0.62, 0.02], [0.08, 0.08, 0.08]),
    ([0.06, 0.55, 0.18], [0.80, 0.80, 0.78]),
    ([0.92, 0.30, 0.02], [0.05, 0.05, 0.06]),
    ([0.42, 0.06, 0.62], [0.75, 0.70, 0.30]),
    ([0.02, 0.55, 0.60], [0.90, 0.90, 0.90]),
    ([0.85, 0.85, 0.88], [0.60, 0.04, 0.04]),
];

pub struct Session {
    pub race: Race,
    prev: Vec<Snap>,
    cur: Vec<Snap>,
    acc: f32,
    pub time: f32,
    steer: f32,
    pub cam_mode: CamMode,
    cam_pos: Vec3,
    cam_yaw: f32,
    cam_fov: f32,
    cam_roll: f32,
    trauma: f32,
    cam_ready: bool,
    particles: Vec<Live>,
    rng: XorShift64,
    pub cues: Vec<Cue>,
    /// Drives the player with the rival AI (attract mode, screenshots).
    pub autopilot: Option<Rival>,
    pub paused: bool,
    best_lap: Option<f32>,
}

impl Session {
    pub fn new(def: TrackDef, seed: u64) -> Self {
        let race = Race::new(def, seed);
        let snaps: Vec<Snap> = race.bikes.iter().map(Snap::of).collect();
        Self {
            race,
            prev: snaps.clone(),
            cur: snaps,
            acc: 0.0,
            time: 0.0,
            steer: 0.0,
            cam_mode: CamMode::Chase,
            cam_pos: Vec3::ZERO,
            cam_yaw: 0.0,
            cam_fov: 1.05,
            cam_roll: 0.0,
            trauma: 0.0,
            cam_ready: false,
            particles: Vec::with_capacity(2048),
            rng: XorShift64::new(seed ^ 0xfeed),
            cues: Vec::new(),
            autopilot: None,
            paused: false,
            best_lap: None,
        }
    }

    pub fn cycle_camera(&mut self) {
        self.cam_mode = match self.cam_mode {
            CamMode::Chase => CamMode::Far,
            CamMode::Far => CamMode::Hood,
            CamMode::Hood => CamMode::Chase,
        };
    }

    /// Advances real time `dt` seconds: fixed ticks, effects and camera.
    pub fn update(&mut self, dt: f32, input: &Input) {
        self.cues.clear();
        let dt = dt.clamp(0.0, 0.1);
        if self.paused {
            return;
        }
        self.time += dt;
        // Keyboard steering eases in and snaps back; sticks are direct.
        let target = input.steer.clamp(-1.0, 1.0);
        if input.analog_steer {
            self.steer = target;
        } else {
            let rate = if target.abs() < 0.01 || target.signum() != self.steer.signum() {
                9.0
            } else {
                5.5
            };
            let step = rate * dt;
            self.steer += (target - self.steer).clamp(-step, step);
        }
        let mut controls = Controls {
            throttle: input.throttle.clamp(0.0, 1.0),
            brake: input.brake.clamp(0.0, 1.0),
            steer: self.steer,
            boost: input.boost,
            attack: input.attack,
        };
        self.acc += dt;
        while self.acc >= DT {
            self.acc -= DT;
            if let Some(ai) = self.autopilot.as_mut() {
                let me = &self.race.bikes[PLAYER];
                let others: Vec<&Bike> = self.race.bikes.iter().skip(1).collect();
                let roll = self.rng.next_f32();
                controls = ai.drive(me, &others, &self.race.track, DT, roll);
                if self.race.is_counting_down() {
                    controls.throttle = 0.7;
                }
            }
            self.prev.clone_from(&self.cur);
            self.race.tick(&controls);
            for (s, b) in self.cur.iter_mut().zip(&self.race.bikes) {
                *s = Snap::of(b);
            }
            self.handle_events();
            self.tick_effects();
        }
        self.step_particles(dt);
        self.update_camera(dt);
    }

    fn handle_events(&mut self) {
        let events = std::mem::take(&mut self.race.events);
        for e in &events {
            let b = self.race.bikes[e.bike].clone();
            let b = &b;
            let mine = e.bike == PLAYER;
            match e.kind {
                EventKind::CountdownBeep { n } => self.cues.push(Cue::Beep(n)),
                EventKind::Go => self.cues.push(Cue::Go),
                EventKind::WallHit { speed } => {
                    let p = b.pos
                        + Vec3::new(b.right().x, 0.0, b.right().y)
                            * b.track_pos.lateral.signum()
                            * 0.6;
                    self.burst_sparks(p + Vec3::Y * 0.5, b, 18 + (speed * 2.0) as usize);
                    if mine {
                        self.trauma = (self.trauma + speed * 0.05).min(1.0);
                        self.cues.push(Cue::WallHit(speed));
                    }
                }
                EventKind::BikeContact { speed } => {
                    self.burst_sparks(b.pos + Vec3::Y * 0.6, b, 10);
                    if mine {
                        self.trauma = (self.trauma + 0.25).min(1.0);
                        self.cues.push(Cue::Contact(speed));
                    }
                }
                EventKind::Hit { attacker } => {
                    self.burst_dust(b.pos + Vec3::Y * 1.2, 6, [0.5, 0.48, 0.45]);
                    if mine {
                        self.trauma = (self.trauma + 0.45).min(1.0);
                        self.cues.push(Cue::GotHit);
                    } else if attacker == PLAYER {
                        self.cues.push(Cue::HitLanded);
                    }
                }
                EventKind::Crash => {
                    self.burst_sparks(b.pos + Vec3::Y * 0.4, b, 40);
                    self.burst_dust(b.pos + Vec3::Y * 0.4, 14, [0.42, 0.4, 0.38]);
                    if mine {
                        self.trauma = 1.0;
                        self.cues.push(Cue::Crash);
                    }
                }
                EventKind::Landed { speed } => {
                    if mine {
                        self.trauma = (self.trauma + speed * 0.04).min(0.6);
                    }
                }
                EventKind::LapComplete { lap, time } => {
                    if mine {
                        self.best_lap = Some(self.best_lap.map_or(time, |b| b.min(time)));
                        self.cues.push(Cue::Lap(lap, time));
                    }
                }
                EventKind::Finished { position } => {
                    if mine {
                        self.cues.push(Cue::Finished(position));
                    }
                }
                EventKind::Overtake { gained } => {
                    if mine {
                        self.cues.push(Cue::Overtake(gained));
                    }
                }
            }
        }
        self.race.events = events;
    }

    fn spawn(&mut self, p: Particle, life: f32, grow: f32, gravity: f32, drag: f32) {
        if self.particles.len() >= 2400 {
            self.particles.swap_remove(0);
        }
        let alpha0 = p.color[3];
        self.particles.push(Live {
            p,
            life,
            age: 0.0,
            grow,
            gravity,
            drag,
            alpha0,
        });
    }

    fn burst_sparks(&mut self, at: Vec3, b: &Bike, n: usize) {
        let v = Vec3::new(b.vel.x, 0.0, b.vel.y);
        for _ in 0..n {
            let r = Vec3::new(
                self.rng.next_f32_in(-1.0, 1.0),
                self.rng.next_f32_in(0.2, 1.4),
                self.rng.next_f32_in(-1.0, 1.0),
            );
            let heat = self.rng.next_f32_in(0.6, 1.0);
            let life = self.rng.next_f32_in(0.25, 0.7);
            self.spawn(
                Particle {
                    kind: ParticleKind::Glow,
                    pos: at,
                    size: 0.018,
                    color: [9.0 * heat, 3.2 * heat, 0.8 * heat, 1.0],
                    vel: v * 0.85 + r * 5.0,
                    stretch: 0.018,
                },
                life,
                0.0,
                9.0,
                1.2,
            );
        }
    }

    fn burst_dust(&mut self, at: Vec3, n: usize, color: [f32; 3]) {
        for _ in 0..n {
            let r = Vec3::new(
                self.rng.next_f32_in(-1.0, 1.0),
                self.rng.next_f32_in(0.0, 1.0),
                self.rng.next_f32_in(-1.0, 1.0),
            );
            let life = self.rng.next_f32_in(0.8, 1.6);
            self.spawn(
                Particle {
                    kind: ParticleKind::Smoke,
                    pos: at + r * 0.4,
                    size: 0.5,
                    color: [color[0], color[1], color[2], 0.5],
                    vel: r * 2.5,
                    stretch: 0.0,
                },
                life,
                1.6,
                -0.3,
                1.5,
            );
        }
    }

    /// Continuous emitters, once per fixed tick.
    fn tick_effects(&mut self) {
        for i in 0..self.race.bikes.len() {
            let b = self.race.bikes[i].clone();
            let v = b.speed();
            let fwd = Vec3::new(b.fwd().x, 0.0, b.fwd().y);
            let right = Vec3::new(b.right().x, 0.0, b.right().y);
            let rear = b.pos - fwd * 0.72;
            let near = b.pos.distance(self.cam_pos) < 260.0 || !self.cam_ready;
            if !near {
                continue;
            }
            let chance = |s: &mut Self, p: f32| s.rng.next_f32() < p;
            if b.is_riding() && b.on_ground {
                if b.surface == Surface::Road
                    && b.slip > 0.3
                    && v > 6.0
                    && chance(self, b.slip * 0.8)
                {
                    self.spawn(
                        Particle {
                            kind: ParticleKind::Smoke,
                            pos: rear + Vec3::Y * 0.2,
                            size: 0.35,
                            color: [0.75, 0.75, 0.76, 0.42 * b.slip.min(1.0)],
                            vel: fwd * v * 0.15 + Vec3::Y * 0.8,
                            stretch: 0.0,
                        },
                        1.6,
                        2.2,
                        -0.4,
                        1.8,
                    );
                }
                if b.surface == Surface::Grass && v > 4.0 && chance(self, (v / 30.0).min(0.9)) {
                    let side = self.rng.next_f32_in(-0.4, 0.4);
                    self.spawn(
                        Particle {
                            kind: ParticleKind::Smoke,
                            pos: rear + right * side + Vec3::Y * 0.15,
                            size: 0.4,
                            color: [0.46, 0.38, 0.26, 0.45],
                            vel: -fwd * v * 0.12 + Vec3::Y * 1.4 + right * side * 3.0,
                            stretch: 0.0,
                        },
                        1.2,
                        2.0,
                        0.6,
                        1.6,
                    );
                }
            }
            if b.boosting {
                let exhaust = b.pos - fwd * 0.75 + right * 0.2 + Vec3::Y * 0.62;
                for _ in 0..2 {
                    let j = Vec3::new(
                        self.rng.next_f32_in(-0.3, 0.3),
                        self.rng.next_f32_in(-0.3, 0.3),
                        self.rng.next_f32_in(-0.3, 0.3),
                    );
                    self.spawn(
                        Particle {
                            kind: ParticleKind::Glow,
                            pos: exhaust,
                            size: 0.09,
                            color: [1.5, 3.0, 9.0, 1.0],
                            vel: Vec3::new(b.vel.x, 0.0, b.vel.y) - fwd * 9.0 + j,
                            stretch: 0.02,
                        },
                        0.12,
                        0.0,
                        0.0,
                        0.0,
                    );
                }
            }
            if matches!(b.state, RiderState::Crashed { .. }) && v > 4.0 && chance(self, 0.6) {
                let j = Vec3::new(
                    self.rng.next_f32_in(-2.0, 2.0),
                    self.rng.next_f32_in(0.5, 3.0),
                    self.rng.next_f32_in(-2.0, 2.0),
                );
                self.spawn(
                    Particle {
                        kind: ParticleKind::Glow,
                        pos: b.pos + Vec3::Y * 0.15,
                        size: 0.016,
                        color: [8.0, 3.0, 0.7, 1.0],
                        vel: Vec3::new(b.vel.x, 0.0, b.vel.y) * 0.8 + j,
                        stretch: 0.02,
                    },
                    0.4,
                    0.0,
                    9.0,
                    1.0,
                );
            }
        }
    }

    fn step_particles(&mut self, dt: f32) {
        self.particles.retain_mut(|l| {
            l.age += dt;
            if l.age >= l.life {
                return false;
            }
            let t = l.age / l.life;
            l.p.vel.y -= l.gravity * dt;
            l.p.vel *= (1.0 - l.drag * dt).max(0.0);
            l.p.pos += l.p.vel * dt;
            l.p.size += l.grow * dt;
            l.p.color[3] = l.alpha0 * (1.0 - t) * (1.0 - t).sqrt();
            true
        });
    }

    fn alpha(&self) -> f32 {
        (self.acc / DT).clamp(0.0, 1.0)
    }

    fn snap(&self, i: usize) -> Snap {
        Snap::lerp(&self.prev[i], &self.cur[i], self.alpha())
    }

    fn update_camera(&mut self, dt: f32) {
        let s = self.snap(PLAYER);
        let b = &self.race.bikes[PLAYER];
        let fwd = Vec3::new(s.heading.cos(), 0.0, s.heading.sin());
        let speed = b.speed();
        if !self.cam_ready {
            self.cam_yaw = s.heading;
            self.cam_pos = s.pos - fwd * 6.0 + Vec3::Y * 2.0;
            self.cam_ready = true;
        }
        let k = 1.0 - (-(5.0 + speed * 0.05) * dt).exp();
        self.cam_yaw += game::bike::wrap_angle(s.heading - self.cam_yaw) * k;
        let boost = if b.boosting { 1.0 } else { 0.0 };
        let fov_target = 1.0 + (speed / BOOST_TOP_SPEED).min(1.2) * 0.2 + boost * 0.1;
        self.cam_fov += (fov_target - self.cam_fov) * (1.0 - (-3.0 * dt).exp());
        let roll_target = s.lean * 0.22;
        self.cam_roll += (roll_target - self.cam_roll) * (1.0 - (-6.0 * dt).exp());
        self.trauma = (self.trauma - dt * 1.4).max(0.0);
    }

    /// The camera for this frame.
    pub fn camera(&self) -> Camera {
        let s = self.snap(PLAYER);
        let b = &self.race.bikes[PLAYER];
        let track = &self.race.track;
        let yaw_fwd = Vec3::new(self.cam_yaw.cos(), 0.0, self.cam_yaw.sin());
        let speed = b.speed();
        let mut cam = Camera {
            fov_y: self.cam_fov,
            ..Default::default()
        };
        // Pre-race orbit swings in behind the bike; post-race it circles.
        let orbit = match self.race.phase {
            Phase::Countdown { t } => Some(((t / 2.6).min(1.0), 0.0)),
            Phase::Finished { t } => Some((1.0, t)),
            _ => None,
        };
        match self.cam_mode {
            CamMode::Hood if orbit.is_none() => {
                let m = bike_matrix(&s);
                cam.pos = m.transform_point3(Vec3::new(0.0, 1.38, -0.15));
                cam.target = m.transform_point3(Vec3::new(0.0, 1.2, -20.0));
                cam.up = m.transform_vector3(Vec3::Y);
                cam.fov_y = self.cam_fov * 1.08;
                cam.near = 0.1;
            }
            _ => {
                let far = self.cam_mode == CamMode::Far;
                let dist = if far { 8.8 } else { 5.0 } + speed * 0.022;
                let height = if far { 3.0 } else { 1.75 };
                let mut pos = s.pos - yaw_fwd * dist + Vec3::Y * height;
                let mut target = s.pos + Vec3::Y * 1.0 + yaw_fwd * 4.0;
                if let Some((u, spin)) = orbit {
                    let e = u * u * (3.0 - 2.0 * u);
                    let a = s.heading + (1.0 - e) * 2.6 + spin * 0.25;
                    let d = 7.5 - e * (7.5 - dist) + spin.min(8.0) * 0.4;
                    let dir = Vec3::new(a.cos(), 0.0, a.sin());
                    pos = s.pos - dir * d + Vec3::Y * (2.6 - e * (2.6 - height));
                    target = s.pos + Vec3::Y * 0.9 + yaw_fwd * (4.0 * e);
                }
                let ground = track.ground_height(pos.x, pos.z) + 0.6;
                pos.y = pos.y.max(ground);
                // Shake.
                let tr = self.trauma * self.trauma;
                let t = self.time * 31.0;
                let shake = Vec3::new(
                    (t * 1.3).sin(),
                    (t * 1.7 + 1.0).sin(),
                    (t * 1.1 + 2.0).sin(),
                ) * tr
                    * 0.12;
                let buzz = if b.boosting { 0.012 } else { 0.0 } + (speed / 80.0).powi(2) * 0.006;
                let jitter = Vec3::new((t * 3.1).sin(), (t * 2.7).cos(), 0.0) * buzz;
                cam.pos = pos + shake + jitter;
                cam.target = target + shake * 0.5;
                let roll = if orbit.is_some() { 0.0 } else { self.cam_roll };
                cam.up = Quat::from_axis_angle(yaw_fwd, roll) * Vec3::Y;
            }
        }
        cam
    }

    pub fn bike_draws(&self) -> Vec<BikeDraw> {
        (0..self.race.bikes.len())
            .map(|i| {
                let s = self.snap(i);
                let b = &self.race.bikes[i];
                let r = &self.race.racers[i];
                let model = bike_matrix(&s);
                let punch = if r.swing > 0.0 {
                    let ph = 1.0 - r.swing / SWING_SECS;
                    (ph * std::f32::consts::PI).sin().powf(0.7) * r.swing_side
                } else {
                    0.0
                };
                let (rider_offset, rider_spin) = if matches!(b.state, RiderState::Crashed { .. }) {
                    let hip = Vec3::new(0.0, 1.0, 0.1);
                    let world = s.pos + s.rider + Vec3::Y * 0.1;
                    (model.inverse().transform_point3(world) - hip, s.spin)
                } else {
                    (Vec3::ZERO, 0.0)
                };
                let (paint, leathers) = PAINTS[i % PAINTS.len()];
                BikeDraw {
                    model,
                    paint,
                    leathers,
                    ghost: (b.ghost / 2.0).min(1.0),
                    brake: b.brake,
                    front_spin: s.wheel,
                    rear_spin: s.wheel * 0.98,
                    steer: s.steer * 0.35,
                    punch,
                    rider_offset,
                    rider_spin,
                }
            })
            .collect()
    }

    pub fn particles(&self) -> Vec<Particle> {
        self.particles.iter().map(|l| l.p).collect()
    }

    pub fn start_lights(&self) -> f32 {
        match self.race.phase {
            Phase::Countdown { t } => {
                // Lights come on one by one over the countdown.
                (t / game::race::COUNTDOWN_SECS * 5.0)
                    .floor()
                    .clamp(0.0, 5.0)
            }
            _ if self.race.clock < 2.0 => 6.0,
            _ => 0.0,
        }
    }

    /// Everything the renderer needs this frame. `bikes`/`particles` are
    /// caller-owned scratch so the frame borrows them.
    pub fn frame_input<'a>(
        &self,
        bikes: &'a [BikeDraw],
        particles: &'a [Particle],
    ) -> FrameInput<'a> {
        let b = &self.race.bikes[PLAYER];
        let s = self.snap(PLAYER);
        let fwd = Vec3::new(s.heading.cos(), 0.0, s.heading.sin());
        FrameInput {
            camera: self.camera(),
            time: self.time,
            bikes,
            particles,
            start_lights: self.start_lights(),
            boost: if b.boosting { 1.0 } else { 0.0 },
            focus: s.pos + fwd * 30.0,
        }
    }

    pub fn hud(&self) -> Hud {
        let race = &self.race;
        let b = &race.bikes[PLAYER];
        let r = &race.racers[PLAYER];
        let lap = r.lap(race.track.length, race.laps).min(race.laps).max(1);
        let lap_time = if race.is_counting_down() {
            0.0
        } else {
            race.clock - r.lap_started
        };
        Hud {
            position: race.place_of(PLAYER) as u32,
            racers: race.bikes.len() as u32,
            lap,
            laps: race.laps,
            lap_time,
            last_lap: r.lap_times.last().copied(),
            best_lap: self.best_lap,
            race_time: race.clock,
            speed_kmh: b.forward_speed().max(0.0) * 3.6,
            gear: b.gear as u32,
            rpm: b.rpm,
            boost: b.boost,
            boosting: b.boosting,
            countdown: race.countdown_remaining(),
            wrong_way: r.wrong_way > 1.0,
            crashed: !b.is_riding(),
            finished: r.finish_time,
            balance: r.balance,
            progress: (r.progress / race.race_distance()).clamp(0.0, 1.0),
            drafting: b.drafting,
        }
    }

    /// Minimap points (x, z) for every bike, player first.
    pub fn bike_positions(&self) -> Vec<Vec2> {
        (0..self.race.bikes.len())
            .map(|i| {
                let s = self.snap(i);
                Vec2::new(s.pos.x, s.pos.z)
            })
            .collect()
    }

    pub fn standings_names(&self) -> Vec<(String, f32, bool)> {
        self.race
            .standings()
            .into_iter()
            .map(|(i, t, done)| (racer_name(i).to_string(), t, done))
            .collect()
    }
}

pub fn racer_name(i: usize) -> &'static str {
    if i == PLAYER {
        "You"
    } else {
        RIVAL_NAMES[(i - 1) % RIVAL_NAMES.len()]
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Hud {
    pub position: u32,
    pub racers: u32,
    pub lap: u32,
    pub laps: u32,
    pub lap_time: f32,
    pub last_lap: Option<f32>,
    pub best_lap: Option<f32>,
    pub race_time: f32,
    pub speed_kmh: f32,
    pub gear: u32,
    pub rpm: f32,
    pub boost: f32,
    pub boosting: bool,
    pub countdown: f32,
    pub wrong_way: bool,
    pub crashed: bool,
    pub finished: Option<f32>,
    pub balance: f32,
    pub progress: f32,
    pub drafting: bool,
}

fn bike_matrix(s: &Snap) -> Mat4 {
    let f = Vec3::new(s.heading.cos(), 0.0, s.heading.sin());
    let r = Vec3::new(-s.heading.sin(), 0.0, s.heading.cos());
    let basis = Mat4::from_cols(
        r.extend(0.0),
        Vec3::Y.extend(0.0),
        (-f).extend(0.0),
        s.pos.extend(1.0),
    );
    basis * Mat4::from_rotation_x(s.pitch) * Mat4::from_rotation_z(-s.lean)
}

#[cfg(test)]
mod tests {
    use super::*;
    use game::tracks::catalog;

    #[test]
    fn session_runs_a_race_start_with_autopilot() {
        let mut s = Session::new(catalog()[0], 3);
        s.autopilot = Some(Rival::new(0.0, 0.9, 0.0));
        let input = Input::default();
        let mut cues = Vec::new();
        for _ in 0..(12.0 * 60.0) as usize {
            s.update(1.0 / 60.0, &input);
            cues.extend(s.cues.iter().copied());
        }
        assert!(cues.contains(&Cue::Go));
        assert!(
            s.race.racers[PLAYER].progress > 150.0,
            "{}",
            s.race.racers[PLAYER].progress
        );
        let cam = s.camera();
        assert!(cam.pos.is_finite() && cam.target.is_finite());
        assert_eq!(s.bike_draws().len(), 8);
    }

    #[test]
    fn bike_matrix_points_forward_and_leans_right() {
        let s = Snap {
            pos: Vec3::ZERO,
            heading: 0.0,
            lean: 0.5,
            pitch: 0.0,
            wheel: 0.0,
            steer: 0.0,
            rider: Vec3::ZERO,
            spin: 0.0,
        };
        let m = bike_matrix(&s);
        let nose = m.transform_point3(Vec3::new(0.0, 0.0, -1.0));
        assert!((nose - Vec3::X).length() < 1e-4);
        // Positive lean tilts the top towards the right (+z for heading 0).
        let top = m.transform_point3(Vec3::Y);
        assert!(top.z > 0.3);
    }
}
