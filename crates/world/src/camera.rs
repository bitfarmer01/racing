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
/// prefers to sit, in metres. Bounds the crash/off-road placement so the
/// camera axis does not chase the bike far off the tarmac in the common
/// case — but the rider band below is the harder guarantee: when a bend's
/// centreline sweep pushes the rider past the band at this bound, the axis
/// follows the rider past it rather than lose them off-frame.
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
    /// plus a small off-road margin (so ordinary lean and shake never walk
    /// the axis off the tarmac), then shifts the axis just enough to hold
    /// the rider inside a bounded screen band. When the two bounds conflict
    /// — a hairpin's centreline sweep across the camera trail can push the
    /// rider past the band at the road bound — the rider band wins: the
    /// axis follows the rider past the road bound, because a rider lost
    /// off-frame is unplayable while a road partially out of frame at a
    /// deep excursion is not (the near strips stay on screen, and sim-side
    /// recovery pulls the rider back toward the tarmac).
    pub fn framed_for_rider(mut self, bounds: FramingBounds) -> Self {
        // The road in frame: the camera axis stays within the road edges
        // plus the off-road margin, whatever the lean terms would do to it.
        let axis_max = bounds.road_half_width + AXIS_OFFROAD_MARGIN_M;
        self.lateral = self.lateral.clamp(-axis_max, axis_max);

        // The rider in a bounded band: shift the axis toward the rider just
        // enough to bring their screen offset back inside the band. The
        // shift is unbounded on purpose — this is the guarantee the frame
        // cannot lose.
        let px_per_m = self.focal / bounds.trail;
        let band_m = RIDER_BAND_FRAC * self.viewport.width * 0.5 / px_per_m;
        let offset_m = bounds.rider_axis - self.lateral;
        let shift_m = offset_m - offset_m.clamp(-band_m, band_m);
        self.lateral += shift_m;
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
    fn the_road_bound_holds_while_the_band_shift_fits_inside_it() {
        // A moderate excursion: the band shift needs less room than the
        // road bound allows, so the axis stays within the road edges plus
        // margin — the road ahead keeps its place in frame.
        let framed = camera_at(0.0).framed_for_rider(bounds(10.0));
        assert!(framed.lateral > 0.0, "the axis shifts toward the rider");
        assert!(
            framed.lateral <= 6.0 + AXIS_OFFROAD_MARGIN_M + 1e-4,
            "a band shift that fits must not leave the road bound (got {})",
            framed.lateral
        );
    }

    #[test]
    fn a_hairpin_sweep_past_the_road_bound_still_holds_the_rider() {
        // The climbing hairpin: the centreline sweep across the 4 m camera
        // trail plus a frozen crash-frame lateral pushes the rider's axis
        // to ~14.7 m — past the road bound (8 m) plus the band (~4.3 m),
        // where the old camera truncated the shift and lost the rider off
        // the right edge. The band is the harder bound: the axis follows
        // the rider past the road bound, both directions.
        let band_m = RIDER_BAND_FRAC * VIEWPORT.width * 0.5 * 4.0 / 300.0;
        let framed = camera_at(1.5).framed_for_rider(bounds(14.7));
        let offset_px = rider_screen_offset(framed, bounds(14.7)).abs();
        assert!(
            offset_px <= RIDER_BAND_FRAC * VIEWPORT.width * 0.5 + 1e-3,
            "the hairpin rider must stay in the band (got {offset_px} px)"
        );
        assert!(
            framed.lateral > 6.0 + AXIS_OFFROAD_MARGIN_M,
            "the band shift must be allowed past the road bound (got {})",
            framed.lateral
        );
        assert!((framed.lateral - (14.7 - band_m)).abs() < 1e-3);

        let framed_left = camera_at(-1.5).framed_for_rider(bounds(-14.7));
        let offset_left = rider_screen_offset(framed_left, bounds(-14.7)).abs();
        assert!(offset_left <= RIDER_BAND_FRAC * VIEWPORT.width * 0.5 + 1e-3);
        assert!(framed_left.lateral < -(6.0 + AXIS_OFFROAD_MARGIN_M));
    }

    #[test]
    fn an_extreme_excursion_still_holds_the_rider_in_the_band() {
        // Even a beyond-the-sim excursion (the 18-30 m/s window used to
        // allow these) cannot push the rider off-frame: the band shift is
        // unbounded, whatever the excursion depth.
        for rider_axis in [17.0, 40.0, 100.0] {
            let framed = camera_at(8.0).framed_for_rider(bounds(rider_axis));
            let offset_px = rider_screen_offset(framed, bounds(rider_axis)).abs();
            let band_px = RIDER_BAND_FRAC * VIEWPORT.width * 0.5;
            assert!(
                offset_px <= band_px + 1e-3,
                "rider at {rider_axis} m sits {offset_px} px off, past the {band_px} px band"
            );
            // And the camera never crosses the rider: it trails behind
            // them by at most a band.
            assert!(framed.lateral.abs() <= rider_axis.abs() + 1e-4);
        }
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
    fn framing_shifts_toward_the_rider_and_holds_them_in_the_band() {
        // A bike off the right edge shifts the camera axis right (toward
        // the bike); mirrored to the left.
        let framed = camera_at(0.0).framed_for_rider(bounds(9.0));
        assert!(framed.lateral > 0.0, "the axis shifts toward the rider");

        let framed_left = camera_at(0.0).framed_for_rider(bounds(-9.0));
        assert!(framed_left.lateral < 0.0, "the axis mirrors to the left");
    }

    #[test]
    fn an_in_band_rider_changes_nothing() {
        // A centred bike on a straight road: framing is the identity.
        let framed = camera_at(0.5).framed_for_rider(bounds(0.5));
        assert!((framed.lateral - 0.5).abs() < 1e-4);
    }
}
