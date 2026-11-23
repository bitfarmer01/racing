//! Fixed 60 Hz tick clock with a frame-time accumulator and a
//! spiral-of-death clamp.
//!
//! The simulation steps at exactly [`TICK_RATE_HZ`] regardless of render
//! frame rate. `update` is called once per rendered frame with the measured
//! frame delta; it returns how many ticks to run this frame, never more than
//! [`MAX_CATCHUP_TICKS`]. Excess backlog is dropped, so a stall costs at most
//! one clamped catch-up burst instead of an unbounded spiral.

/// Simulation tick rate, in ticks per second.
pub const TICK_RATE_HZ: u32 = 60;

/// Duration of one tick, in seconds.
pub const TICK_DURATION_SECS: f64 = 1.0 / TICK_RATE_HZ as f64;

/// Maximum catch-up ticks run in a single `update`. Anything the accumulator
/// holds beyond this is dropped: the game slows down under sustained load
/// rather than freezing in a spiral of ever-growing catch-up work.
pub const MAX_CATCHUP_TICKS: usize = 5;

/// Fixed-tick clock. Driven by rendered-frame deltas, consumed by the tick
/// loop.
#[derive(Clone, Copy, Debug)]
pub struct TickClock {
    /// Leftover frame time, in seconds; carries partial progress toward the
    /// next tick.
    accumulator: f64,
    /// Total ticks run since construction. Drives simulation determinism
    /// (replays and the RNG stream both key off this).
    tick_index: u64,
}

impl TickClock {
    /// A clock that has run no ticks yet.
    pub fn new() -> Self {
        Self {
            accumulator: 0.0,
            tick_index: 0,
        }
    }

    /// Feed one rendered frame's delta and report how many ticks to run.
    ///
    /// Negative deltas are treated as zero. The accumulator is clamped to
    /// [`MAX_CATCHUP_TICKS`] ticks of backlog before draining, so the return
    /// value is always in `0..=[MAX_CATCHUP_TICKS]`.
    pub fn update(&mut self, frame_dt_secs: f64) -> usize {
        self.accumulator += frame_dt_secs.max(0.0);
        let max_backlog = f64::from(MAX_CATCHUP_TICKS as u32) * TICK_DURATION_SECS;
        if self.accumulator > max_backlog {
            self.accumulator = max_backlog;
        }

        let mut ticks_run = 0usize;
        while self.accumulator >= TICK_DURATION_SECS {
            self.accumulator -= TICK_DURATION_SECS;
            ticks_run += 1;
        }
        self.tick_index += ticks_run as u64;
        ticks_run
    }

    /// Total ticks run since construction.
    pub fn tick_index(&self) -> u64 {
        self.tick_index
    }

    /// Interpolation alpha in `[0, 1)` between the last two ticks, for the
    /// renderer's between-tick interpolation.
    pub fn alpha(&self) -> f64 {
        self.accumulator / TICK_DURATION_SECS
    }
}

impl Default for TickClock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Close enough to one tick that float accumulation from real frame
    /// timings lands on the intended tick count.
    const EPSILON: f64 = 1e-9;

    #[test]
    fn sub_tick_delta_runs_no_ticks() {
        let mut clock = TickClock::new();
        assert_eq!(clock.update(TICK_DURATION_SECS * 0.5), 0);
        assert_eq!(clock.tick_index(), 0);
    }

    #[test]
    fn one_frame_of_time_runs_one_tick() {
        let mut clock = TickClock::new();
        assert_eq!(clock.update(TICK_DURATION_SECS), 1);
        assert_eq!(clock.tick_index(), 1);
    }

    #[test]
    fn small_frames_accumulate_into_a_tick() {
        let mut clock = TickClock::new();
        // 6 ms frames: no single frame reaches a tick, but three of them do
        // (3 * 6 ms = 18 ms > 16.67 ms).
        let dt = 0.006;
        assert_eq!(clock.update(dt), 0);
        assert_eq!(clock.update(dt), 0);
        assert_eq!(clock.update(dt), 1);
        assert_eq!(clock.tick_index(), 1);
    }

    #[test]
    fn backlog_is_clamped_to_max_catchup_ticks() {
        let mut clock = TickClock::new();
        // A 10 s stall would demand ~600 catch-up ticks; only 5 may run.
        assert_eq!(clock.update(10.0), MAX_CATCHUP_TICKS);
        assert_eq!(clock.tick_index(), MAX_CATCHUP_TICKS as u64);
    }

    #[test]
    fn dropped_backlog_does_not_return_later() {
        let mut clock = TickClock::new();
        clock.update(10.0);
        // The excess backlog was discarded, so the next (zero-length) frame
        // must not run any ticks.
        assert_eq!(clock.update(0.0), 0);
        assert_eq!(clock.tick_index(), MAX_CATCHUP_TICKS as u64);
    }

    #[test]
    fn negative_delta_is_treated_as_zero() {
        let mut clock = TickClock::new();
        assert_eq!(clock.update(-1.0), 0);
        assert_eq!(clock.tick_index(), 0);
    }

    #[test]
    fn tick_index_accumulates_across_frames() {
        let mut clock = TickClock::new();
        let mut total = 0;
        for _ in 0..120 {
            total += clock.update(TICK_DURATION_SECS + EPSILON);
        }
        assert_eq!(clock.tick_index(), 120);
        assert_eq!(total, 120);
    }

    #[test]
    fn alpha_stays_in_zero_to_one() {
        let mut clock = TickClock::new();
        clock.update(TICK_DURATION_SECS * 0.25);
        let a = clock.alpha();
        assert!((0.0..1.0).contains(&a), "alpha {a} out of range");
        assert!((a - 0.25).abs() < EPSILON);
    }

    #[test]
    fn alpha_is_zero_after_a_tick_drains_the_accumulator() {
        let mut clock = TickClock::new();
        clock.update(TICK_DURATION_SECS * 2.0 + EPSILON);
        // The leftover epsilon (1e-9 s) maps to alpha ~6e-8; assert it is a
        // rounding-level fraction of a tick, not exactly zero.
        assert!(clock.alpha() < 1e-6);
    }
}
