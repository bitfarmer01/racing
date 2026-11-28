//! Motorcycle physics.
//!
//! Arcade model on the ground plane with a real grip budget: steering asks
//! for a yaw rate, the tyres can only deliver `grip / speed` of it, so a
//! corner taken too fast washes wide and the rider has to brake for it.
//! Sideways velocity bleeds off at the tyres' rate, which gives a short,
//! readable slide (and tyre smoke) whenever the bike is pushed past grip.
//! Height follows the ground with gravity, so crests lift the bike briefly.

use glam::{Vec2, Vec3};

use crate::track::{Track, TrackPos, RUNOFF};

pub const GRAVITY: f32 = 9.81;
/// Top speed on tarmac, m/s (~245 km/h).
pub const TOP_SPEED: f32 = 68.0;
/// Extra top speed while boosting.
pub const BOOST_TOP_SPEED: f32 = 82.0;
pub const OFFROAD_TOP_SPEED: f32 = 30.0;
/// Peak engine acceleration from a standstill, m/s².
pub const ENGINE_ACCEL: f32 = 13.5;
pub const BOOST_ACCEL: f32 = 9.0;
pub const BRAKE_DECEL: f32 = 30.0;
pub const REVERSE_SPEED: f32 = 6.0;
/// Lateral grip on tarmac and grass, m/s².
pub const GRIP_ROAD: f32 = 17.0;
pub const GRIP_GRASS: f32 = 9.5;
/// Low-speed steering yaw-rate limit, rad/s.
pub const MAX_LOW_SPEED_YAW: f32 = 2.4;
/// Wall impact speed (normal component) that throws the rider off.
pub const WALL_CRASH_SPEED: f32 = 17.0;
pub const CRASH_SECS: f32 = 2.4;
pub const GHOST_SECS: f32 = 2.0;
pub const WHEEL_RADIUS: f32 = 0.32;
/// Boost meter drain per second while boosting.
pub const BOOST_DRAIN: f32 = 0.30;
pub const BOOST_REFILL: f32 = 0.035;
/// Static suspension sag and travel, metres.
pub const SUSP_SAG: f32 = 0.03;
pub const SUSP_TRAVEL: f32 = 0.11;
/// Top-speed gain when fully tucked.
pub const TUCK_GAIN: f32 = 0.025;
pub const GEAR_TOPS: [f32; 6] = [14.0, 24.0, 34.0, 45.0, 57.0, BOOST_TOP_SPEED];

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Controls {
    /// 0..1
    pub throttle: f32,
    /// 0..1
    pub brake: f32,
    /// -1 (left) .. 1 (right)
    pub steer: f32,
    pub boost: bool,
    /// Punch towards the nearest bike alongside.
    pub attack: bool,
    /// Kick: slower, longer reach, hits harder.
    pub kick: bool,
    /// Guard up: soaks most of a hit; raised just before one lands, it
    /// parries. The rider sits up, so the bike pulls a little less.
    pub block: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Road,
    Grass,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RiderState {
    Riding,
    /// Thrown off; counts up to `CRASH_SECS`, then respawns.
    Crashed {
        t: f32,
    },
}

/// Per-bike performance knobs (rivals differ slightly).
#[derive(Debug, Clone, Copy)]
pub struct Tuning {
    pub power: f32,
    pub grip: f32,
    pub top_speed: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            power: 1.0,
            grip: 1.0,
            top_speed: 1.0,
        }
    }
}

/// What happened to a bike this tick, for effects and sound.
#[derive(Debug, Clone, Copy, Default)]
pub struct TickEffects {
    /// Normal speed of a barrier contact this tick (0 = none).
    pub wall_hit: f32,
    pub crashed: bool,
    pub landed: f32,
}

