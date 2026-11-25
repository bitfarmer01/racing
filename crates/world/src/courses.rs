//! Wave 8 — the course catalog (RED-STATE SKELETON).
//!
//! This module is the compile seam for the wave-8 failing acceptance tests
//! (spec: `.obvious/specs/wave8-tracks.md`). It carries the course-definition
//! API surface ONLY: the catalog still registers exactly one course — the
//! wave-7 demo — so every wave-8 test in this file fails at assertion today
//! and passes, unmodified, once the catalog lists the spec'd courses and the
//! shell wires selection through. None of the feature is implemented here.

use crate::geometry::{CurveSegment, ElevationKey, Landmark, ZoneLayout};

/// Stable selector for a course, mirrored in the web shell's track-select UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CourseId {
    /// The wave-7 demo course (2 000 m: coast / town / forest / ridge).
    CoastRun,
    /// The wave-8 island loop (1 600 m: lagoon / village / jungle / summit).
    IslandHop,
}

/// One selectable course: pure data end to end — its own zone table,
/// landmarks, corner composition, hill profile, and lap length.
pub struct Course {
    pub id: CourseId,
    pub name: &'static str,
    pub blurb: &'static str,
    /// The course's zone table and lap length (the 7c machinery).
    pub layout: ZoneLayout,
    /// Large "you are here" anchors at fixed per-lap distances.
    pub landmarks: &'static [Landmark],
    /// The course's corner composition; `RoadGeometry::new` consumes it.
    pub curves: &'static [CurveSegment],
    /// The course's hill profile; `RoadGeometry::new` consumes it.
    pub elevation: &'static [ElevationKey],
    pub road_width: f32,
}

/// Every selectable course, in selection-UI order.
///
/// RED STATE: only the demo course is registered. The wave-8 implementation
/// adds Island Hop (and any later course) here — see the spec's course table.
pub fn catalog() -> &'static [Course] {
    // OnceLock rather than a promoted const: the demo entry borrows the
    // landmark table through `course_landmarks()`, a function call, which
    // const promotion cannot evaluate.
    static CATALOG: std::sync::OnceLock<[Course; 1]> = std::sync::OnceLock::new();
    CATALOG
        .get_or_init(|| {
            [Course {
                id: CourseId::CoastRun,
                name: "Coast Run",
                blurb: "The wave-7 demo lap: coast out of the grid, through the town, \
                        up the forest climb, along the ridge.",
                layout: ZoneLayout::FOR_TRACK,
                landmarks: crate::course_landmarks(),
                // RED STATE: the demo course's corner and hill tables still
                // live in runtime::track_geometry; moving them into this entry
                // is part of the wave-8 implementation, so they are not
                // restated (or tested) yet.
                curves: &[],
                elevation: &[],
                road_width: 12.0,
            }]
        })
        .as_slice()
}

