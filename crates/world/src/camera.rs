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

/// Camera axis off-road margin: how far past the road edge the camera axis
/// may sit, in metres. Bounds the crash/off-road placement so the camera
/// never chases the bike far off the tarmac — the road ahead must stay in
/// frame at every lateral offset (turn-visibility audit).
pub const AXIS_OFFROAD_MARGIN_M: f32 = 2.0;

/// Rider screen band: the fraction of the half viewport width the rider may
/// occupy sideways before the camera shifts to hold them in frame. 0.8 keeps
/// the rider clear of the frame edges through turns.
pub const RIDER_BAND_FRAC: f32 = 0.8;

/// Framing inputs for [`Camera::framed_for_rider`], sampled by the caller
/// from the road geometry around the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FramingBounds {
    /// Camera-to-rider distance along the road, in metres (> 0).
    pub trail: f32,
    /// Half the road width, in metres.
    pub road_half_width: f32,
    /// The rider's offset from the camera's road point, in metres:
    /// `player_lateral + centre_offset(player) − centre_offset(camera)`.
    pub rider_axis: f32,
}

impl Camera {
    /// Reframes the camera for a turn: bounds the camera axis to the road
    /// plus a small off-road margin (so the road ahead stays in frame at
    /// every curvature and lateral offset — the camera axis never chases the
    /// bike off the tarmac), then shifts the axis just enough to hold the
    /// rider inside a bounded screen band. The road bound wins when the two
    /// conflict: a deep off-road excursion parks the rider near the frame
    /// edge rather than sweeping the whole road off-screen, and crash
    /// recovery pulls the rider back inside the edge anyway.
    pub fn framed_for_rider(mut self, bounds: FramingBounds) -> Self {
        // The road in frame: the camera axis stays within the road edges
        // plus the off-road margin, whatever the lean terms and the bike
        // excursion would do to it.
        let axis_max = bounds.road_half_width + AXIS_OFFROAD_MARGIN_M;
        self.lateral = self.lateral.clamp(-axis_max, axis_max);

        // The rider in a bounded band: shift the axis toward the rider just
        // enough to bring their screen offset back inside the band, never
        // past the road bound above.
        let px_per_m = self.focal / bounds.trail;
        let band_m = RIDER_BAND_FRAC * self.viewport.width * 0.5 / px_per_m;
        let offset_m = bounds.rider_axis - self.lateral;
        let shift_m = offset_m - offset_m.clamp(-band_m, band_m);
        self.lateral = (self.lateral + shift_m).clamp(-axis_max, axis_max);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: Viewport = Viewport {
        width: 800.0,
        height: 450.0,
    };

    fn camera_at(lateral: f32) -> Camera {
        Camera {
            distance: 100.0,
            height: 1.9,
            lateral,
            focal: 300.0,
            horizon_y_px: 180.0,
            viewport: VIEWPORT,
        }
    }

    fn bounds(rider_axis: f32) -> FramingBounds {
        FramingBounds {
            trail: 4.0,
            road_half_width: 6.0,
            rider_axis,
        }
    }

    /// The rider's screen offset from centre under the framed camera.
    fn rider_screen_offset(camera: Camera, bounds: FramingBounds) -> f32 {
        let px_per_m = camera.focal / bounds.trail;
        px_per_m * (bounds.rider_axis - camera.lateral)
    }

    #[test]
    fn an_extreme_excursion_never_pushes_the_camera_axis_off_the_road() {
        // The lean terms and a deep off-road bike both pull the same way;
        // the camera axis still stays within the road edges plus margin.
        let framed = camera_at(40.0).framed_for_rider(bounds(40.0));
        assert!(framed.lateral <= 6.0 + AXIS_OFFROAD_MARGIN_M + 1e-4);
        assert!(framed.lateral >= -(6.0 + AXIS_OFFROAD_MARGIN_M) - 1e-4);
    }

    #[test]
    fn a_rider_at_the_road_edge_stays_inside_the_screen_band() {
        // Bike hugging the edge (6 m) with the bend sweeping it 2 m further
        // off-axis: the band clamp shifts the axis to hold the rider in
        // frame — the pre-fix camera left them at the frame edge.
        let rider_axis = 8.0;
        let framed = camera_at(0.0).framed_for_rider(bounds(rider_axis));
        let offset_px = rider_screen_offset(framed, bounds(rider_axis)).abs();
        let band_px = RIDER_BAND_FRAC * VIEWPORT.width * 0.5;
        assert!(
            offset_px <= band_px + 1e-3,
            "rider offset {offset_px} px exceeds the {band_px} px band"
        );
    }

    #[test]
    fn a_deep_off_road_excursion_keeps_the_road_in_frame() {
        // The old off-road camera followed the bike's lateral outright; a
        // deep excursion swept the road clean off the screen. The framed
        // camera parks at the road bound, so the road always stays visible.
        let framed = camera_at(17.0).framed_for_rider(bounds(17.0));
        assert!((framed.lateral - (6.0 + AXIS_OFFROAD_MARGIN_M)).abs() < 1e-4);
    }

    #[test]
    fn the_band_holds_for_every_sim_reachable_excursion() {
        // A crash freezes the bike the moment it crosses the edge above the
        // off-road crash speed, and recovery places it 3 m back inside the
        // edge — so real excursions top out a few metres past the edge. Up
        // to the road bound plus one band, framing holds the rider inside
        // the screen band while keeping the camera axis at the road bound.
        let half_width = 6.0;
        let axis_max = half_width + AXIS_OFFROAD_MARGIN_M;
        let band_m = RIDER_BAND_FRAC * VIEWPORT.width * 0.5 * 4.0 / 300.0;
        for rider_axis in [6.5, 8.0, 9.0, 10.0, 11.0, axis_max + band_m] {
            let framed = camera_at(0.0).framed_for_rider(bounds(rider_axis));
            let offset_px = rider_screen_offset(framed, bounds(rider_axis)).abs();
            let band_px = RIDER_BAND_FRAC * VIEWPORT.width * 0.5;
            assert!(
                offset_px <= band_px + 1e-3,
                "rider at {rider_axis} m sits {offset_px} px off, past the {band_px} px band"
            );
            assert!(framed.lateral <= axis_max + 1e-4);
        }
    }

    #[test]
    fn framing_shifts_toward_the_rider_and_never_past_the_road_bound() {
        // A bike off the right edge shifts the camera axis right (toward
        // the bike), but no shift can push the axis beyond the road bound.
        let framed = camera_at(0.0).framed_for_rider(bounds(14.0));
        assert!(framed.lateral > 0.0, "the axis shifts toward the rider");
        assert!(framed.lateral <= 6.0 + AXIS_OFFROAD_MARGIN_M + 1e-4);

        let framed_left = camera_at(0.0).framed_for_rider(bounds(-14.0));
        assert!(framed_left.lateral < 0.0, "the axis mirrors to the left");
        assert!(framed_left.lateral >= -(6.0 + AXIS_OFFROAD_MARGIN_M) - 1e-4);
    }

    #[test]
    fn an_in_band_rider_changes_nothing() {
        // A centred bike on a straight road: framing is the identity.
        let framed = camera_at(0.5).framed_for_rider(bounds(0.5));
        assert!((framed.lateral - 0.5).abs() < 1e-4);
    }
}