#[derive(Debug, Clone)]
pub struct Bike {
    pub pos: Vec3,
    pub vel: Vec2,
    pub vy: f32,
    pub heading: f32,
    pub yaw_rate: f32,
    pub lean: f32,
    pub pitch: f32,
    pub steer: f32,
    pub throttle: f32,
    pub brake: f32,
    pub on_ground: bool,
    pub surface: Surface,
    pub wheel_angle: f32,
    pub slip: f32,
    pub rpm: f32,
    pub gear: u8,
    pub boost: f32,
    pub boosting: bool,
    pub drafting: bool,
    pub state: RiderState,
    pub ghost: f32,
    pub track_pos: TrackPos,
    pub tuning: Tuning,
    /// Rider thrown clear during a crash: offset from the bike and spin.
    pub rider_offset: Vec3,
    pub rider_vel: Vec3,
    pub rider_spin: f32,
    /// Fork and shock compression (m) and their rates.
    pub susp: [f32; 2],
    pub susp_vel: [f32; 2],
    /// 0..1 rider folded behind the screen: flat out in a straight line.
    pub tuck: f32,
    /// Seconds of wheelspin left from a botched start.
    pub wheelspin: f32,
    /// Thrown into the air off a ramp: ordinary gravity until it lands.
    pub launched: bool,
    /// Seconds of the current jump so far (0 on the ground).
    pub air_time: f32,
}

impl Bike {
    pub fn new(track: &Track, s: f32, lateral: f32, tuning: Tuning) -> Self {
        let f = track.frame_at(s);
        let p = track.point_at(s, lateral);
        let tp = track.locate(Vec2::new(p.x, p.z));
        Self {
            pos: p,
            vel: Vec2::ZERO,
            vy: 0.0,
            heading: f.heading(),
            yaw_rate: 0.0,
            lean: 0.0,
            pitch: 0.0,
            steer: 0.0,
            throttle: 0.0,
            brake: 0.0,
            on_ground: true,
            launched: false,
            air_time: 0.0,
            surface: Surface::Road,
            wheel_angle: 0.0,
            slip: 0.0,
            rpm: 0.0,
            gear: 1,
            boost: 0.35,
            boosting: false,
            drafting: false,
            state: RiderState::Riding,
            ghost: 0.0,
            track_pos: tp,
            tuning,
            rider_offset: Vec3::ZERO,
            rider_vel: Vec3::ZERO,
            rider_spin: 0.0,
            susp: [SUSP_SAG; 2],
            susp_vel: [0.0; 2],
            tuck: 0.0,
            wheelspin: 0.0,
        }
    }

    pub fn fwd(&self) -> Vec2 {
        Vec2::new(self.heading.cos(), self.heading.sin())
    }

    pub fn right(&self) -> Vec2 {
        Vec2::new(-self.heading.sin(), self.heading.cos())
    }

    pub fn speed(&self) -> f32 {
        self.vel.length()
    }

    pub fn forward_speed(&self) -> f32 {
        self.vel.dot(self.fwd())
    }

    pub fn is_riding(&self) -> bool {
        matches!(self.state, RiderState::Riding)
    }

    /// Bike-bike collisions and attacks skip ghosts and fallen riders.
    pub fn is_solid(&self) -> bool {
        self.is_riding() && self.ghost <= 0.0
    }

    pub fn pos2(&self) -> Vec2 {
        Vec2::new(self.pos.x, self.pos.z)
    }

    /// Throws the rider off.
    pub fn crash(&mut self) {
        if !self.is_riding() {
            return;
        }
        self.state = RiderState::Crashed { t: 0.0 };
        self.boosting = false;
        let v = self.vel;
        self.rider_offset = Vec3::new(0.0, 0.9, 0.0);
        self.rider_vel = Vec3::new(v.x * 0.9, 3.5, v.y * 0.9);
        self.rider_spin = 0.0;
    }

    pub fn step(&mut self, track: &Track, c: &Controls, dt: f32) -> TickEffects {
        let mut fx = TickEffects::default();
        self.ghost = (self.ghost - dt).max(0.0);
        match self.state {
            RiderState::Riding => self.ride(track, c, dt, &mut fx),
            RiderState::Crashed { t } => {
                self.tumble(track, dt);
                let t = t + dt;
                if t >= CRASH_SECS {
                    self.respawn(track);
                } else {
                    self.state = RiderState::Crashed { t };
                }
            }
        }
        fx
    }

