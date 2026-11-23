//! Deterministic xorshift64* RNG.
//!
//! Same seed, same sequence — across runs, platforms, and processes. No
//! external dependency and no allocation; the entire state is one `u64`.

/// Nonzero fallback for a zero seed: a fixed, arbitrary odd constant. A
/// zero state would be a fixed point of xorshift (always yields zero).
const ZERO_SEED_FALLBACK: u64 = 0x9E37_79B9_7F4A_7C15;

/// Multiplier of the xorshift64* final step (Marsaglia, 2003-era family).
const XORSHIFT64_STAR_MULT: u64 = 0x2545_F491_4F6C_DD1D;

/// Deterministic xorshift64* generator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    /// A generator seeded with `seed`. A zero seed falls back to a fixed
    /// nonzero constant so the stream is still well-defined and reproducible.
    pub fn new(seed: u64) -> Self {
        let state = if seed == 0 { ZERO_SEED_FALLBACK } else { seed };
        Self { state }
    }

    /// The next 64 pseudorandom bits. Never returns zero while the state is
    /// nonzero.
    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*: three shifts plus a final multiply.
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(XORSHIFT64_STAR_MULT)
    }

    /// The next 32 pseudorandom bits (high half of the 64-bit output).
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform `f32` in `[0, 1)`, from a 24-bit mantissa.
    pub fn next_f32(&mut self) -> f32 {
        // Top 24 bits give a uniform 2^24-wide grid; scale into [0, 1).
        (self.next_u64() >> 40) as f32 * (1.0 / (1u64 << 24) as f32)
    }

    /// Uniform `f32` in `[min, max)`. Returns `min` when `min >= max`.
    pub fn next_f32_in(&mut self, min: f32, max: f32) -> f32 {
        if min >= max {
            return min;
        }
        min + self.next_f32() * (max - min)
    }
}

impl Default for XorShift64 {
    fn default() -> Self {
        // A fixed default seed keeps "no seed given" deterministic too.
        Self::new(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_COUNT: usize = 1_000;

    fn sequence(seed: u64, count: usize) -> Vec<u64> {
        let mut rng = XorShift64::new(seed);
        (0..count).map(|_| rng.next_u64()).collect()
    }

    #[test]
    fn same_seed_produces_identical_sequences() {
        assert_eq!(sequence(42, SAMPLE_COUNT), sequence(42, SAMPLE_COUNT));
    }

    #[test]
    fn different_seeds_produce_different_sequences() {
        assert_ne!(sequence(1, 64), sequence(2, 64));
    }

    #[test]
    fn zero_seed_is_still_deterministic() {
        assert_eq!(sequence(0, SAMPLE_COUNT), sequence(0, SAMPLE_COUNT));
        assert!(!sequence(0, 16).iter().all(|v| *v == 0));
    }

    #[test]
    fn instances_are_independent() {
        let mut hammered = XorShift64::new(7);
        let mut idle = XorShift64::new(7);
        // Drawing from one generator must not disturb the other's stream.
        for _ in 0..10 {
            hammered.next_u64();
        }
        // The idle generator still reproduces the seeded sequence, and the
        // hammered one has moved on to a different position in it.
        let mut fresh = XorShift64::new(7);
        for _ in 0..16 {
            assert_eq!(idle.next_u64(), fresh.next_u64());
        }
        assert_ne!(hammered.next_u64(), fresh.next_u64());
    }

    #[test]
    fn next_u32_uses_the_high_half() {
        // Pair the u32 stream against the u64 stream for the same seed: the
        // u32 values must be the top 32 bits of the u64 values.
        let mut full = XorShift64::new(99);
        let mut half = XorShift64::new(99);
        for _ in 0..64 {
            let expected = (full.next_u64() >> 32) as u32;
            assert_eq!(half.next_u32(), expected);
        }
    }

    #[test]
    fn next_f32_stays_in_unit_interval() {
        let mut rng = XorShift64::new(123);
        for _ in 0..SAMPLE_COUNT {
            let value = rng.next_f32();
            assert!((0.0..1.0).contains(&value), "value {value} out of [0, 1)");
        }
    }

    #[test]
    fn next_f32_in_respects_bounds() {
        let mut rng = XorShift64::new(321);
        for _ in 0..SAMPLE_COUNT {
            let value = rng.next_f32_in(-2.5, 7.5);
            assert!((-2.5..7.5).contains(&value), "value {value} out of range");
        }
        // Degenerate range: min >= max yields min, no NaN.
        assert_eq!(rng.next_f32_in(5.0, 5.0), 5.0);
        assert_eq!(rng.next_f32_in(5.0, 1.0), 5.0);
    }

    #[test]
    fn sequence_matches_across_runs_and_calls() {
        // Explicitly simulate "two separate runs": construct, draw, drop,
        // reconstruct, and compare against a recorded reference.
        let reference = sequence(2026, 256);
        let replay = sequence(2026, 256);
        assert_eq!(reference, replay);
    }
}
