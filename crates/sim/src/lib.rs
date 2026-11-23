//! Fixed-tick simulation core for the racing game.
//!
//! Pure simulation: zero external dependencies, 100% safe Rust, and no
//! allocation in any steady-state code path. Every structure here is
//! preallocated; the frame loop never allocates.
//!
//! - [`clock`]: fixed 60 Hz tick clock with a spiral-of-death clamp
//! - [`input`]: input ring buffer, drained exactly once per tick start
//! - [`pool`]: generation-indexed structure-of-arrays entity pool
//! - [`rng`]: deterministic xorshift64* RNG

#![forbid(unsafe_code)]

pub mod clock;
pub mod input;
pub mod pool;
pub mod rng;
