//! Rival riders.
//!
//! Each rival follows a lookahead point on its own lane (cutting to the
//! inside of corners), plans its speed from the curvature ahead against its
//! grip budget, swings out to pass slower bikes, threads round road traffic,
//! and picks fights (punches and kicks) when a bike rides alongside.
//!
//! Brawlers, and any rival the player has hit, go further: they hunt the
//! player down, ride up alongside and match speed to fight. Guarding is
//! decided by the race, which sees strikes coming.

use glam::Vec2;

use crate::bike::{wrap_angle, Bike, Controls, BRAKE_DECEL, GRIP_ROAD};
use crate::track::Track;
use crate::traffic::Car;

/// How long a grudge lasts after the player lands a hit.
pub const GRUDGE_SECS: f32 = 12.0;
/// A brawler hunts for this long, then rests before it hunts again.
const HUNT_SECS: f32 = 14.0;
const REST_SECS: f32 = 9.0;

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
    /// Comes looking for the player.
    pub brawler: bool,
    /// Seconds left of a grudge against the player.
    pub grudge: f32,
    hunt_t: f32,
    rest_t: f32,
    /// True while riding to fight the player this tick.
    pub hunting: bool,
    /// Steering round a car this tick.
    pub avoiding: bool,
    /// The car being avoided (index into the traffic) and the side chosen.
    avoid: Option<(usize, f32)>,
    /// Lateral it is steering for this tick.
    pub target_lat: f32,
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
            brawler: false,
            grudge: 0.0,
            hunt_t: 0.0,
            rest_t: 4.0,
            hunting: false,
            avoiding: false,
            avoid: None,
            target_lat: 0.0,
        }
    }

    /// Decides this tick's controls. `others` are every other bike, `cars`
    /// the road traffic, `prey` the player's bike; `roll` is a fresh uniform
    /// random number.
    #[allow(clippy::too_many_arguments)]
    pub fn drive(
        &mut self,
        me: &Bike,
        others: &[&Bike],
        cars: &[Car],
        prey: Option<&Bike>,
        track: &Track,
        dt: f32,
        roll: f32,
    ) -> Controls {
        self.attack_cooldown = (self.attack_cooldown - dt).max(0.0);
        self.grudge = (self.grudge - dt).max(0.0);
        self.rest_t = (self.rest_t - dt).max(0.0);
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
        // Keep right of centre where the oncoming traffic isn't.
        let mut target_lat = self.lane * hw * 0.4 + inside + self.dodge + 1.0;

        // Hunting: ride up alongside the player and hold station there.
        let mut hunt = None;
        if let Some(p) = prey.filter(|p| p.is_solid() && me.is_riding() && p.forward_speed() > 12.0)
        {
            let d = track.delta_s(s, p.track_pos.s);
            let keen = self.grudge > 0.0 || (self.brawler && self.rest_t <= 0.0);
            if keen && (-30.0..45.0).contains(&d) {
                hunt = Some((d, p));
            }
        }
        self.hunting = hunt.is_some();
        if self.hunting {
            self.hunt_t += dt;
            if self.hunt_t > HUNT_SECS && self.grudge <= 0.0 {
                self.hunt_t = 0.0;
                self.rest_t = REST_SECS;
            }
        } else {
            self.hunt_t = (self.hunt_t - dt).max(0.0);
        }
        if let Some((_, p)) = hunt {
            let pl = p.track_pos.lateral;
            let mut side = (me.track_pos.lateral - pl).signum();
            if side == 0.0 {
                side = 1.0;
            }
            if (pl + side * 1.7).abs() > hw - 1.0 {
                side = -side;
            }
            target_lat = pl + side * 1.7;
        }

        if me.track_pos.lateral.abs() > hw {
            target_lat = 0.0;
        }
        target_lat = target_lat.clamp(-hw + 1.2, hw - 1.2);

        // Road traffic: find the nearest car in our path and go round it on
        // the side we picked when we first saw it, or sit behind it.
        let mut follow_car: Option<f32> = None;
        self.avoiding = false;
        let horizon_car = 20.0 + v * 3.0;
        let lat_now = me.track_pos.lateral;
        let mut nearest = f32::MAX;
        let mut soonest = f32::MAX;
        let mut threat = None;
        for (ci, c) in cars.iter().enumerate() {
            let d = track.delta_s(s, c.s);
            let he = c.half_extents();
            if d < -he.y || d > horizon_car {
                continue;
            }
            // Already alongside and clear of it: we're getting past.
            if d < he.y + 1.0 && (c.lateral - lat_now).abs() > he.x + 0.5 {
                continue;
            }
            let closing = v - c.speed * c.dir;
            // Only what we'll reach within a few seconds matters.
            let ttc = if closing > 0.5 {
                (d - he.y - 1.0).max(0.0) / closing
            } else {
                f32::MAX
            };
            if ttc > 3.2 && d > he.y + 3.0 {
                continue;
            }
            // Where we'll be laterally when we get there.
            let then = lat_now + (target_lat - lat_now) * (d / look).clamp(0.0, 1.0);
            let reach = he.x + 1.5;
            let in_path = (c.lateral - then).abs() < reach
                || (c.lateral - lat_now).abs() < reach
                || (c.lateral - target_lat).abs() < reach;
            // The one we'll reach first, not the one nearest now.
            if in_path && ttc.min(d / v.max(5.0)) < soonest {
                soonest = ttc.min(d / v.max(5.0));
                nearest = d;
                threat = Some((ci, c));
            }
        }
        if let Some((ci, c)) = threat {
            let he = c.half_extents();
            let left = c.lateral - he.x - 1.3;
            let right = c.lateral + he.x + 1.3;
            // Open if nothing else will be there when we arrive.
            let t_arrive = nearest / v.max(5.0);
            let clear = |lat: f32| {
                lat.abs() < hw - 0.6
                    && !cars.iter().enumerate().any(|(oi, o)| {
                        let od = track.delta_s(s, o.s) + o.speed * o.dir * t_arrive;
                        oi != ci
                            && (od - nearest).abs() < o.half_extents().y + 10.0
                            && (o.lateral - lat).abs() < o.half_extents().x + 1.1
                    })
            };
            let lat_of = |sd: f32| if sd < 0.0 { left } else { right };
            // Our side of it needs no crossing in front; the far side only if
            // there's time to get across.
            let near_side = if lat_now < c.lateral { -1.0 } else { 1.0 };
            let ttc = (nearest - he.y).max(0.0) / (v - c.speed * c.dir).max(1.0);
            let side = match self.avoid {
                Some((i, sd)) if i == ci && clear(lat_of(sd)) => Some(sd),
                _ => {
                    if clear(lat_of(near_side)) {
                        Some(near_side)
                    } else if ttc > 1.6 && clear(lat_of(-near_side)) {
                        Some(-near_side)
                    } else if c.dir < 0.0 && lat_of(near_side).abs() < hw - 0.6 {
                        // Head-on: squeeze past on our side whatever's there.
                        Some(near_side)
                    } else {
                        None
                    }
                }
            };
            match side {
                Some(sd) => {
                    self.avoid = Some((ci, sd));
                    target_lat = if sd < 0.0 { left } else { right };
                    self.avoiding = true;
                    // Still lined up with it close in: slow while we get round.
                    let overlap = (c.lateral - lat_now).abs() < he.x + 0.7;
                    if overlap && nearest < 6.0 + v * 0.5 {
                        follow_car = Some(c.speed * c.dir + 6.0);
                    }
                }
                None => {
                    self.avoid = None;
                    follow_car = Some(c.speed * c.dir - 2.0);
                }
            }
        } else {
            self.avoid = None;
        }
        self.target_lat = target_lat;
        let max_yaw = (GRIP_ROAD * me.tuning.grip / v.max(1.0)).min(2.4);
        let want_yaw = if self.avoiding || self.hunting {
            // Close quarters: hold an exact lateral on the road. Follow the
            // road's own turn and close the lateral gap at a set rate.
            let f = track.frame_at(s + v * 0.15);
            let lat_vel = me.vel.dot(f.right);
            let err = target_lat - me.track_pos.lateral;
            let want_lat_vel = (err * 2.2).clamp(-8.0, 8.0);
            let offset = ((want_lat_vel + (want_lat_vel - lat_vel) * 0.6) / v.max(6.0))
                .clamp(-0.45, 0.45)
                .asin();
            let yaw_err = wrap_angle(f.heading() + offset - me.heading);
            v * f.curvature + yaw_err * 4.0
        } else {
            let tp = track.point_at(s + look, target_lat);
            let to = Vec2::new(tp.x, tp.z) - me.pos2();
            let alpha = fwd.perp_dot(to.normalize_or_zero()).asin();
            2.0 * alpha.sin() * v.max(6.0) / look
        };
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
        if let Some(fv) = follow_car {
            target_v = target_v.min(fv.max(0.0));
        }
        // Hunting: match the player's speed alongside.
        let mut chase = false;
        if let Some((d, p)) = hunt {
            let pv = p.forward_speed();
            if d < 3.0 {
                target_v = target_v.min((pv + d * 1.2).max(pv - 5.0));
            } else if d > 6.0 {
                chase = true;
            }
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
        // Chasing the player down: burn the boost to close the gap.
        let boost = (self.boost_timer > 0.0 || (chase && me.boost > 0.15)) && throttle > 0.9;

        let (rate, agg) = if self.hunting {
            (0.12, self.aggression.max(0.5))
        } else {
            (0.05, self.aggression)
        };
        let strike = alongside.is_some()
            && self.attack_cooldown <= 0.0
            && roll < agg * rate
            && !ahead_slow
            && !self.avoiding;
        if strike {
            self.attack_cooldown = if self.hunting {
                0.8 + (1.0 - agg) * 1.0
            } else {
                1.4 + (1.0 - agg) * 2.0
            };
        }
        // About a third of strikes are kicks; the angrier the rider, the more.
        let kick = strike && roll < agg * rate * (0.2 + 0.3 * agg);

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
