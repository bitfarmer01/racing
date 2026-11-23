//! Road geometry: curvature and hill profile as functions of distance along
//! the road.

/// A stretch of road with constant signed curvature.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveSegment {
    /// Distance along the road where the segment begins.
    pub start: f32,
    /// Length of the segment in world units.
    pub length: f32,
    /// Signed curvature: heading change per unit of distance.
    pub curvature: f32,
}

/// One anchor of the piecewise-linear hill profile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElevationKey {
    /// Distance along the road.
    pub distance: f32,
    /// Road-surface height at that distance.
    pub elevation: f32,
}

/// Curvature and hill profile over distance, plus road width.
///
/// Road before the first curve and past the last one is straight; elevation
/// holds the nearest anchor's value outside the keyed range.
#[derive(Debug, Clone, PartialEq)]
pub struct RoadGeometry {
    curves: Vec<CurveSegment>,
    elevation: Vec<ElevationKey>,
    road_width: f32,
}

impl RoadGeometry {
    /// Builds geometry, sorting segments and anchors by distance so callers
    /// need not pre-order them.
    pub fn new(
        mut curves: Vec<CurveSegment>,
        mut elevation: Vec<ElevationKey>,
        road_width: f32,
    ) -> Self {
        curves.sort_by(|a, b| a.start.total_cmp(&b.start));
        elevation.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        Self {
            curves,
            elevation,
            road_width,
        }
    }

    /// Full width of the road surface in world units.
    pub fn road_width(&self) -> f32 {
        self.road_width
    }

    /// Signed curvature at `distance`; zero outside any segment.
    pub fn curvature_at(&self, distance: f32) -> f32 {
        for seg in &self.curves {
            if distance < seg.start {
                break;
            }
            if distance <= seg.start + seg.length {
                return seg.curvature;
            }
        }
        0.0
    }

    /// Accumulated heading at `distance`: the integral of curvature.
    pub fn heading_at(&self, distance: f32) -> f32 {
        let mut heading = 0.0_f32;
        for seg in &self.curves {
            if distance <= seg.start {
                break;
            }
            heading += seg.curvature * seg.length.min(distance - seg.start);
        }
        heading
    }

    /// Lateral offset of the road centre at `distance`, relative to the
    /// centreline heading at distance zero. Analytic integral of curvature,
    /// so the result is exact, allocation-free, and deterministic.
    pub fn center_offset(&self, distance: f32) -> f32 {
        let mut x = 0.0_f32;
        let mut heading = 0.0_f32;
        let mut covered = 0.0_f32;
        for seg in &self.curves {
            if distance <= covered {
                break;
            }
            let start = seg.start.max(covered);
            x += heading * (start - covered); // any gap between segments is straight
            covered = start;
            let s = seg.length.min(distance - start);
            if s > 0.0 {
                x += heading * s + 0.5 * seg.curvature * s * s;
                heading += seg.curvature * s;
                covered += s;
            }
        }
        x += heading * (distance - covered); // straight run past the last segment
        x
    }

    /// Road-surface height at `distance`, piecewise linear between anchors.
    pub fn elevation_at(&self, distance: f32) -> f32 {
        let Some(first) = self.elevation.first() else {
            return 0.0;
        };
        if distance <= first.distance {
            return first.elevation;
        }
        for pair in self.elevation.windows(2) {
            if distance <= pair[1].distance {
                let span = pair[1].distance - pair[0].distance;
                let t = (distance - pair[0].distance) / span;
                return pair[0].elevation + t * (pair[1].elevation - pair[0].elevation);
            }
        }
        self.elevation.last().map_or(0.0, |key| key.elevation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_road_has_zero_offset_heading_and_elevation() {
        let g = RoadGeometry::new(Vec::new(), Vec::new(), 12.0);
        for d in [0.0, 100.0, 5_000.0] {
            assert_eq!(g.center_offset(d), 0.0);
            assert_eq!(g.heading_at(d), 0.0);
            assert_eq!(g.elevation_at(d), 0.0);
            assert_eq!(g.curvature_at(d), 0.0);
        }
    }

    #[test]
    fn curvature_is_constant_within_segment_and_zero_outside() {
        let g = RoadGeometry::new(
            vec![CurveSegment {
                start: 100.0,
                length: 200.0,
                curvature: 0.02,
            }],
            Vec::new(),
            12.0,
        );
        assert_eq!(g.curvature_at(50.0), 0.0);
        assert_eq!(g.curvature_at(100.0), 0.02);
        assert_eq!(g.curvature_at(300.0), 0.02);
        assert_eq!(g.curvature_at(300.5), 0.0);
    }

    #[test]
    fn heading_integrates_curvature_and_holds_past_the_segment() {
        let g = RoadGeometry::new(
            vec![CurveSegment {
                start: 0.0,
                length: 100.0,
                curvature: 0.01,
            }],
            Vec::new(),
            12.0,
        );
        assert!((g.heading_at(50.0) - 0.5).abs() < 1e-6);
        assert!((g.heading_at(100.0) - 1.0).abs() < 1e-6);
        assert!((g.heading_at(150.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn center_offset_matches_the_analytic_integral() {
        let c = 0.01_f32;
        let g = RoadGeometry::new(
            vec![CurveSegment {
                start: 0.0,
                length: 100.0,
                curvature: c,
            }],
            Vec::new(),
            12.0,
        );
        // Inside the segment: offset = c * z^2 / 2.
        let z = 60.0;
        assert!((g.center_offset(z) - 0.5 * c * z * z).abs() < 1e-4);
        // Past the segment: straight continuation at the final heading of 1.0.
        let z = 200.0;
        let expected = 0.5 * c * 100.0 * 100.0 + 1.0 * (z - 100.0);
        assert!((g.center_offset(z) - expected).abs() < 1e-3);
    }

    #[test]
    fn elevation_interpolates_linearly_and_clamps_outside_the_keys() {
        let g = RoadGeometry::new(
            Vec::new(),
            vec![
                ElevationKey {
                    distance: 0.0,
                    elevation: 2.0,
                },
                ElevationKey {
                    distance: 400.0,
                    elevation: 10.0,
                },
            ],
            12.0,
        );
        assert!((g.elevation_at(-50.0) - 2.0).abs() < 1e-6);
        assert!((g.elevation_at(200.0) - 6.0).abs() < 1e-4);
        assert!((g.elevation_at(900.0) - 10.0).abs() < 1e-5);
    }

    #[test]
    fn unsorted_input_is_normalized() {
        let g = RoadGeometry::new(
            vec![
                CurveSegment {
                    start: 200.0,
                    length: 100.0,
                    curvature: 0.01,
                },
                CurveSegment {
                    start: 0.0,
                    length: 100.0,
                    curvature: -0.01,
                },
            ],
            vec![
                ElevationKey {
                    distance: 400.0,
                    elevation: 8.0,
                },
                ElevationKey {
                    distance: 0.0,
                    elevation: 0.0,
                },
            ],
            12.0,
        );
        assert_eq!(g.curvature_at(50.0), -0.01);
        assert_eq!(g.curvature_at(250.0), 0.01);
        assert!((g.elevation_at(400.0) - 8.0).abs() < 1e-6);
    }
}
