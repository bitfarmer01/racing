//! Pure-math 2.5D road projection in the OutRun / Road Rash lineage.
//!
//! Zero external dependencies, 100% safe Rust, deterministic f32 math.
#![forbid(unsafe_code)]

mod camera;
mod geometry;
mod projection;

pub use camera::{Camera, Viewport};
pub use geometry::{
    course_landmarks, pick_dressing, CurveSegment, DressingKind, DressingMix, ElevationKey,
    Landmark, LandmarkKind, RoadGeometry, Zone, ZoneLayout, ZoneSample,
};
pub use projection::{Billboard, Projected, Projector, RoadStrip, RoadStrips, NEAR_PLANE};
