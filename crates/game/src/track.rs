//! Closed 3D circuits.
//!
//! A circuit is authored as a loop of 2D control points. It is turned into
//! a centripetal Catmull-Rom spline, resampled at an even spacing, and lifted
//! onto the theme's landform: the road's height is the natural ground height
//! along the line, heavily smoothed, so the circuit follows the hills without
//! steep grades. Every sample carries a frame (tangent, right), signed
//! curvature (positive = right-hand turn) and a bank angle.
//!
//! Conventions: y is up; on the ground plane a heading `h` faces
//! `(cos h, sin h)` in (x, z) and its right-hand side is `(-sin h, cos h)`.
//! Increasing `h` turns right.

use glam::{Vec2, Vec3};

use crate::noise::smoothstep;
use crate::theme::Theme;

/// Spacing between centreline samples in metres.
pub const SAMPLE_SPACING: f32 = 2.0;
/// Drivable run-off beyond each road edge before the barrier, in metres.
pub const RUNOFF: f32 = 8.5;
/// Minimum radius of a crest in the road's vertical profile, metres.
pub const CREST_RADIUS: f32 = 300.0;
/// Steepest climb the road is allowed, rise over run.
const MAX_GRADE: f32 = 0.085;
/// Spatial-hash cell for nearest-sample queries.
const CELL: f32 = 24.0;
/// The longest straight on each circuit opens out by this much each side.
pub const WIDE_EXTRA: f32 = 5.0;
/// Length of the ramp in and out of the wide section, metres.
const WIDE_TAPER: f32 = 45.0;
/// Kicker ramps on the wide straight's shoulders.
pub const RAMP_LENGTH: f32 = 11.0;
pub const RAMP_HEIGHT: f32 = 1.3;
pub const RAMP_HALF_WIDTH: f32 = 1.8;