    fn ride(&mut self, track: &Track, c: &Controls, dt: f32, fx: &mut TickEffects) {
        // Steering arrives already shaped (the shell eases keyboard input).
        self.steer = c.steer.clamp(-1.0, 1.0);
        self.throttle = c.throttle.clamp(0.0, 1.0);
        self.brake = c.brake.clamp(0.0, 1.0);

        let fwd = self.fwd();
        let right = self.right();
        let v_long_before = self.vel.dot(fwd);
        let mut v_long = self.vel.dot(fwd);
        let mut v_lat = self.vel.dot(right);
        let speed = self.vel.length();

        self.surface =
            if self.track_pos.lateral.abs() <= track.half_width_at(self.track_pos.s) + 0.4 {
                Surface::Road
            } else {
                Surface::Grass
            };
        let on_road = self.surface == Surface::Road;

        // Boost: needs meter and the ground under the wheels.
        self.boosting = c.boost && self.boost > 0.02 && self.on_ground && v_long > 2.0;
        if self.boosting {
            self.boost = (self.boost - BOOST_DRAIN * dt).max(0.0);
        } else {
            self.boost = (self.boost + BOOST_REFILL * dt).min(1.0);
        }

        // Longitudinal.
        let top = if self.boosting {
            BOOST_TOP_SPEED
        } else if on_road {
            TOP_SPEED
        } else {
            OFFROAD_TOP_SPEED
        } * self.tuning.top_speed
            * (1.0 + TUCK_GAIN * self.tuck);
        // A botched start spins the rear tyre: drive goes up in smoke.
        self.wheelspin = (self.wheelspin - dt).max(0.0);
        let spin_loss = if self.wheelspin > 0.0 { 0.3 } else { 1.0 };
        let traction = if self.on_ground { spin_loss } else { 0.0 };
        let mut a = 0.0;
        if v_long < top {
            let frac = (v_long.max(0.0) / top).min(1.0);
            let pull = ENGINE_ACCEL
                * self.tuning.power
                * (1.0 - frac.powf(2.4))
                * (0.55 + 0.45 * (1.0 - frac));
            a += self.throttle * pull;
            if self.drafting {
                a += 1.6 * self.throttle;
            }
        } else {
            // Above the cap (boost ended, ran onto grass): ease down.
            a -= (v_long - top) * if on_road { 0.6 } else { 2.0 };
        }
        if self.boosting {
            a += BOOST_ACCEL;
        }
        if self.brake > 0.0 {
            if v_long > 0.5 {
                a -= self.brake * BRAKE_DECEL;
            } else if self.throttle < 0.1 && v_long > -REVERSE_SPEED {
                a -= self.brake * 5.0;
            }
        }
        // Rolling resistance and air drag; grass is draggy.
        a -= v_long.signum() * (0.35 + 0.00035 * v_long * v_long);
        if !on_road {
            a -= v_long.signum() * 3.5 * (v_long.abs() / 10.0).min(1.0);
        }
        // Gravity along the slope.
        a -= GRAVITY * self.pitch.sin();
        v_long += a * traction * dt;
        if self.brake > 0.0 && self.throttle < 0.1 && v_long.abs() < 0.3 && a < 0.0 {
            v_long = v_long.min(0.0);
        }

        // Steering: the tyres deliver at most grip/speed of yaw rate.
        let grip = if on_road { GRIP_ROAD } else { GRIP_GRASS } * self.tuning.grip;
        let low = MAX_LOW_SPEED_YAW * (speed / 5.0).min(1.0);
        let max_yaw = low.min(grip / speed.max(1.0));
        let dir = if v_long < -0.5 { -1.0 } else { 1.0 };
        let want = self.steer * max_yaw * dir;
        let response = if self.on_ground { 10.0 } else { 1.5 };
        self.yaw_rate += (want - self.yaw_rate) * (1.0 - (-response * dt).exp());
        self.heading += self.yaw_rate * dt;

        // Re-express the velocity in the new heading: the turn leaves some
        // of it sideways, which the tyres then scrub away.
        let fwd2 = self.fwd();
        let right2 = self.right();
        let vel = fwd * v_long + right * v_lat;
        v_long = vel.dot(fwd2);
        v_lat = vel.dot(right2);
        let lat_grip = if !self.on_ground {
            0.3
        } else if on_road {
            7.5 * self.tuning.grip
        } else {
            3.0
        };
        let before = v_lat;
        v_lat *= (-lat_grip * dt).exp();
        // Hard braking with lots of lean unsettles the rear.
        let lost = (before - v_lat).abs();
        v_long -= v_long.signum() * lost * 0.15;
        self.slip = (v_lat.abs() / 3.0
            + if self.brake > 0.6 && speed > 20.0 {
                0.35
            } else {
                0.0
            })
        .min(1.0);
        self.vel = fwd2 * v_long + right2 * v_lat;

        // Move and resolve the barrier.
        let prev_ground = self.ground(track);
        self.pos.x += self.vel.x * dt;
        self.pos.z += self.vel.y * dt;
        self.relocate(track);
        let limit = track.half_width_at(self.track_pos.s) + RUNOFF;
        if self.track_pos.lateral.abs() > limit {
            let f = track.frame_at(self.track_pos.s);
            let side = self.track_pos.lateral.signum();
            let n = f.right * side;
            let push = self.track_pos.lateral.abs() - limit;
            self.pos.x -= n.x * push;
            self.pos.z -= n.y * push;
            let vn = self.vel.dot(n);
            if vn > 0.0 {
                fx.wall_hit = vn;
                self.vel -= n * vn * 1.35;
                self.vel *= 0.9;
                // Glance off: turn the nose back along the barrier.
                let along = f.heading();
                let diff = wrap_angle(along - self.heading);
                let toward = if diff.abs() < std::f32::consts::FRAC_PI_2 {
                    diff
                } else {
                    wrap_angle(diff + std::f32::consts::PI)
                };
                self.heading += toward * 0.35;
                if vn > WALL_CRASH_SPEED {
                    self.crash();
                    fx.crashed = true;
                }
            }
            self.relocate(track);
        }

        // Vertical: gravity, with the ground pushing back.
        // Close to the ground the suspension keeps the tyres planted (an
        // arcade 2.5 g of pull), so only a real kicker launches the bike.
        let ground = self.ground(track);
        // Ground speed from position differences, capped at a steep road's
        // grade so stepping up a kerb or shoulder can't launch the bike.
        // A ramp's face is the one place the ground may throw the bike up.
        let horiz = self.vel.length();
        let ramp = track.ramp_at(self.track_pos.s, self.track_pos.lateral);
        let ramp_slope = ramp.map_or(0.0, |r| r.1);
        let cap = horiz * (0.1 + ramp_slope) + 0.5;
        let ground_vy = ((ground - prev_ground) / dt).clamp(-cap - 30.0, cap.min(8.5));
        if ramp_slope > 0.08 && horiz > 8.0 {
            self.launched = true;
        }
        let gap = self.pos.y - ground;
        let vy_before = self.vy;
        let pull = if gap < 0.8 && !self.launched {
            GRAVITY * 2.5
        } else {
            GRAVITY
        };
        self.vy -= pull * dt;
        self.pos.y += self.vy * dt;
        if self.pos.y <= ground {
            if !self.on_ground && self.vy < -4.0 {
                fx.landed = -self.vy;
            }
            self.pos.y = ground;
            self.vy = ground_vy.max(self.vy);
            self.vy = self.vy.min(ground_vy.max(0.0) + 0.5);
            self.on_ground = true;
            if ramp.is_none() {
                self.launched = false;
            }
            self.air_time = 0.0;
        } else {
            self.on_ground = self.pos.y - ground < 0.25;
            if !self.on_ground {
                self.air_time += dt;
            }
        }

        // Pose.
        let ahead = self.ground_at_offset(track, 0.8);
        let behind = self.ground_at_offset(track, -0.8);
        let slope = ((ahead - behind) / 1.6).atan();
        let target_pitch = if self.on_ground {
            slope
        } else {
            self.pitch * 0.98
        };
        self.pitch += (target_pitch - self.pitch) * (1.0 - (-12.0 * dt).exp());
        let lean_target = (v_long.max(0.0) * self.yaw_rate / GRAVITY)
            .atan()
            .clamp(-0.95, 0.95);
        self.lean += (lean_target - self.lean) * (1.0 - (-9.0 * dt).exp());
        self.wheel_angle = (self.wheel_angle + v_long / WHEEL_RADIUS * dt) % std::f32::consts::TAU;
        self.update_engine(v_long);

        // Tuck in a straight line flat out; sit up to brake, turn or fight.
        let flat_out = self.on_ground
            && self.brake < 0.05
            && self.throttle > 0.9
            && self.steer.abs() < 0.25
            && v_long > 30.0;
        let tuck_target = if flat_out || self.boosting { 1.0 } else { 0.0 };
        self.tuck += (tuck_target - self.tuck) * (1.0 - (-4.0 * dt).exp());

        let a_long = (v_long - v_long_before) / dt;
        let a_vert = if self.on_ground {
            (self.vy - vy_before) / dt
        } else {
            0.0
        };
        self.update_suspension(a_long, a_vert, on_road, fx.landed, dt);
    }

