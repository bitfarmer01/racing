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
    /// Screen-space roll of the quad, in radians (positive tips the top of
    /// the sprite right). The rider's visual bank; everything else uses 0.0.
    pub roll: f32,
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
        Some(billboard) => batch.push(instance_rolled(billboard, style, placement.roll)),
        None => false,
    }
}

/// Builds a screen-space instance rotated about the sprite's ground-contact
/// point rather than its centre, so a banked rider keeps its wheel on the
/// road. `roll` is radians, positive tipping the top of the sprite right —
/// the shader's rotation convention under screen-space y-down. Zero roll
/// builds the plain billboard instance.
pub fn instance_rolled(billboard: Billboard, style: SpriteStyle, roll: f32) -> SpriteInstance {
    let base = instance_from_billboard(billboard, style);
    if roll == 0.0 {
        return base;
    }
    let half_height = billboard.height * 0.5;
    let (s, c) = roll.sin_cos();
    SpriteInstance {
        // Rotating the quad about its bottom edge: the centre swings
        // sideways by h/2·sin(roll) and drops by h/2·(1 − cos(roll)).
        position: [
            base.position[0] + half_height * s,
            base.position[1] + half_height * (1.0 - c),
        ],
        rotation: base.rotation + roll,
        ..base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world::Viewport;

    const STYLE: SpriteStyle = SpriteStyle {
        uv_rect: [0.0, 0.0, 1.0, 1.0],
        tint: [1.0, 1.0, 1.0, 1.0],
    };

    fn billboard() -> world::Billboard {
        world::Billboard {
            center_x: 100.0,
            bottom_y: 200.0,
            width: 40.0,
            height: 80.0,
        }
    }

    #[test]
    fn zero_roll_is_the_plain_billboard() {
        let plain = instance_rolled(billboard(), STYLE, 0.0);
        let base = instance_from_billboard(billboard(), STYLE);
        assert_eq!(plain, base);
        assert_eq!(plain.rotation, 0.0);
    }

    #[test]
    fn roll_tips_right_and_keeps_the_ground_contact() {
        let roll = 0.3_f32;
        let plain = instance_from_billboard(billboard(), STYLE);
        let rolled = instance_rolled(billboard(), STYLE, roll);
        assert_eq!(rolled.rotation, roll);

        // The centre swings right and rides lower — the top of the sprite
        // tips right while the pivot stays at the road-contact bottom edge.
        assert!(rolled.position[0] > plain.position[0]);
        assert!(rolled.position[1] > plain.position[1]);

        // The pivot is preserved exactly: rotating the centre back about
        // the bottom edge lands on it for both axes' magnitudes.
        let half_height = billboard().height * 0.5;
        let pivot_x = rolled.position[0] - half_height * roll.sin();
        assert!((pivot_x - billboard().center_x).abs() < 1e-4);
    }

    #[test]
    fn push_projected_carries_roll_into_the_batch() {
        let mut batch = SpriteBatch::with_capacity(16);
        // Any in-front placement works: a straight road keeps it on screen.
        let geometry = world::RoadGeometry::new(Vec::new(), Vec::new(), 12.0);
        let camera = world::Camera {
            distance: 0.0,
            height: 1.9,
            lateral: 0.0,
            focal: 300.0,
            horizon_y_px: 180.0,
            viewport: Viewport {
                width: 800.0,
                height: 450.0,
            },
        };
        let projector = world::Projector::new(&geometry, camera);
        let pushed = push_projected(
            &mut batch,
            &projector,
            RoadPlacement {
                distance: 10.0,
                lateral: 0.0,
                world_width: 1.0,
                world_height: 1.7,
                roll: -0.25,
            },
            STYLE,
        );
        assert!(pushed, "on-screen placement pushes");
        assert_eq!(
            batch.instances()[0].rotation,
            -0.25,
            "roll lands on the instance"
        );
    }
}
