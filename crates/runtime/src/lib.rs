//! Frame-composition runtime: the layer that finally puts `game` and
//! `render` together.
//!
//! - [`frame`]: [`GameFrame`] advances the race one tick and builds the
//!   camera, road mesh, and sprite batch that draw it
//! - [`sprites`]: procedural placeholder bike frames (no binary art files)
//!   packed through the `assets` crate's atlas packer
//! - [`offscreen`]: the one-frame wgpu render to an offscreen texture, read
//!   back and written as a PNG — the GPU-path proof
//!
//! GPU-path code is exercised by the `frame-proof` binary, not by tests: the
//! GPU-less host cannot run it, and the blueprint's test gate is intentionally
//! unmet here.

#![forbid(unsafe_code)]

pub mod frame;
pub mod offscreen;
pub mod sprites;

pub use frame::{
    advance_game_frame, AtlasMapping, GameFrame, RoadMesh, RoadVertex, BIKE_WORLD_HEIGHT,
    BIKE_WORLD_WIDTH, CAMERA_HEIGHT, CAMERA_TRAIL, FOCAL, ROAD_DRAW_DISTANCE, ROAD_STEP,
    VIEWPORT_HEIGHT, VIEWPORT_WIDTH,
};
pub use offscreen::{render_proof, ProofError, ProofScene, RenderedProof, TARGET_FORMAT};
pub use sprites::{pack_placeholder_atlas, placeholder_frames, FRAME_HEIGHT, FRAME_WIDTH};
