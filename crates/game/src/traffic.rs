//! Road traffic: cars, vans and trucks running both ways on the circuit.
//!
//! The road is two lanes each way: traffic going the race direction keeps
//! right of the centreline, oncoming traffic keeps left. Cars hold their
//! lane, follow the car in front, change lanes now and then, and stop dead
//! for a while after something hits them. They never react to bikes: it is
//! the rider's job to get through.

use glam::{Vec2, Vec3};
use sim::rng::XorShift64;

use crate::track::Track;

/// Lane centres, metres right of the centreline (race direction).
pub const LANES_WITH: [f32; 2] = [1.7, 4.6];
pub const LANES_ONCOMING: [f32; 2] = [-1.7, -4.6];
/// No traffic is placed this close to the start line at the green, so the
/// grid gets away clean.
const GRID_CLEAR_BEHIND: f32 = 120.0;
const GRID_CLEAR_AHEAD: f32 = 320.0;
/// Seconds a car sits stopped after a crash.
pub const STOP_SECS: f32 = 2.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarKind {
    Hatch,
    Sedan,
    Van,
    Truck,
}

impl CarKind {
    pub const ALL: [CarKind; 4] = [CarKind::Hatch, CarKind::Sedan, CarKind::Van, CarKind::Truck];

    /// Half width and half length, metres.
    pub fn half_extents(self) -> Vec2 {
        match self {
            CarKind::Hatch => Vec2::new(0.85, 1.95),
            CarKind::Sedan => Vec2::new(0.9, 2.35),
            CarKind::Van => Vec2::new(1.0, 2.6),
            CarKind::Truck => Vec2::new(1.2, 4.2),
        }
    }

    /// Typical cruising speed, m/s.
    pub fn cruise(self) -> f32 {
        match self {
            CarKind::Hatch => 24.0,
            CarKind::Sedan => 27.0,
            CarKind::Van => 22.0,
            CarKind::Truck => 18.0,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone)]
pub struct Car {
    pub kind: CarKind,
    /// Lap distance of the car's centre.
    pub s: f32,
    pub lateral: f32,
    /// Lateral it is steering towards (its lane).
    pub lane: f32,
    /// +1 runs with the race, -1 runs against it.
    pub dir: f32,
    pub speed: f32,
    pub cruise: f32,
    /// Seconds left stopped after a crash.
    pub stopped: f32,
    /// World pose, refreshed every step.
    pub pos: Vec3,
    pub heading: f32,
    pub pitch: f32,
    pub roll: f32,
    /// Paint swatch index.
    pub paint: u8,
    lane_timer: f32,
    /// Seconds since the player last scored a near miss on this car.
    pub near_miss_cd: f32,
    /// Which side of the car (along the track) the player was last tick.
    pub player_side: f32,
}

impl Car {
    pub fn fwd(&self) -> Vec2 {
        Vec2::new(self.heading.cos(), self.heading.sin())
    }

    pub fn right(&self) -> Vec2 {
        Vec2::new(-self.heading.sin(), self.heading.cos())
    }

    pub fn pos2(&self) -> Vec2 {
        Vec2::new(self.pos.x, self.pos.z)
    }

    pub fn vel(&self) -> Vec2 {
        self.fwd() * self.speed
    }

    pub fn half_extents(&self) -> Vec2 {
        self.kind.half_extents()
    }

