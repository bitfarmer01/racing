//! Gameplay layer for the racing game: bike state machine, fixed-tick race
//! loop, deterministic pursuit AI, bike-vs-bike collisions, and lap scoring.
//!
//! Wires the wave-one crates together: the [`sim`] fixed-tick core (clock,
//! input ring, bike pool, RNG) and the [`world`] road geometry (lookahead
//! queries). Zero external dependencies, 100% safe Rust, and no allocation in
//! any per-tick code path.
//!
//! - [`input`]: player intent, distilled from the sim input ring
//! - [`bike`]: the per-bike control state machine and its tuning constants
//! - [`ai`]: deterministic rival rider logic over world-crate lookahead
//! - [`combat`]: adjacent-rider attacks, stamina, and dismounts (wave 7b)
//! - [`race`]: the race driver that advances one 60 Hz tick per call
//!
//! Determinism: a tick depends only on the pushed input sequence and the
//! seed handed to [`race::RaceConfig`], never on wall-clock time.

#![forbid(unsafe_code)]

pub mod ai;
pub mod bike;
pub mod combat;
pub mod input;
pub mod race;
