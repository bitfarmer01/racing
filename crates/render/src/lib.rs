//! The 3D renderer.
//!
//! Frame: sun shadow map -> HDR forward pass (terrain, road, props, bikes,
//! water, sky, particles) -> bloom chain -> tonemap/grade -> FXAA to the
//! surface, with a stylised "finish" in the composite (ink outlines from the
//! depth buffer, impact frames, speed lines, split-tone, grain) driven by the
//! design tokens in [`style`]. Everything runs on WebGL2-class hardware (no storage buffers,
//! no compute) so the same code ships to WebGPU and WebGL2.

pub mod camera;
pub mod mesh;
pub mod models;
pub mod renderer;
pub mod style;
pub mod world;

pub use camera::Camera;
pub use renderer::{BikeDraw, FrameInput, Particle, ParticleKind, Renderer, Settings};