    /// Fork and shock as two springs: braking dives the front, drive squats
    /// the rear, crests and dips load both, landings bottom them out.
    fn update_suspension(&mut self, a_long: f32, a_vert: f32, on_road: bool, landed: f32, dt: f32) {
        let heave = if self.on_ground {
            (a_vert * 0.0025).clamp(-0.025, 0.05)
        } else {
            -SUSP_SAG
        };
        let pitch = (a_long * 0.0022).clamp(-0.05, 0.035);
        // Grass is bumpy.
        let rough = if on_road || !self.on_ground {
            0.0
        } else {
            (self.track_pos.s * 9.1).sin() * 0.012 + (self.track_pos.s * 23.7).sin() * 0.006
        };
        let target = [
            SUSP_SAG + heave - pitch + rough,
            SUSP_SAG + heave + pitch * 0.8 - rough,
        ];
        for ((x, v), t) in self.susp.iter_mut().zip(&mut self.susp_vel).zip(target) {
            if landed > 0.0 {
                *v += landed * 0.06;
            }
            let acc = 260.0 * (t - *x) - 22.0 * *v;
            *v += acc * dt;
            *x += *v * dt;
            if *x < 0.0 || *x > SUSP_TRAVEL {
                *x = x.clamp(0.0, SUSP_TRAVEL);
                *v *= -0.2;
            }
        }
    }

