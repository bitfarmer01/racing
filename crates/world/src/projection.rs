//! Projection of road-space points to the screen, plus the road-strip and
//! billboard helpers the renderer consumes.

use crate::camera::Camera;
use crate::geometry::RoadGeometry;

/// Depth closer than this (measured from the camera) is culled, so the
/// projection never divides by zero or flips sign.
pub const NEAR_PLANE: f32 = 1.0;

/// A projected point: screen position plus the local pixels-per-world-unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projected {
    pub screen_x: f32,
    pub screen_y: f32,
    /// Pixels per world unit at this depth.
    pub scale: f32,
}

/// A roadside sprite rect: centre x, road-contact bottom y, pixel size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Billboard {
    pub center_x: f32,
    pub bottom_y: f32,
    pub width: f32,
    pub height: f32,
}

/// One trapezoid of road between two distances: projected near and far edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoadStrip {
    pub near_x: f32,
    pub near_y: f32,
    pub near_half_width: f32,
    pub far_x: f32,
    pub far_y: f32,
    pub far_half_width: f32,
}

/// Bundles geometry and camera with the camera-space terms precomputed once,
/// so per-call projection is a handful of f32 ops with zero allocations.
pub struct Projector<'a> {
    geometry: &'a RoadGeometry,
    camera: Camera,
    camera_elevation: f32,
    camera_offset: f32,
}

impl<'a> Projector<'a> {
    pub fn new(geometry: &'a RoadGeometry, camera: Camera) -> Self {
        Self {
            geometry,
            camera,
            camera_elevation: geometry.elevation_at(camera.distance) + camera.height,
            camera_offset: geometry.center_offset(camera.distance) + camera.lateral,
        }
    }

    /// The vanishing line: the y the road converges toward at infinite
    /// distance.
    pub fn horizon_y(&self) -> f32 {
        self.camera.viewport.center_y()
    }

    /// Projects a point `lateral` units off the road centreline at `distance`
    /// along the road. Returns `None` for anything within the near plane
    /// (at or behind the camera).
    pub fn project(&self, distance: f32, lateral: f32) -> Option<Projected> {
        let dz = distance - self.camera.distance;
        if dz < NEAR_PLANE {
            return None;
        }
        let scale = self.camera.focal / dz;
        let rel_x = lateral + self.geometry.center_offset(distance) - self.camera_offset;
        let rel_y = self.geometry.elevation_at(distance) - self.camera_elevation;
        Some(Projected {
            screen_x: self.camera.viewport.center_x() + scale * rel_x,
            screen_y: self.camera.viewport.center_y() - scale * rel_y,
            scale,
        })
    }

    /// Screen rect for a roadside object `world_width` by `world_height`
    /// whose base sits at (`distance`, `lateral`). Culled like any other
    /// point.
    pub fn billboard(
        &self,
        distance: f32,
        lateral: f32,
        world_width: f32,
        world_height: f32,
    ) -> Option<Billboard> {
        let p = self.project(distance, lateral)?;
        Some(Billboard {
            center_x: p.screen_x,
            bottom_y: p.screen_y,
            width: p.scale * world_width,
            height: p.scale * world_height,
        })
    }

    /// Road strips from the near plane out to `max_distance`, near to far,
    /// `step` world units apart. Consecutive strips share each projected
    /// edge exactly, so they tile without cracks. Allocates nothing.
    pub fn road_strips(&self, step: f32, max_distance: f32) -> RoadStrips<'_, 'a> {
        RoadStrips {
            projector: self,
            step,
            next_near: self.camera.distance + NEAR_PLANE,
            max_distance,
        }
    }
}

/// Iterator over road strips; see [`Projector::road_strips`].
pub struct RoadStrips<'a, 'b> {
    projector: &'a Projector<'b>,
    step: f32,
    next_near: f32,
    max_distance: f32,
}

impl Iterator for RoadStrips<'_, '_> {
    type Item = RoadStrip;