    fn place(&mut self, track: &Track) {
        let f = track.frame_at(self.s);
        let hl = self.kind.half_extents().y;
        let p = track.point_at(self.s, self.lateral);
        let front = track.point_at(self.s + hl * self.dir, self.lateral);
        let back = track.point_at(self.s - hl * self.dir, self.lateral);
        self.pos = p;
        let mut h = f.heading();
        if self.dir < 0.0 {
            h += std::f32::consts::PI;
        }
        // Lane changes angle the car a touch.
        let drift = (self.lane - self.lateral).clamp(-1.0, 1.0) * 0.08 * self.dir;
        self.heading = h + drift;
        self.pitch = ((front.y - back.y) / (2.0 * hl)).atan();
        self.roll = f.bank * self.dir;
    }
}

#[derive(Debug, Clone, Default)]
pub struct Traffic {
    pub cars: Vec<Car>,
}

impl Traffic {
    /// About one vehicle per 170 m of circuit, scaled by `density`.
    pub fn spawn(track: &Track, density: f32, rng: &mut XorShift64) -> Self {
        let count = ((track.length / 170.0 * density).round() as usize).clamp(6, 30);
        let mut cars: Vec<Car> = Vec::with_capacity(count);
        let usable = track.length - GRID_CLEAR_BEHIND - GRID_CLEAR_AHEAD;
        // Spread evenly round the lap (one per slot, jittered), so there is
        // always something coming.
        let slot = usable / count as f32;
        let mut tries = 0;
        while cars.len() < count && tries < count * 20 {
            tries += 1;
            let k = cars.len() as f32;
            let s = GRID_CLEAR_AHEAD + (k + rng.next_f32_in(0.1, 0.9)) * slot;
            let oncoming = rng.next_f32() < 0.45;
            let lanes = if oncoming { LANES_ONCOMING } else { LANES_WITH };
            let lane = lanes[(rng.next_f32() * 2.0) as usize % 2];
            let roll = rng.next_f32();
            let kind = if roll < 0.36 {
                CarKind::Hatch
            } else if roll < 0.68 {
                CarKind::Sedan
            } else if roll < 0.86 {
                CarKind::Van
            } else {
                CarKind::Truck
            };
            // Keep spawns apart in the same lane.
            let clash = cars
                .iter()
                .any(|c| (c.lane - lane).abs() < 0.5 && track.delta_s(c.s, s).abs() < 30.0);
            if clash {
                continue;
            }
            let cruise = kind.cruise() * rng.next_f32_in(0.85, 1.15);
            let mut car = Car {
                kind,
                s,
                lateral: lane,
                lane,
                dir: if oncoming { -1.0 } else { 1.0 },
                speed: cruise,
                cruise,
                stopped: 0.0,
                pos: Vec3::ZERO,
                heading: 0.0,
                pitch: 0.0,
                roll: 0.0,
                paint: (rng.next_f32() * 8.0) as u8 % 8,
                lane_timer: rng.next_f32_in(4.0, 14.0),
                near_miss_cd: 0.0,
                player_side: 0.0,
            };
            car.place(track);
            cars.push(car);
        }
        Self { cars }
    }