    fn update_engine(&mut self, v_long: f32) {
        let v = v_long.abs();
        let mut gear = 0;
        while gear < GEAR_TOPS.len() - 1 && v > GEAR_TOPS[gear] * 0.97 {
            gear += 1;
        }
        let lo = if gear == 0 {
            0.0
        } else {
            GEAR_TOPS[gear - 1] * 0.97
        };
        let hi = GEAR_TOPS[gear];
        let band = ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
        let floor = if gear == 0 { 0.18 } else { 0.55 };
        let target =
            1_600.0 + (floor + (1.0 - floor) * band) * 12_000.0 * (0.6 + 0.4 * self.throttle);
        self.rpm += (target - self.rpm) * 0.25;
        self.gear = gear as u8 + 1;
    }

    fn tumble(&mut self, track: &Track, dt: f32) {
        // The bike slides on its side and grinds to a halt.
        let speed = self.vel.length();
        let decel = 7.0 * dt;
        if speed > decel {
            self.vel *= (speed - decel) / speed;
        } else {
            self.vel = Vec2::ZERO;
        }
        self.pos.x += self.vel.x * dt;
        self.pos.z += self.vel.y * dt;
        self.relocate(track);
        let limit = track.half_width_at(self.track_pos.s) + RUNOFF - 0.5;
        if self.track_pos.lateral.abs() > limit {
            let f = track.frame_at(self.track_pos.s);
            let n = f.right * self.track_pos.lateral.signum();
            let push = self.track_pos.lateral.abs() - limit;
            self.pos.x -= n.x * push;
            self.pos.z -= n.y * push;
            let vn = self.vel.dot(n);
            if vn > 0.0 {
                self.vel -= n * vn * 1.2;
            }
            self.relocate(track);
        }
        self.pos.y = self.ground(track);
        let side = if self.lean >= 0.0 { 1.0 } else { -1.0 };
        self.lean += (side * 1.42 - self.lean) * (1.0 - (-6.0 * dt).exp());
        self.yaw_rate *= 0.97;
        self.heading += self.yaw_rate * dt + speed * 0.004 * side * dt * 10.0;
        self.slip = (speed / 20.0).min(1.0);
        self.boosting = false;

        // The rider flies, bounces and rolls.
        self.rider_vel.y -= GRAVITY * dt;
        self.rider_offset += (self.rider_vel - Vec3::new(self.vel.x, 0.0, self.vel.y)) * dt;
        let rider_world_y = self.pos.y + self.rider_offset.y;
        let rx = self.pos.x + self.rider_offset.x;
        let rz = self.pos.z + self.rider_offset.z;
        let g = track.ground_height(rx, rz);
        if rider_world_y < g + 0.25 {
            self.rider_offset.y = g + 0.25 - self.pos.y;
            if self.rider_vel.y < 0.0 {
                self.rider_vel.y *= -0.35;
            }
            let h = Vec2::new(self.rider_vel.x, self.rider_vel.z) * (1.0 - 4.0 * dt).max(0.0);
            self.rider_vel.x = h.x;
            self.rider_vel.z = h.y;
        }
        self.rider_spin += Vec2::new(self.rider_vel.x, self.rider_vel.z).length() * dt * 1.6;
    }