/// A kicker on the road: rides up a curved face and drops off a lip.
#[derive(Debug, Clone, Copy)]
pub struct Ramp {
    /// Lap distance of the foot of the ramp; the lip is `RAMP_LENGTH` on.
    pub s: f32,
    /// Lateral of its centre.
    pub lateral: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TrackDef {
    pub slug: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub points: &'static [[f32; 2]],
    pub half_width: f32,
    pub laps: u32,
    pub theme: Theme,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TrackSample {
    /// Centreline point on the road surface.
    pub pos: Vec3,
    /// Horizontal unit tangent in (x, z).
    pub fwd: Vec2,
    /// Horizontal unit right vector in (x, z).
    pub right: Vec2,
    /// Distance from the start line along the centreline.
    pub dist: f32,
    /// Signed curvature, 1/m; positive turns right.
    pub curvature: f32,
    /// Bank angle in radians; positive raises the left edge.
    pub bank: f32,
    /// Half the road's width here (wider on the circuit's main straight).
    pub half_width: f32,
}

impl TrackSample {
    /// Road surface height at `lateral` metres right of the centreline.
    pub fn surface_y(&self, lateral: f32) -> f32 {
        self.pos.y - lateral * self.bank.tan()
    }

    pub fn heading(&self) -> f32 {
        self.fwd.y.atan2(self.fwd.x)
    }
}

/// Where a point sits relative to the circuit.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrackPos {
    /// Index of the sample at or just behind the point.
    pub index: usize,
    /// Distance along the lap, in [0, length).
    pub s: f32,
    /// Signed distance right of the centreline.
    pub lateral: f32,
}

#[derive(Debug, Clone)]
pub struct Track {
    pub def: TrackDef,
    pub samples: Vec<TrackSample>,
    pub length: f32,
    pub spacing: f32,
    /// Narrowest half width (most of the lap); see [`Track::half_width_at`].
    pub half_width: f32,
    /// Lap distance where the wide straight starts and its full-width
    /// length, if the circuit has one.
    pub wide: Option<(f32, f32)>,
    pub ramps: Vec<Ramp>,
    grid_origin: Vec2,
    grid_dims: (i32, i32),
    grid: Vec<Vec<u32>>,
}

impl Track {
    pub fn build(def: TrackDef) -> Self {
        let dense = spline_polyline(def.points, 48);
        let (samples2, length) = resample(&dense, SAMPLE_SPACING);
        let n = samples2.len();
        let spacing = length / n as f32;

        // Heights: the landform along the line, smoothed hard, kept above water.
        let theme = def.theme;
        let mut heights: Vec<f32> = samples2
            .iter()
            .map(|p| theme.natural_height(p.x, p.y))
            .collect();
        for _ in 0..4 {
            heights = circular_box_blur(&heights, (45.0 / spacing) as usize);
        }
        let floor = theme.water_level + 4.0;
        for h in heights.iter_mut() {
            *h = h.max(floor);
        }
        for _ in 0..2 {
            heights = circular_box_blur(&heights, (30.0 / spacing) as usize);
        }
        limit_grade(&mut heights, MAX_GRADE * spacing);
        for _ in 0..3 {
            heights = circular_box_blur(&heights, (16.0 / spacing) as usize);
        }
        // Round off crests so bikes stay planted at top speed.
        limit_crests(&mut heights, spacing * spacing / (2.0 * CREST_RADIUS));
        heights = circular_box_blur(&heights, (6.0 / spacing) as usize);

        let mut samples: Vec<TrackSample> = (0..n)
            .map(|i| {
                let prev = samples2[(i + n - 1) % n];
                let next = samples2[(i + 1) % n];
                let fwd = (next - prev).normalize_or_zero();
                TrackSample {
                    pos: Vec3::new(samples2[i].x, heights[i], samples2[i].y),
                    fwd,
                    right: Vec2::new(-fwd.y, fwd.x),
                    dist: i as f32 * spacing,
                    curvature: 0.0,
                    bank: 0.0,
                    half_width: def.half_width,
                }
            })
            .collect();

        let raw_curv: Vec<f32> = (0..n)
            .map(|i| {
                let a = samples[(i + n - 2) % n].fwd;
                let b = samples[(i + 2) % n].fwd;
                let ang = (a.x * b.y - a.y * b.x).atan2(a.dot(b));
                ang / (4.0 * spacing)
            })
            .collect();
        let curv = circular_box_blur(&circular_box_blur(&raw_curv, 4), 4);
        let bank: Vec<f32> = circular_box_blur(
            &curv
                .iter()
                .map(|k| (k * 7.0).clamp(-0.10, 0.10))
                .collect::<Vec<_>>(),
            10,
        );
        for (i, s) in samples.iter_mut().enumerate() {
            s.curvature = curv[i];
            s.bank = bank[i];
        }
        let wide = widen_longest_straight(&mut samples, spacing, length);
        // Two kickers on the wide straight's shoulders, opposite sides,
        // kept off the start line and its grid.
        let mut ramps: Vec<Ramp> = Vec::new();
        if let Some((start, len)) = wide {
            let lat = def.half_width + WIDE_EXTRA * 0.5;
            for f in [0.3, 0.62, 0.15, 0.45, 0.8] {
                let at = (start + len * f).rem_euclid(length);
                let from_line = at.min(length - at);
                let spaced = ramps.iter().all(|r| {
                    let d = (r.s - at).abs();
                    d.min(length - d) > 90.0
                });
                if ramps.len() < 2 && from_line > 140.0 && spaced {
                    let side = if ramps.is_empty() { 1.0 } else { -1.0 };
                    ramps.push(Ramp {
                        s: at,
                        lateral: side * lat,
                    });
                }
            }
        }

        let mut track = Self {
            def,
            samples,
            length,
            spacing,
            half_width: def.half_width,
            wide,
            ramps,
            grid_origin: Vec2::ZERO,
            grid_dims: (0, 0),
            grid: Vec::new(),
        };
        track.build_grid();
        track
    }

