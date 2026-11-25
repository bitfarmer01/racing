//! Rival riders.
//!
//! Each rival follows a lookahead point on its own lane (cutting to the
//! inside of corners), plans its speed from the curvature ahead against its
//! grip budget, swings out to pass slower bikes, and picks fights (punches
//! and kicks) when a bike rides alongside. Guarding is decided by the race,
//! which sees strikes coming.

use glam::Vec2;

use crate::bike::{Bike, Controls, BRAKE_DECEL, GRIP_ROAD};
use crate::track::Track;

#[derive(Debug, Clone)]
pub struct Rival {
    /// Preferred lane, -1 (left) .. 1 (right).
    pub lane: f32,
    /// How close to its grip limit it dares to corner (0.8..0.97).
    pub commitment: f32,
    /// Likelihood of swinging at a bike alongside (0..1).
    pub aggression: f32,
    pub attack_cooldown: f32,
    /// Temporary lane shift while passing.
    pub dodge: f32,
    pub dodge_timer: f32,
    pub boost_timer: f32,
}

impl Rival {
    pub fn new(lane: f32, commitment: f32, aggression: f32) -> Self {
        Self {
            lane,
            commitment,
            aggression,
            attack_cooldown: 2.0,
            dodge: 0.0,
            dodge_timer: 0.0,
            boost_timer: 0.0,
        }
    }

    /// Decides this tick's controls. `others` are every other bike; `roll`
    /// is a fresh uniform random number.
    pub fn drive(
        &mut self,
        me: &Bike,
        others: &[&Bike],
        track: &Track,
        dt: f32,
        roll: f32,
    ) -> Controls {
        self.attack_cooldown = (self.attack_cooldown - dt).max(0.0);
        self.dodge_timer = (self.dodge_timer - dt).max(0.0);
        if self.dodge_timer <= 0.0 {
            self.dodge *= 1.0 - (2.0 * dt).min(1.0);
        }
        let v = me.forward_speed().max(0.0);
        let s = me.track_pos.s;
        let hw = track.half_width;

        // Traffic: swing out around a slower bike ahead in our lane.
        let fwd = me.fwd();
        let mut alongside = None;
        let mut ahead_slow = false;
        for o in others.iter().filter(|o| o.is_solid()) {
            let d = track.delta_s(s, o.track_pos.s);
            let lat = o.track_pos.lateral - me.track_pos.lateral;
            if (2.0..28.0).contains(&d) && lat.abs() < 1.8 && o.forward_speed() < v + 2.0 {
                ahead_slow = true;
                if self.dodge_timer <= 0.0 {
                    let room_left = o.track_pos.lateral + hw;
                    let room_right = hw - o.track_pos.lateral;
                    self.dodge = if room_right > room_left { 2.6 } else { -2.6 };
                    self.dodge_timer = 1.6;
                }
            }
            if d.abs() < 1.6 && (0.4..2.6).contains(&lat.abs()) {
                alongside = Some(o.pos2());
            }
        }

        // Line: lane + inside of the corner ahead.
        let look = 9.0 + v * 0.55;
        let curv_ahead = track.frame_at(s + look * 1.4).curvature;
        let inside = (curv_ahead * 500.0).clamp(-1.0, 1.0) * hw * 0.55;
        let mut target_lat = self.lane * hw * 0.45 + inside + self.dodge;
        if me.track_pos.lateral.abs() > hw {
            target_lat = 0.0;
        }
        target_lat = target_lat.clamp(-hw + 1.2, hw - 1.2);
        let tp = track.point_at(s + look, target_lat);
        let to = Vec2::new(tp.x, tp.z) - me.pos2();
        let alpha = fwd.perp_dot(to.normalize_or_zero()).asin();
        let want_yaw = 2.0 * alpha.sin() * v.max(6.0) / look;
        let max_yaw = (GRIP_ROAD * me.tuning.grip / v.max(1.0)).min(2.4);
        let steer = (want_yaw / max_yaw).clamp(-1.0, 1.0);

        // Speed: the slowest of the corner limits ahead, reachable by braking.
        let grip = GRIP_ROAD * me.tuning.grip * self.commitment;
        let horizon = (v * v / (2.0 * BRAKE_DECEL * 0.6) + 40.0).min(220.0);
        let mut target_v = f32::MAX;
        let mut d = 0.0;
        while d <= horizon {
            let k = track.frame_at(s + d).curvature.abs().max(1e-4);
            let v_corner = (grip / k).sqrt();
            let v_here = (v_corner * v_corner + 2.0 * BRAKE_DECEL * 0.6 * d).sqrt();
            target_v = target_v.min(v_here);
            d += 6.0;
        }
        // Running out of grip on the current line: scrub speed now.
        let overload = want_yaw.abs() / max_yaw;
        if overload > 0.92 && v > 14.0 {
            target_v = target_v.min(v - 4.0 * (overload - 0.9).min(1.0) * 10.0);
        }
        let (throttle, brake) = if v > target_v + 1.5 {
            (0.0, ((v - target_v) / 6.0).clamp(0.3, 1.0))
        } else if v > target_v {
            (0.2, 0.0)
        } else {
            (1.0, 0.0)
        };

        // Boost on straights when the meter is healthy.
        self.boost_timer = (self.boost_timer - dt).max(0.0);
        if self.boost_timer <= 0.0
            && me.boost > 0.6
            && curv_ahead.abs() < 0.004
            && target_v > v + 15.0
            && roll < 0.02
        {
            self.boost_timer = 2.5;
        }
        let boost = self.boost_timer > 0.0 && throttle > 0.9;

        let strike = alongside.is_some()
            && self.attack_cooldown <= 0.0
            && roll < self.aggression * 0.05
            && !ahead_slow;
        if strike {
            self.attack_cooldown = 1.4 + (1.0 - self.aggression) * 2.0;
        }
        // About a third of strikes are kicks; the angrier the rider, the more.
        let kick = strike && roll < self.aggression * 0.05 * (0.2 + 0.3 * self.aggression);

        Controls {
            throttle,
            brake,
            steer,
            boost,
            attack: strike && !kick,
            kick,
            block: false,
        }
    }
}
