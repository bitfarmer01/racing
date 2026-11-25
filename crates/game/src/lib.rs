//! Gameplay for the racer: 3D circuits on procedural landforms, motorcycle
//! physics, rival AI, bike-to-bike combat and the race loop.
//!
//! Everything here is deterministic and renderer-agnostic; the `render`
//! crate turns a [`race::Race`] into pictures.

#![forbid(unsafe_code)]

pub mod ai;
pub mod bike;
pub mod noise;
pub mod race;
pub mod scenery;
pub mod theme;
pub mod track;
pub mod tracks;
