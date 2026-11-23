//! 2D orthographic camera: pixel-space (as the `world` projector emits it)
//! to clip-space, as pure math with no device involved.

use world::Billboard;

use crate::batch::SpriteBatch;
use crate::instance::SpriteInstance;

/// Orthographic camera over a pixel-space viewport with y pointing down —
/// the same convention the `world` crate's projector uses, so projected
/// coordinates feed straight in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OrthoCamera {
    pub width: f32,
    pub height: f32,
}

impl OrthoCamera {
    /// Panics on a zero-sized viewport; a camera with no output surface has
    /// no meaningful projection, and the caller builds the camera from real
    /// surface dimensions.
    pub fn new(width: f32, height: f32) -> Self {
        assert!(width > 0.0 && height > 0.0, "viewport must be non-empty");
        Self { width, height }
    }

    /// Column-major 4x4 clip matrix for `wgpu` (mat4x4 columns are the inner
    /// arrays). Maps pixel x right to clip x right, flips pixel y down to
    /// clip y up, and keeps depth in 0..1.
    pub fn clip_matrix(&self) -> [[f32; 4]; 4] {
        [
            [2.0 / self.width, 0.0, 0.0, 0.0],
            [0.0, -2.0 / self.height, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0, 1.0],
        ]
    }

    /// Transforms one screen-space point to clip space — the same mapping
    /// [`clip_matrix`](Self::clip_matrix) applies, exposed as a function for
    /// callers that reason in points rather than matrices.
    pub fn point_to_clip(&self, x: f32, y: f32) -> [f32; 2] {
        [2.0 * x / self.width - 1.0, 1.0 - 2.0 * y / self.height]
    }
}

/// Builds a screen-space instance from the `world` projector's billboard
/// output. The billboard reports the road-contact bottom edge; the sprite is
/// centred on that rect, which is what the instanced quad shader expects.
pub fn instance_from_billboard(billboard: Billboard, style: SpriteStyle) -> SpriteInstance {
    SpriteInstance {
        position: [
            billboard.center_x,
            billboard.bottom_y - billboard.height * 0.5,
        ],
        scale: [billboard.width, billboard.height],
        rotation: 0.0,
        uv_rect: style.uv_rect,
        tint: style.tint,
    }
}

/// Where a sprite sits on the road — the four inputs to
/// `world::Projector::billboard`, bundled so callers pass one named value
/// instead of four loose floats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadPlacement {
    /// Distance along the road from the camera plane.
    pub distance: f32,
    /// Lateral offset across the road, in world units.
    pub lateral: f32,
    /// Sprite width in world units (projected to screen width).
    pub world_width: f32,
    /// Sprite height in world units (projected to screen height).
    pub world_height: f32,
}

/// Atlas region and tint for one sprite — the two appearance inputs shared by
/// every push helper, bundled to keep signatures short.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteStyle {
    /// Atlas UV rect `[u0, v0, u1, v1]` from [`uv_rect`](crate::uv_rect).
    pub uv_rect: [f32; 4],
    /// Linear RGBA multiply, 0..1.
    pub tint: [f32; 4],
}

/// Projects a road-space sprite through the `world` crate's projector and
/// pushes the resulting instance onto the batch in one step.
///
/// Returns `false` without touching the batch when the point falls behind the
/// near plane (the projector culls it) — caller decides whether that is an
/// error or an expected off-screen case.
pub fn push_projected(
    batch: &mut SpriteBatch,
    projector: &world::Projector<'_>,
    placement: RoadPlacement,
    style: SpriteStyle,
) -> bool {
    match projector.billboard(
        placement.distance,
        placement.lateral,
        placement.world_width,
        placement.world_height,
    ) {
        Some(billboard) => batch.push(instance_from_billboard(billboard, style)),
        None => false,
    }
}
