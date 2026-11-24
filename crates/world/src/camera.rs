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

    /// Screen y of the viewport at a fraction of the height from the top.
    pub fn y_at_frac(&self, frac: f32) -> f32 {
        self.height * frac
    }
}

/// Viewer state for the projection. The camera sits at `distance` along the
/// road, `height` above the surface, offset `lateral` meters across it, looking
/// straight down the road.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Distance along the road where the camera sits.
    pub distance: f32,
    /// Height above the road surface.
    pub height: f32,
    /// Lateral offset across the road, in meters: the camera lean. Positive
    /// shifts the view right.
    pub lateral: f32,
    /// Focal length in pixels: the pixels-per-world-unit scale at one world
    /// unit of depth.
    pub focal: f32,
    /// Where the vanishing point sits on screen, in pixels from the top.
    /// Mid-screen is the default (`viewport.center_y()`); a smaller value
    /// pitches the view up, lifting the horizon and pushing the road into
    /// the lower part of the screen.
    pub horizon_y_px: f32,
    pub viewport: Viewport,
}