    fn build_grid(&mut self) {
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for s in &self.samples {
            let p = Vec2::new(s.pos.x, s.pos.z);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        lo -= Vec2::splat(CELL * 2.0);
        hi += Vec2::splat(CELL * 2.0);
        let dims = (
            ((hi.x - lo.x) / CELL).ceil() as i32,
            ((hi.y - lo.y) / CELL).ceil() as i32,
        );
        let mut grid = vec![Vec::new(); (dims.0 * dims.1) as usize];
        for (i, s) in self.samples.iter().enumerate() {
            let cx = ((s.pos.x - lo.x) / CELL) as i32;
            let cz = ((s.pos.z - lo.y) / CELL) as i32;
            grid[(cz * dims.0 + cx) as usize].push(i as u32);
        }
        self.grid_origin = lo;
        self.grid_dims = dims;
        self.grid = grid;
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn sample(&self, i: usize) -> &TrackSample {
        &self.samples[i % self.samples.len()]
    }

    pub fn wrap_s(&self, s: f32) -> f32 {
        s.rem_euclid(self.length)
    }

    /// Interpolated frame at lap distance `s` (any value; wraps).
    pub fn frame_at(&self, s: f32) -> TrackSample {
        let s = self.wrap_s(s);
        let f = s / self.spacing;
        let i = f.floor() as usize % self.samples.len();
        let t = f - f.floor();
        let a = &self.samples[i];
        let b = &self.samples[(i + 1) % self.samples.len()];
        let fwd = a.fwd.lerp(b.fwd, t).normalize_or_zero();
        TrackSample {
            pos: a.pos.lerp(b.pos, t),
            fwd,
            right: Vec2::new(-fwd.y, fwd.x),
            dist: s,
            curvature: a.curvature + (b.curvature - a.curvature) * t,
            bank: a.bank + (b.bank - a.bank) * t,
            half_width: a.half_width + (b.half_width - a.half_width) * t,
        }
    }

    /// Half the road's width at lap distance `s`.
    pub fn half_width_at(&self, s: f32) -> f32 {
        self.frame_at(s).half_width
    }

    /// Widest half width anywhere on the lap.
    pub fn max_half_width(&self) -> f32 {
        self.samples
            .iter()
            .map(|s| s.half_width)
            .fold(self.half_width, f32::max)
    }

    /// Height a ramp adds at (`s`, `lateral`) and the slope of its face
    /// (rise over run along the road), or `None` off every ramp.
    pub fn ramp_at(&self, s: f32, lateral: f32) -> Option<(f32, f32)> {
        self.ramps.iter().find_map(|r| {
            let x = self.delta_s(r.s, s);
            if !(0.0..RAMP_LENGTH).contains(&x) {
                return None;
            }
            // Square sides with a short chamfer so a glancing pass rides up.
            let side = (lateral - r.lateral).abs() - RAMP_HALF_WIDTH;
            if side > 0.35 {
                return None;
            }
            let edge = 1.0 - (side / 0.35).clamp(0.0, 1.0);
            let u = x / RAMP_LENGTH;
            let h = RAMP_HEIGHT * u * u * edge;
            let slope = 2.0 * RAMP_HEIGHT * u / RAMP_LENGTH * edge;
            Some((h, slope))
        })
    }

    /// World position of a point `lateral` metres right of the line at `s`.
    pub fn point_at(&self, s: f32, lateral: f32) -> Vec3 {
        let f = self.frame_at(s);
        Vec3::new(
            f.pos.x + f.right.x * lateral,
            f.surface_y(lateral),
            f.pos.z + f.right.y * lateral,
        )
    }

    /// Locates `p` near the sample `hint` (searching ±window samples).
    pub fn locate_near(&self, p: Vec2, hint: usize, window: usize) -> TrackPos {
        let n = self.samples.len();
        let mut best = hint % n;
        let mut best_d = f32::MAX;
        for k in 0..=(2 * window) {
            let i = (hint + n * 4 + k - window) % n;
            let s = &self.samples[i];
            let d = Vec2::new(s.pos.x, s.pos.z).distance_squared(p);
            if d < best_d {
                best_d = d;
                best = i;
            }
        }
        self.refine(p, best)
    }

    /// Locates `p` against the whole circuit (no hint).
    pub fn locate(&self, p: Vec2) -> TrackPos {
        let best = self.nearest_index(p, f32::MAX).unwrap_or(0);
        self.refine(p, best)
    }

    /// Nearest sample within `max_dist` using the spatial hash, or `None`.
    pub fn nearest_index(&self, p: Vec2, max_dist: f32) -> Option<usize> {
        let rel = (p - self.grid_origin) / CELL;
        let (cx, cz) = (rel.x.floor() as i32, rel.y.floor() as i32);
        let full = self.grid_dims.0.max(self.grid_dims.1) + 1;
        let reach = ((max_dist / CELL).ceil().min(full as f32) as i32 + 1).min(full);
        let mut best = None;
        let mut best_d = max_dist * max_dist;
        // Expanding rings: stop once a ring can no longer beat the best.
        for r in 0..=reach {
            if best.is_some() && ((r - 1) as f32 * CELL).powi(2) > best_d {
                break;
            }
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let (gx, gz) = (cx + dx, cz + dz);
                    if gx < 0 || gz < 0 || gx >= self.grid_dims.0 || gz >= self.grid_dims.1 {
                        continue;
                    }
                    for &i in &self.grid[(gz * self.grid_dims.0 + gx) as usize] {
                        let s = &self.samples[i as usize];
                        let d = Vec2::new(s.pos.x, s.pos.z).distance_squared(p);
                        if d < best_d {
                            best_d = d;
                            best = Some(i as usize);
                        }
                    }
                }
            }
        }
        best
    }