/// Resolves a course by id, for boot and for the shell's track-select flow.
pub fn for_course(id: CourseId) -> Option<&'static Course> {
    catalog().iter().find(|course| course.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One spec'd zone span: name, start, length — in meters of the lap.
    type SpecZone = (&'static str, f32, f32);

    /// The spec's expected course table (`.obvious/specs/wave8-tracks.md`,
    /// "The courses"): every course the wave-8 catalog must list, with its
    /// display name, lap length, and zone sequence.
    const EXPECTED: &[(CourseId, &str, f32, &[SpecZone])] = &[
        (
            CourseId::CoastRun,
            "Coast Run",
            2_000.0,
            &[
                ("coast", 0.0, 520.0),
                ("town", 520.0, 560.0),
                ("forest", 1_080.0, 520.0),
                ("ridge", 1_600.0, 400.0),
            ],
        ),
        (
            CourseId::IslandHop,
            "Island Hop",
            1_600.0,
            &[
                ("lagoon", 0.0, 480.0),
                ("village", 480.0, 440.0),
                ("jungle", 920.0, 360.0),
                ("summit", 1_280.0, 320.0),
            ],
        ),
    ];

    fn course_of(id: CourseId) -> &'static Course {
        for_course(id)
            .expect("the spec's course must be registered — see .obvious/specs/wave8-tracks.md")
    }

    /// The course's zone sequence, read through the same public sampling API
    /// the render path consumes: one mid-zone sample per spec'd span. Every
    /// span sits well past the 80 m blend window, so a sample answers with
    /// exactly one zone's name.
    fn sampled_sequence(layout: &ZoneLayout, zones: &[SpecZone]) -> Vec<&'static str> {
        zones
            .iter()
            .map(|(name, start, length)| {
                let mid = start + length * 0.5;
                assert_eq!(
                    layout.sample(mid).name,
                    *name,
                    "mid-zone sample at {mid} m must resolve to {name}"
                );
                *name
            })
            .collect()
    }

    #[test]
    fn wave8_catalog_lists_two_or_more_distinct_courses() {
        let ids: Vec<CourseId> = catalog().iter().map(|course| course.id).collect();
        for (id, name, _, _) in EXPECTED {
            assert!(
                ids.contains(id),
                "the catalog must list {} ({id:?}) — it lists {} course(s)",
                name,
                ids.len(),
            );
        }
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "course ids must be distinct");
    }

    #[test]
    fn wave8_a_second_course_defines_its_own_zone_sequence() {
        let island = course_of(CourseId::IslandHop);
        let demo = course_of(CourseId::CoastRun);
        // Island Hop parses and resolves to its own spec'd sequence — not the
        // demo course's coast/town/forest/ridge.
        let island_zones = &EXPECTED[1].3;
        let demo_zones = &EXPECTED[0].3;
        let island_names = sampled_sequence(&island.layout, island_zones);
        let demo_names = sampled_sequence(&demo.layout, demo_zones);
        assert_ne!(
            island_names, demo_names,
            "the second course must not restate the demo course's zone sequence"
        );
    }

    #[test]
    fn wave8_every_course_tiles_its_lap_and_carries_its_specd_lap_length() {
        for (id, _, lap_length, zones) in EXPECTED {
            let course = course_of(*id);
            // The zones resolve in the spec'd order at the spec'd distances —
            // a gap or overlap in the table would misresolve a mid-zone sample.
            sampled_sequence(&course.layout, zones);
            // The spec'd spans tile the lap: they sum to the lap length, and
            // the course's own lap_length is the spec'd one (the HUD and the
            // lap-bar markers read exactly this).
            let tiled: f32 = zones.iter().map(|(_, _, length)| length).sum();
            assert!(
                (tiled - lap_length).abs() < 1e-3,
                "{id:?}'s spec'd zones tile {tiled} m but its lap is {lap_length} m"
            );
            assert!(
                (course.layout.lap_length - lap_length).abs() < 1e-3,
                "{id:?} lap_length is {} m, the spec asks for {lap_length} m",
                course.layout.lap_length
            );
            // Landmarks are "you are here" anchors: fixed distances inside
            // the course's own lap.
            for landmark in course.landmarks {
                assert!(
                    landmark.distance < course.layout.lap_length,
                    "{id:?} landmark at {} m sits past its {} m lap",
                    landmark.distance,
                    course.layout.lap_length
                );
            }
        }
    }

    #[test]
    fn wave8_course_lookup_resolves_distinct_courses_for_distinct_ids() {
        // The selection semantic the shell consumes: two ids, two different
        // courses — different laps, different places.
        let a = course_of(CourseId::CoastRun);
        let b = course_of(CourseId::IslandHop);
        assert_ne!(a.id, b.id);
        assert!(
            (a.layout.lap_length - b.layout.lap_length).abs() > 1.0,
            "distinct courses must not share the demo's lap length"
        );
        assert_ne!(
            sampled_sequence(&a.layout, &EXPECTED[0].3),
            sampled_sequence(&b.layout, &EXPECTED[1].3),
            "distinct ids must resolve to distinct zone sequences"
        );
    }
}