    fn next(&mut self) -> Option<RoadStrip> {
        loop {
            let near = self.next_near;
            if near >= self.max_distance {
                return None;
            }
            let far = (near + self.step).min(self.max_distance);
            self.next_near = far;
            if let (Some(n), Some(f)) = (
                self.projector.project(near, 0.0),
                self.projector.project(far, 0.0),
            ) {
                let half_width = self.projector.geometry.road_width() * 0.5;
                return Some(RoadStrip {
                    near_x: n.screen_x,
                    near_y: n.screen_y,
                    near_half_width: n.scale * half_width,
                    far_x: f.screen_x,
                    far_y: f.screen_y,
                    far_half_width: f.scale * half_width,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Viewport;
    use crate::geometry::{CurveSegment, ElevationKey};

    const VIEWPORT: Viewport = Viewport {
        width: 800.0,
        height: 450.0,
    };

    fn camera_at(distance: f32) -> Camera {
        Camera {
            distance,
            height: 1.2,
            lateral: 0.0,
            focal: 300.0,
            viewport: VIEWPORT,
        }
    }

    fn straight_road() -> RoadGeometry {
        RoadGeometry::new(Vec::new(), Vec::new(), 12.0)
    }

    #[test]
    fn straight_road_projects_to_screen_center() {
        let road = straight_road();
        let p = Projector::new(&road, camera_at(0.0));
        for d in [5.0, 50.0, 500.0] {
            let pr = p.project(d, 0.0).expect("ahead of camera");
            assert!(
                (pr.screen_x - VIEWPORT.center_x()).abs() < 1e-4,
                "straight road centre must sit on the screen centre"
            );
        }
    }

    #[test]
    fn flat_road_surface_sits_below_the_horizon() {
        let road = straight_road();
        let p = Projector::new(&road, camera_at(0.0));
        let pr = p.project(50.0, 0.0).expect("ahead of camera");
        assert!(pr.screen_y > p.horizon_y());
    }

    #[test]
    fn scale_decreases_monotonically_with_distance() {
        let road = straight_road();
        let p = Projector::new(&road, camera_at(0.0));
        let mut prev = f32::INFINITY;
        for d in [2.0, 5.0, 10.0, 50.0, 100.0, 500.0, 5_000.0] {
            let pr = p.project(d, 0.0).expect("ahead of camera");
            assert!(pr.scale < prev, "scale must shrink with distance at {d}");
            prev = pr.scale;
        }
    }

    #[test]
    fn curve_shifts_x_monotonically_with_distance() {
        let road = RoadGeometry::new(
            vec![CurveSegment {
                start: 0.0,
                length: 2_000.0,
                curvature: 0.002,
            }],
            Vec::new(),
            12.0,
        );
        let p = Projector::new(&road, camera_at(0.0));
        let mut prev = VIEWPORT.center_x();
        for d in [5.0, 50.0, 100.0, 300.0, 800.0, 1_500.0] {
            let pr = p.project(d, 0.0).expect("ahead of camera");
            assert!(
                pr.screen_x > prev,
                "curve must shift x rightward with distance at {d}"
            );
            prev = pr.screen_x;
        }
    }

    #[test]
    fn points_behind_the_camera_are_culled() {
        let road = straight_road();
        let p = Projector::new(&road, camera_at(100.0));
        assert!(p.project(99.9, 0.0).is_none(), "behind the camera");
        assert!(p.project(100.0, 0.0).is_none(), "at the camera plane");
        assert!(p.project(100.5, 0.0).is_none(), "inside the near plane");
        assert!(p.project(101.0, 0.0).is_some(), "one unit ahead is kept");
        assert!(
            p.billboard(50.0, 0.0, 2.0, 2.0).is_none(),
            "billboards cull like any point"
        );
    }

    #[test]
    fn hill_profile_raises_the_horizon_line() {
        let hilly = RoadGeometry::new(
            Vec::new(),
            vec![
                ElevationKey {
                    distance: 0.0,
                    elevation: 0.0,
                },
                ElevationKey {
                    distance: 500.0,
                    elevation: 25.0,
                },
            ],
            12.0,
        );
        let flat = straight_road();
        let cam = camera_at(0.0);
        let p_flat = Projector::new(&flat, cam);
        let p_hill = Projector::new(&hilly, cam);
        let flat_y = p_flat.project(400.0, 0.0).unwrap().screen_y;
        let hill_y = p_hill.project(400.0, 0.0).unwrap().screen_y;
        assert!(hill_y < flat_y, "the climb must sit above the flat road");
        assert!(flat_y > p_flat.horizon_y(), "flat surface below horizon");
        assert!(hill_y < p_hill.horizon_y(), "climb pokes above the horizon");
    }

    #[test]
    fn strips_run_near_to_far_and_tile_without_gaps() {
        let road = straight_road();
        let p = Projector::new(&road, camera_at(0.0));
        let strips: Vec<_> = p.road_strips(10.0, 400.0).collect();
        assert!(!strips.is_empty());
        for (i, s) in strips.iter().enumerate() {
            assert!(s.far_y < s.near_y, "strip {i} must recede upward");
            assert!(
                s.far_half_width < s.near_half_width,
                "strip {i} must narrow with distance"
            );
            if let Some(next) = strips.get(i + 1) {
                assert_eq!(s.far_x, next.near_x, "shared edge x must match");
                assert_eq!(s.far_y, next.near_y, "shared edge y must match");
            }
        }
        let last = strips.last().unwrap();
        let horizon = p.horizon_y();
        assert!(
            last.far_y <= strips[0].near_y && last.far_y >= horizon,
            "strips must converge toward the horizon"
        );
    }

    #[test]
    fn strips_skip_culled_ranges() {
        // The first strip starts exactly at the near plane: everything
        // closer is culled, and the near edge matches a direct projection.
        let road = straight_road();
        let p = Projector::new(&road, camera_at(0.0));
        let first = p.road_strips(10.0, 400.0).next().unwrap();
        let pr = p.project(crate::NEAR_PLANE, 0.0).unwrap();
        assert!((first.near_x - pr.screen_x).abs() < 1e-4);
        assert!((first.near_y - pr.screen_y).abs() < 1e-4);
    }

    #[test]
    fn billboard_scales_with_distance_and_anchors_on_the_road() {
        let road = straight_road();
        let p = Projector::new(&road, camera_at(0.0));
        let near = p.billboard(20.0, 8.0, 3.0, 2.0).unwrap();
        let far = p.billboard(200.0, 8.0, 3.0, 2.0).unwrap();
        assert!(near.width > far.width && near.height > far.height);
        assert!(near.bottom_y > far.bottom_y, "closer means lower on screen");
        // The base sits exactly where the road surface projects.
        let surface = p.project(20.0, 8.0).unwrap();
        assert!((near.bottom_y - surface.screen_y).abs() < 1e-4);
        // Both billboards sit right of centre at the same lateral offset.
        assert!(near.center_x > VIEWPORT.center_x());
    }
}