    /// Back on the bike: centred-ish on the road, facing the right way.
    pub fn respawn(&mut self, track: &Track) {
        let s = self.track_pos.s;
        let hw = track.half_width_at(s);
        let lat = self.track_pos.lateral.clamp(-hw + 1.5, hw - 1.5);
        let f = track.frame_at(s);
        self.pos = track.point_at(s, lat);
        self.heading = f.heading();
        self.vel = f.fwd * 6.0;
        self.vy = 0.0;
        self.launched = false;
        self.air_time = 0.0;
        self.yaw_rate = 0.0;
        self.lean = 0.0;
        self.pitch = 0.0;
        self.steer = 0.0;
        self.state = RiderState::Riding;
        self.ghost = GHOST_SECS;
        self.rider_offset = Vec3::ZERO;
        self.rider_vel = Vec3::ZERO;
        self.rider_spin = 0.0;
        self.slip = 0.0;
        self.susp = [SUSP_SAG; 2];
        self.susp_vel = [0.0; 2];
        self.tuck = 0.0;
        self.wheelspin = 0.0;
        self.relocate(track);
    }

    pub fn relocate(&mut self, track: &Track) {
        self.track_pos = track.locate_near(self.pos2(), self.track_pos.index, 14);
    }

    fn ground(&self, track: &Track) -> f32 {
        track.ground_at(self.pos.x, self.pos.z, &self.track_pos)
    }

    fn ground_at_offset(&self, track: &Track, d: f32) -> f32 {
        let f = self.fwd();
        let (x, z) = (self.pos.x + f.x * d, self.pos.z + f.y * d);
        let tp = track.locate_near(Vec2::new(x, z), self.track_pos.index, 3);
        track.ground_at(x, z, &tp)
    }
}