    fn refine(&self, p: Vec2, best: usize) -> TrackPos {
        let n = self.samples.len();
        let s0 = &self.samples[best];
        let base = Vec2::new(s0.pos.x, s0.pos.z);
        let along = (p - base).dot(s0.fwd);
        // Step back one sample when the point lies behind the nearest one.
        let (i, a) = if along < 0.0 {
            let j = (best + n - 1) % n;
            (j, &self.samples[j])
        } else {
            (best, s0)
        };
        let b = &self.samples[(i + 1) % n];
        let pa = Vec2::new(a.pos.x, a.pos.z);
        let pb = Vec2::new(b.pos.x, b.pos.z);
        let seg = pb - pa;
        let t = ((p - pa).dot(seg) / seg.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let on = pa + seg * t;
        let right = a.right.lerp(b.right, t).normalize_or_zero();
        TrackPos {
            index: i,
            s: self.wrap_s(a.dist + t * self.spacing),
            lateral: (p - on).dot(right),
        }
    }

    /// Ground height at (x, z): the road surface on and beside the road,
    /// blending into the theme's landform further out.
    pub fn ground_height(&self, x: f32, z: f32) -> f32 {
        let p = Vec2::new(x, z);
        let natural = self.def.theme.natural_height(x, z);
        match self.nearest_index(p, 70.0) {
            Some(i) => {
                let tp = self.refine(p, i);
                self.blend_ground(natural, &tp)
            }
            None => natural,
        }
    }

    /// Ground height for a point already located on the circuit.
    pub fn ground_at(&self, x: f32, z: f32, tp: &TrackPos) -> f32 {
        let natural = self.def.theme.natural_height(x, z);
        self.blend_ground(natural, tp)
    }

    fn blend_ground(&self, natural: f32, tp: &TrackPos) -> f32 {
        let f = self.frame_at(tp.s);
        let hw = f.half_width;
        let edge = tp.lateral.abs() - hw;
        let road_y = f.surface_y(tp.lateral.clamp(-hw, hw));
        if edge <= 0.0 {
            let lift = self.ramp_at(tp.s, tp.lateral).map_or(0.0, |r| r.0);
            return f.surface_y(tp.lateral) + lift;
        }
        // A shallow shoulder falls away a little, then the land takes over.
        let shoulder = road_y - 0.08 - (edge.min(RUNOFF + 2.0) * 0.025);
        let t = smoothstep(RUNOFF + 1.0, RUNOFF + 46.0, edge);
        shoulder + (natural - shoulder) * t
    }

    /// Ground height under a terrain vertex. The mesh sits a touch below the
    /// road so the road ribbon never z-fights it.
    pub fn terrain_vertex_height(&self, x: f32, z: f32) -> f32 {
        let p = Vec2::new(x, z);
        let natural = self.def.theme.natural_height(x, z);
        match self.nearest_index(p, 70.0) {
            Some(i) => {
                let tp = self.refine(p, i);
                let h = self.blend_ground(natural, &tp);
                let edge = tp.lateral.abs() - self.half_width_at(tp.s);
                if edge < 1.5 {
                    h - 0.45
                } else {
                    h - 0.12 * (1.0 - smoothstep(1.5, 6.0, edge))
                }
            }
            None => natural,
        }
    }

    /// Lap distance `ahead` metres past `s`, signed difference helper.
    pub fn delta_s(&self, from: f32, to: f32) -> f32 {
        let mut d = to - from;
        let half = self.length * 0.5;
        if d > half {
            d -= self.length;
        } else if d < -half {
            d += self.length;
        }
        d
    }

    /// Bounds of the centreline in (x, z).
    pub fn bounds(&self) -> (Vec2, Vec2) {
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for s in &self.samples {
            lo = lo.min(Vec2::new(s.pos.x, s.pos.z));
            hi = hi.max(Vec2::new(s.pos.x, s.pos.z));
        }
        (lo, hi)
    }
}

/// Opens the longest near-straight run out by [`WIDE_EXTRA`] a side,
/// easing in and out, and returns where the full width starts and how long
/// it lasts.
fn widen_longest_straight(
    samples: &mut [TrackSample],
    spacing: f32,
    length: f32,
) -> Option<(f32, f32)> {
    let n = samples.len();
    let straight = |i: usize| samples[i % n].curvature.abs() < 0.0028;
    let (mut best, mut best_start, mut run, mut run_start) = (0usize, 0usize, 0usize, 0usize);
    for i in 0..2 * n {
        if straight(i) {
            if run == 0 {
                run_start = i;
            }
            run = (run + 1).min(n);
            if run > best {
                best = run;
                best_start = run_start % n;
            }
        } else {
            run = 0;
        }
    }
    // Keep clear of the corners at each end.
    let inset = (25.0 / spacing) as usize;
    let taper = (WIDE_TAPER / spacing) as usize;
    if best < 2 * (inset + taper) + (40.0 / spacing) as usize {
        return None;
    }
    let first = best_start + inset;
    let last = best_start + best - inset;
    for k in first..last {
        let into = (k - first).min(last - 1 - k) as f32 / taper as f32;
        let t = smoothstep(0.0, 1.0, into.min(1.0));
        samples[k % n].half_width += WIDE_EXTRA * t;
    }
    let start = ((first + taper) % n) as f32 * spacing;
    Some((
        start.rem_euclid(length),
        (last - first - 2 * taper) as f32 * spacing,
    ))
}

/// Dense closed polyline through `points` (centripetal Catmull-Rom).
fn spline_polyline(points: &[[f32; 2]], steps: usize) -> Vec<Vec2> {
    let n = points.len();
    let p = |i: usize| Vec2::from(points[i % n]);
    let mut out = Vec::with_capacity(n * steps);
    for i in 0..n {
        let (p0, p1, p2, p3) = (p(i + n - 1), p(i), p(i + 1), p(i + 2));
        let knot = |a: Vec2, b: Vec2| a.distance(b).max(1e-3).sqrt();
        let t0 = 0.0;
        let t1 = t0 + knot(p0, p1);
        let t2 = t1 + knot(p1, p2);
        let t3 = t2 + knot(p2, p3);
        for k in 0..steps {
            let t = t1 + (t2 - t1) * k as f32 / steps as f32;
            let a1 = p0 * ((t1 - t) / (t1 - t0)) + p1 * ((t - t0) / (t1 - t0));
            let a2 = p1 * ((t2 - t) / (t2 - t1)) + p2 * ((t - t1) / (t2 - t1));
            let a3 = p2 * ((t3 - t) / (t3 - t2)) + p3 * ((t - t2) / (t3 - t2));
            let b1 = a1 * ((t2 - t) / (t2 - t0)) + a2 * ((t - t0) / (t2 - t0));
            let b2 = a2 * ((t3 - t) / (t3 - t1)) + a3 * ((t - t1) / (t3 - t1));
            out.push(b1 * ((t2 - t) / (t2 - t1)) + b2 * ((t - t1) / (t2 - t1)));
        }
    }
    out
}

/// Even resampling of a closed polyline. Returns the samples and the length.
fn resample(dense: &[Vec2], spacing: f32) -> (Vec<Vec2>, f32) {
    let n = dense.len();
    let mut cum = Vec::with_capacity(n + 1);
    cum.push(0.0f32);
    for i in 0..n {
        let d = dense[i].distance(dense[(i + 1) % n]);
        cum.push(cum[i] + d);
    }
    let length = cum[n];
    let count = (length / spacing).round().max(8.0) as usize;
    let step = length / count as f32;
    let mut out = Vec::with_capacity(count);
    let mut j = 0;
    for k in 0..count {
        let target = k as f32 * step;
        while cum[j + 1] < target {
            j += 1;
        }
        let seg = (cum[j + 1] - cum[j]).max(1e-6);
        let t = (target - cum[j]) / seg;
        out.push(dense[j].lerp(dense[(j + 1) % n], t));
    }
    (out, length)
}

/// Clamps the climb between neighbours on a closed loop.
/// Lowers samples that stand above their neighbours' mean by more than
/// `max_bump`, i.e. caps the convex vertical curvature.
fn limit_crests(h: &mut [f32], max_bump: f32) {
    let n = h.len();
    for _ in 0..600 {
        let mut changed = false;
        for i in 0..n {
            let avg = 0.5 * (h[(i + n - 1) % n] + h[(i + 1) % n]);
            if h[i] > avg + max_bump {
                h[i] = avg + max_bump;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn limit_grade(h: &mut [f32], max_step: f32) {
    let n = h.len();
    for _ in 0..6 {
        for i in 0..2 * n {
            let (a, b) = (i % n, (i + 1) % n);
            h[b] = h[b].clamp(h[a] - max_step, h[a] + max_step);
        }
        for i in (0..2 * n).rev() {
            let (a, b) = ((i + 1) % n, i % n);
            h[b] = h[b].clamp(h[a] - max_step, h[a] + max_step);
        }
    }
}

fn circular_box_blur(v: &[f32], radius: usize) -> Vec<f32> {
    let n = v.len();
    if radius == 0 || n == 0 {
        return v.to_vec();
    }
    let w = (2 * radius + 1) as f32;
    let mut sum: f32 = (0..=2 * radius).map(|k| v[(k + n * 2 - radius) % n]).sum();
    let mut out = vec![0.0; n];
    for i in 0..n {
        out[i] = sum / w;
        sum += v[(i + radius + 1) % n] - v[(i + n * 2 - radius) % n];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::catalog;

    #[test]
    fn circuits_close_and_resample_evenly() {
        for def in catalog() {
            let t = Track::build(*def);
            assert!(t.length > 1500.0, "{} too short: {}", def.name, t.length);
            assert!((t.spacing - SAMPLE_SPACING).abs() < 0.1);
            // Consecutive samples are evenly spaced all the way round,
            // including the seam back to the start.
            for i in 0..t.len() {
                let a = t.sample(i).pos;
                let b = t.sample(i + 1).pos;
                let d = Vec2::new(a.x - b.x, a.z - b.z).length();
                assert!((d - t.spacing).abs() < 0.25, "{} gap {d} at {i}", def.name);
            }
            // Total turning of a simple closed loop is ±2π.
            let turn: f32 = t.samples.iter().map(|s| s.curvature * t.spacing).sum();
            assert!(
                (turn.abs() - std::f32::consts::TAU).abs() < 0.3,
                "{} turns {turn}",
                def.name
            );
        }
    }

    #[test]
    fn road_stays_above_water_with_gentle_grades() {
        for def in catalog() {
            let t = Track::build(*def);
            for i in 0..t.len() {
                let a = t.sample(i).pos;
                let b = t.sample(i + 1).pos;
                assert!(a.y >= def.theme.water_level + 3.9);
                let grade = (b.y - a.y).abs() / t.spacing;
                assert!(grade < 0.14, "{} grade {grade} at {i}", def.name);
            }
        }
    }

    #[test]
    fn locate_round_trips_points_on_the_road() {
        let t = Track::build(catalog()[0]);
        for k in 0..40 {
            let s = k as f32 * t.length / 40.0 + 0.7;
            for lat in [-5.0, 0.0, 3.5] {
                let p = t.point_at(s, lat);
                let tp = t.locate(Vec2::new(p.x, p.z));
                assert!(t.delta_s(s, tp.s).abs() < 0.3, "s {s} vs {}", tp.s);
                assert!(
                    (tp.lateral - lat).abs() < 0.3,
                    "lat {lat} vs {}",
                    tp.lateral
                );
                let near = t.locate_near(Vec2::new(p.x, p.z), tp.index + 3, 12);
                assert!(t.delta_s(s, near.s).abs() < 0.3);
                let g = t.ground_at(p.x, p.z, &tp);
                assert!((g - p.y).abs() < 0.05);
            }
        }
    }

    #[test]
    fn terrain_never_pokes_through_the_road() {
        for def in catalog() {
            let t = Track::build(*def);
            for k in 0..300 {
                let s = k as f32 * t.length / 300.0;
                for lat in [-t.half_width, 0.0, t.half_width] {
                    let p = t.point_at(s, lat);
                    assert!(t.terrain_vertex_height(p.x, p.z) < p.y - 0.1);
                }
            }
        }
    }
}
