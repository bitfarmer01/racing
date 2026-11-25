//! wgpu sprite-batch renderer: instanced atlas sprites, an orthographic
//! camera over the `world` projector's output, and quality-tier plumbing.
//!
//! Layering per the architecture blueprint: every decision (batch building,
//! UV mapping, transforms, culling, tier selection) is a pure function in the
//! device-free modules; `gpu` is the only module that touches a device and is
//! untested on GPU-less hosts by design.

#![forbid(unsafe_code)]

pub mod atlas;
pub mod batch;
pub mod camera;
pub mod gpu;
pub mod instance;
pub mod sampling;
pub mod tier;

pub use atlas::{
    check_atlas_budget, uv_rect, uv_rect_inset, AtlasDims, AtlasTooLarge, MAX_ATLAS_BYTES,
};
pub use batch::SpriteBatch;
pub use camera::{
    instance_from_billboard, instance_rolled, push_projected, OrthoCamera, RoadPlacement,
    SpriteStyle,
};
pub use gpu::{AtlasUpload, GpuRenderer, RenderError, RoadVertex, ROAD_SHADER, SPRITE_SHADER};
pub use instance::SpriteInstance;
pub use tier::{Backend, Effects, QualityTier};