pub fn wrap_angle(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    let mut a = a % t;
    if a > std::f32::consts::PI {
        a -= t;
    } else if a < -std::f32::consts::PI {
        a += t;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::catalog;

    const DT: f32 = 1.0 / 120.0;

    fn straight_test(track: &Track) -> Bike {
        // Start of each catalogue circuit is a straight.
        Bike::new(track, 2.0, 0.0, Tuning::default())
    }

    #[test]
    fn full_throttle_reaches_highway_speed_quickly() {
        let track = Track::build(catalog()[0]);
        let mut b = straight_test(&track);
        let c = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        let mut t = 0.0;
        while b.speed() < 27.8 && t < 10.0 {
            b.step(&track, &c, DT);
            t += DT;
        }
        // 0-100 km/h in ~3 s feels sporty without being twitchy.
        assert!((2.0..4.0).contains(&t), "0-100 took {t}");
    }

    #[test]
    fn braking_stops_hard() {
        let track = Track::build(catalog()[0]);
        let mut b = straight_test(&track);
        b.vel = b.fwd() * 40.0;
        let c = Controls {
            brake: 1.0,
            ..Controls::default()
        };
        let mut t = 0.0;
        while b.forward_speed() > 0.5 && t < 10.0 {
            b.step(&track, &c, DT);
            t += DT;
        }
        assert!(t < 2.0, "stopping from 144 km/h took {t}");
    }

    #[test]
    fn steering_is_grip_limited() {
        let track = Track::build(catalog()[0]);
        let mut b = straight_test(&track);
        b.vel = b.fwd() * 50.0;
        let c = Controls {
            throttle: 1.0,
            steer: 1.0,
            ..Controls::default()
        };
        for _ in 0..60 {
            b.step(&track, &c, DT);
        }
        let lateral_accel = b.yaw_rate * b.forward_speed();
        assert!(lateral_accel <= GRIP_ROAD * 1.05, "{lateral_accel}");
        assert!(b.yaw_rate > 0.0, "steer right turns right");
        assert!(b.lean > 0.3, "leans into the turn: {}", b.lean);
    }

    #[test]
    fn ramps_throw_the_bike_into_the_air_and_it_lands() {
        let track = Track::build(catalog()[0]);
        let r = track.ramps[0];
        let s0 = r.s - 40.0;
        let mut b = Bike::new(&track, s0, r.lateral, Tuning::default());
        b.vel = b.fwd() * 38.0;
        let c = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        let (mut air, mut peak) = (0.0f32, 0.0f32);
        for _ in 0..(5.0 / DT) as usize {
            // Hold the ramp's line (straight road, so a nudge is enough).
            let err = r.lateral - b.track_pos.lateral;
            b.heading = track.frame_at(b.track_pos.s).heading() + (err * 0.02).clamp(-0.05, 0.05);
            b.step(&track, &c, DT);
            air = air.max(b.air_time);
            let g = track.ground_at(b.pos.x, b.pos.z, &b.track_pos);
            peak = peak.max(b.pos.y - g);
        }
        assert!(air > 0.8, "air time {air}");
        assert!(peak > 1.8, "peak height {peak}");
        assert!(b.on_ground && b.is_riding(), "landed and still riding");
        // Riding alongside the ramp leaves the road flat.
        assert!(track
            .ramp_at(r.s + 5.0, r.lateral - 3.0 * r.lateral.signum())
            .is_none());
    }

    #[test]
    fn barrier_contains_the_bike() {
        let track = Track::build(catalog()[0]);
        let mut b = straight_test(&track);
        let c = Controls {
            throttle: 1.0,
            steer: -1.0,
            ..Controls::default()
        };
        for _ in 0..1200 {
            b.step(&track, &c, DT);
            assert!(b.track_pos.lateral.abs() <= track.max_half_width() + RUNOFF + 0.5);
        }
    }

    #[test]
    fn crashed_rider_respawns_on_the_road() {
        let track = Track::build(catalog()[1]);
        let mut b = straight_test(&track);
        b.vel = b.fwd() * 30.0;
        b.crash();
        for _ in 0..((CRASH_SECS / DT) as usize + 2) {
            b.step(&track, &Controls::default(), DT);
        }
        assert!(b.is_riding());
        assert!(b.ghost > 0.0);
        assert!(b.track_pos.lateral.abs() < track.half_width);
    }

    #[test]
    fn suspension_dives_under_braking_and_tucks_flat_out() {
        let track = Track::build(catalog()[0]);
        let mut b = straight_test(&track);
        b.vel = b.fwd() * 40.0;
        let flat = Controls {
            throttle: 1.0,
            ..Controls::default()
        };
        for _ in 0..(1.2 / DT) as usize {
            b.step(&track, &flat, DT);
        }
        assert!(
            b.tuck > 0.8,
            "tucked at {:.0} m/s: {}",
            b.forward_speed(),
            b.tuck
        );
        let settled = b.susp[0];
        let brake = Controls {
            brake: 1.0,
            ..Controls::default()
        };
        let mut deepest: f32 = 0.0;
        for _ in 0..(0.4 / DT) as usize {
            b.step(&track, &brake, DT);
            deepest = deepest.max(b.susp[0]);
        }
        assert!(deepest > settled + 0.02, "fork {settled} -> {deepest}");
        assert!(b.tuck < 0.5, "sat up to brake");
        assert!(b.susp.iter().all(|x| (0.0..=SUSP_TRAVEL).contains(x)));
    }
}