    pub fn step(&mut self, track: &Track, dt: f32, rng: &mut XorShift64) {
        let n = self.cars.len();
        for i in 0..n {
            // Follow the nearest car ahead in the same lane.
            let (s, lane, dir) = {
                let c = &self.cars[i];
                (c.s, c.lane, c.dir)
            };
            let mut gap = f32::MAX;
            let mut lead_speed = 0.0;
            let mut other_lane_clear = true;
            let other_lane = flip_lane(lane);
            for (j, o) in self.cars.iter().enumerate() {
                if j == i || o.dir != dir {
                    continue;
                }
                let d = track.delta_s(s, o.s) * dir;
                let len = o.kind.half_extents().y + self.cars[i].kind.half_extents().y;
                if (o.lateral - lane).abs() < 1.4 && d > 0.0 && d - len < gap {
                    gap = d - len;
                    lead_speed = o.speed;
                }
                if (o.lateral - other_lane).abs() < 1.6 && d.abs() < 22.0 {
                    other_lane_clear = false;
                }
            }
            let c = &mut self.cars[i];
            c.near_miss_cd = (c.near_miss_cd - dt).max(0.0);
            c.lane_timer -= dt;
            if c.stopped > 0.0 {
                c.stopped -= dt;
                c.speed = (c.speed - 30.0 * dt).max(0.0);
            } else {
                let mut want = c.cruise;
                if gap < 40.0 {
                    want = want.min(lead_speed + (gap - 8.0) * 0.5).max(0.0);
                }
                // Stuck behind something slow: pull out when the other lane
                // is clear.
                if (gap < 25.0 && lead_speed < c.cruise - 3.0 || c.lane_timer <= 0.0)
                    && other_lane_clear
                {
                    c.lane = other_lane;
                    c.lane_timer = rng.next_f32_in(8.0, 18.0);
                } else if c.lane_timer <= 0.0 {
                    c.lane_timer = rng.next_f32_in(3.0, 6.0);
                }
                let accel = if want > c.speed { 3.5 } else { 9.0 };
                c.speed += (want - c.speed).clamp(-accel * dt, accel * dt);
            }
            let step = (c.lane - c.lateral).clamp(-1.6 * dt, 1.6 * dt);
            c.lateral += step;
            c.s = track.wrap_s(c.s + c.speed * c.dir * dt);
            c.place(track);
        }
    }
}

/// The other lane on the same side of the road.
fn flip_lane(lane: f32) -> f32 {
    let lanes = if lane > 0.0 {
        LANES_WITH
    } else {
        LANES_ONCOMING
    };
    if (lane - lanes[0]).abs() < (lane - lanes[1]).abs() {
        lanes[1]
    } else {
        lanes[0]
    }
}

/// Collision of a circle (centre `p`, radius `r`) with a car's box.
/// Returns the push-out normal (from the car towards the circle) and depth.
pub fn circle_vs_car(car: &Car, p: Vec2, r: f32) -> Option<(Vec2, f32)> {
    let he = car.half_extents();
    let rel = p - car.pos2();
    let (f, rt) = (car.fwd(), car.right());
    let local = Vec2::new(rel.dot(rt), rel.dot(f));
    let clamped = local.clamp(-he, he);
    let d = local - clamped;
    let dist = d.length();
    if dist > r {
        return None;
    }
    let (n_local, depth) = if dist > 1e-4 {
        (d / dist, r - dist)
    } else {
        // Centre inside the box: push out the nearest face.
        let ex = he.x - local.x.abs();
        let ez = he.y - local.y.abs();
        if ex < ez {
            (Vec2::new(local.x.signum(), 0.0), ex + r)
        } else {
            (Vec2::new(0.0, local.y.signum()), ez + r)
        }
    };
    let n = rt * n_local.x + f * n_local.y;
    Some((n, depth))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks;

    #[test]
    fn traffic_runs_both_ways_and_keeps_its_lanes() {
        let track = Track::build(tracks::catalog()[0]);
        let mut rng = XorShift64::new(7);
        let mut t = Traffic::spawn(&track, 1.0, &mut rng);
        assert!(t.cars.len() >= 6);
        assert!(t.cars.iter().any(|c| c.dir > 0.0));
        assert!(t.cars.iter().any(|c| c.dir < 0.0));
        let start: Vec<f32> = t.cars.iter().map(|c| c.s).collect();
        for _ in 0..1200 {
            t.step(&track, 1.0 / 120.0, &mut rng);
        }
        for (c, s0) in t.cars.iter().zip(start) {
            let moved = track.delta_s(s0, c.s) * c.dir;
            assert!(moved > 50.0, "car moved {moved}");
            assert!(c.lateral.abs() < track.half_width, "lateral {}", c.lateral);
            assert_eq!(c.lateral.signum(), c.dir.signum(), "wrong side of the road");
        }
    }

    #[test]
    fn circle_hits_the_side_of_a_car() {
        let track = Track::build(tracks::catalog()[0]);
        let mut rng = XorShift64::new(3);
        let t = Traffic::spawn(&track, 1.0, &mut rng);
        let car = &t.cars[0];
        let beside = car.pos2() + car.right() * (car.half_extents().x + 0.3);
        let (n, depth) = circle_vs_car(car, beside, 0.5).expect("touching");
        assert!(n.dot(car.right()) > 0.9);
        assert!((depth - 0.2).abs() < 1e-3);
        let clear = car.pos2() + car.right() * 3.0;
        assert!(circle_vs_car(car, clear, 0.5).is_none());
    }
}
