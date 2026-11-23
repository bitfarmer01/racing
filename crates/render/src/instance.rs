//! GPU instance layout: one sprite quad per instance.
//!
//! This module is pure data — the struct is shared verbatim between the Rust
//! side (batch building) and the WGSL vertex shader (instance buffer layout),
//! so any field change must be mirrored in `gpu.rs`'s shader source.

use bytemuck::{Pod, Zeroable};

/// Everything the GPU needs to draw one sprite quad. 52 bytes.
///
/// Positions and sizes are screen-space pixels (x right, y down) as produced
/// by the `world` crate's projector; the orthographic camera turns them into
/// clip space in the vertex shader.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct SpriteInstance {
    /// Screen-space centre of the quad in pixels.
    pub position: [f32; 2],
    /// Quad size in pixels (width, height) before rotation.
    pub scale: [f32; 2],
    /// Rotation about the quad centre, radians.
    pub rotation: f32,
    /// Normalized atlas rectangle (u0, v0, u1, v1); see [`crate::atlas`].
    pub uv_rect: [f32; 4],
    /// Multiply tint, linear RGBA in 0..1.
    pub tint: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<SpriteInstance>() == 52);
const _: () = assert!(std::mem::align_of::<SpriteInstance>() == 4);

impl SpriteInstance {
    /// An instance spanning the full atlas (identity UVs), untinted.
    pub const fn fullscreen(position: [f32; 2], scale: [f32; 2]) -> Self {
        Self {
            position,
            scale,
            rotation: 0.0,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0, 1.0, 1.0, 1.0],
        }
    }
}
