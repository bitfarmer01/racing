//! Camera and viewport for the 2.5D projection.

/// Output surface dimensions in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    pub fn center_x(&self) -> f32 {
        self.width * 0.5
    }

    pub fn center_y(&self) -> f32 {
        self.height * 0.5
    }
}

/// Viewer state for the projection. The camera sits on the road centreline
/// at `distance`, `height` above the surface, looking straight down the road.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Distance along the road where the camera sits.
    pub distance: f32,
    /// Height above the road surface.
    pub height: f32,
    /// Focal length in pixels: the pixels-per-world-unit scale at one world
    /// unit of depth.
    pub focal: f32,
    pub viewport: Viewport,
}
