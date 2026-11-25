//! Small deterministic noise helpers shared by terrain and scenery.
//!
//! Value noise with a quintic fade: cheap, stable across platforms (pure
//! integer hashing plus f32 math), and smooth enough for rolling hills.

/// 2D integer hash to [0, 1).
#[inline]
pub fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Smooth value noise in [0, 1).
pub fn value2(x: f32, y: f32, seed: u32) -> f32 {
    let xi = x.floor();
    let yi = y.floor();
    let (fx, fy) = (fade(x - xi), fade(y - yi));
    let (ix, iy) = (xi as i32, yi as i32);
    let a = hash2(ix, iy, seed);
    let b = hash2(ix + 1, iy, seed);
    let c = hash2(ix, iy + 1, seed);
    let d = hash2(ix + 1, iy + 1, seed);
    let ab = a + (b - a) * fx;
    let cd = c + (d - c) * fx;
    ab + (cd - ab) * fy
}

/// Fractal sum of `octaves` layers, normalised to roughly [-1, 1].
pub fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut norm = 0.0;
    let (mut px, mut py) = (x, y);
    for o in 0..octaves {
        sum += (value2(px, py, seed.wrapping_add(o * 131)) * 2.0 - 1.0) * amp;
        norm += amp;
        amp *= 0.5;
        // Rotate a little between octaves to hide the lattice.
        let (rx, ry) = (px * 1.6 - py * 1.2, px * 1.2 + py * 1.6);
        px = rx;
        py = ry;
    }
    sum / norm
}

/// Ridged multifractal in [0, 1]: sharp crests for mountain ranges.
pub fn ridged(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut weight = 1.0;
    let (mut px, mut py) = (x, y);
    for o in 0..octaves {
        let n = value2(px, py, seed.wrapping_add(o * 977)) * 2.0 - 1.0;
        let r = (1.0 - n.abs()).powi(2) * weight;
        weight = (r * 2.0).clamp(0.0, 1.0);
        sum += r * amp;
        amp *= 0.5;
        let (rx, ry) = (px * 1.7 - py * 1.1, px * 1.1 + py * 1.7);
        px = rx;
        py = ry;
    }
    sum.clamp(0.0, 1.0)
}

#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_and_bounded() {
        for i in 0..200 {
            let x = i as f32 * 0.37;
            let y = i as f32 * -0.91;
            let a = value2(x, y, 7);
            assert_eq!(a, value2(x, y, 7));
            assert!((0.0..1.0).contains(&a));
            let f = fbm(x, y, 5, 3);
            assert!((-1.0..=1.0).contains(&f));
            let r = ridged(x, y, 4, 3);
            assert!((0.0..=1.0).contains(&r));
        }
    }
}
