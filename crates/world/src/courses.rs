//! Wave 8 — the course catalog.
//!
//! Every selectable course is pure, compile-time data (spec:
//! `.obvious/specs/wave8-tracks.md`): its own zone table, landmarks, corner
//! composition, hill profile, and lap length. The zone/landmark machinery in
//! `geometry.rs` is course-agnostic — adding a course is data, never render
//! code. Selection happens at boot only; the shell resolves an id through
//! [`for_course`] and the race is built from the entry's own tables.

use crate::geometry::{CurveSegment, ElevationKey, Landmark, LandmarkKind, ZoneLayout};

/// Stable selector for a course, mirrored in the web shell's track-select UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CourseId {
    /// The wave-7 demo course (2 000 m: coast / town / forest / ridge).
    CoastRun,
    /// The wave-8 island loop (1 600 m: lagoon / village / jungle / summit).
    IslandHop,
}

impl CourseId {
    /// The stable string the web shell's track-select control carries as its
    /// option value and passes across the wasm boundary.
    pub fn slug(self) -> &'static str {
        match self {
            Self::CoastRun => "coast-run",
            Self::IslandHop => "island-hop",
        }
    }

    /// Resolves a selection-UI value back to a course id. An unknown slug is
    /// a boot error on the loading screen — same path as today's boot
    /// failures (spec: "Track-select flow").
    pub fn from_slug(slug: &str) -> Option<Self> {
        [Self::CoastRun, Self::IslandHop]
            .into_iter()
            .find(|id| id.slug() == slug)
    }
}

/// One selectable course: pure data end to end — its own zone table,
/// landmarks, corner composition, hill profile, and lap length.
#[derive(Debug, Clone, Copy)]
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

/// The Coast Run hill profile, moved verbatim from `runtime::track_geometry`
/// (the demo course's authored curve/elevation tables now live with the rest
/// of the course's data — no tuning change rode along).
const COAST_RUN_CURVES: &[CurveSegment] = &[
    CurveSegment {
        start: 200.0,
        length: 500.0,
        // Opening bend: winnable flat out under the corner-force
        // balance — a fight but a survivable one.
        curvature: -0.003,
    },
    CurveSegment {
        start: 700.0,
        length: 200.0,
        // Medium right out of the opening left: rideable flat with
        // a steady hand, but it punishes a lazy line.
        curvature: 0.009,
    },
    CurveSegment {
        start: 1_100.0,
        length: 140.0,
        // The Ridgeback right: a climbing hairpin — flat out, the
        // bend outlasts the grip; brake to the corner speed or run
        // off the tarmac.
        curvature: 0.016,
    },
    CurveSegment {
        start: 1_450.0,
        length: 140.0,
        // Mirror hairpin left, entered on the descent from the
        // crest: same bargain, opposite direction.
        curvature: -0.016,
    },
];

/// The Coast Run hill profile (see `COAST_RUN_CURVES`).
const COAST_RUN_ELEVATION: &[ElevationKey] = &[
    ElevationKey {
        distance: 0.0,
        elevation: 0.0,
    },
    ElevationKey {
        distance: 250.0,
        elevation: 0.0,
    },
    ElevationKey {
        distance: 450.0,
        elevation: 7.0,
    },
    ElevationKey {
        distance: 650.0,
        elevation: 14.0,
    },
    ElevationKey {
        distance: 900.0,
        elevation: 10.0,
    },
    ElevationKey {
        distance: 1_100.0,
        elevation: 2.0,
    },
    ElevationKey {
        distance: 1_400.0,
        elevation: 9.0,
    },
    ElevationKey {
        distance: 1_700.0,
        elevation: 3.0,
    },
    ElevationKey {
        distance: 2_000.0,
        elevation: 0.0,
    },
];

/// Every selectable course, in selection-UI order.
pub fn catalog() -> &'static [Course] {
    // OnceLock rather than a promoted const: the demo entry borrows the
    // landmark table through `course_landmarks()`, a function call, which
    // const promotion cannot evaluate.
    static CATALOG: std::sync::OnceLock<[Course; 2]> = std::sync::OnceLock::new();
    CATALOG
        .get_or_init(|| {
            [
                Course {
                    id: CourseId::CoastRun,
                    name: "Coast Run",
                    blurb: "The wave-7 demo lap: coast out of the grid, through the town, \
                            up the forest climb, along the ridge.",
                    layout: ZoneLayout::FOR_TRACK,
                    landmarks: crate::course_landmarks(),
                    curves: COAST_RUN_CURVES,
                    elevation: COAST_RUN_ELEVATION,
                    road_width: 12.0,
                },
                Course {
                    id: CourseId::IslandHop,
                    name: "Island Hop",
                    blurb: "A shorter, tighter island loop: out along the lagoon, through \
                            the village, under the jungle canopy, up to the summit crest.",
                    layout: ZoneLayout::from_zones(1_600.0, 80.0, ISLAND_HOP_ZONES),
                    landmarks: ISLAND_HOP_LANDMARKS,
                    curves: ISLAND_HOP_CURVES,
                    elevation: ISLAND_HOP_ELEVATION,
                    road_width: 12.0,
                },
            ]
        })
        .as_slice()
}

/// Resolves a course by id, for boot and for the shell's track-select flow.
pub fn for_course(id: CourseId) -> Option<&'static Course> {
    catalog().iter().find(|course| course.id == id)
}

// ---------------------------------------------------------------------------
// Island Hop — the wave-8 course. Authored here as data, bounded by the
// spec's tiling and lap-length contract (1 600 m, lagoon/village/jungle/
// summit, landmarks in the wave-7 `LandmarkKind` set).
// ---------------------------------------------------------------------------

/// Island Hop's zone table: the spec'd sequence tiles the 1 600 m lap
/// contiguously (lagoon 0/480, village 480/440, jungle 920/360, summit
/// 1280/320).
const ISLAND_HOP_ZONES: &[crate::geometry::Zone] = &[
    crate::geometry::Zone {
        name: "lagoon",
        start: 0.0,
        length: 480.0,
        grass_tint: [0.52, 0.58, 0.30, 1.0],
        sky_tint: [0.62, 0.84, 0.96, 1.0],
        backdrop_amp: 0.6,
        dressing: crate::geometry::DressingMix {
            palm: 0.55,
            tree: 0.10,
            building: 0.0,
            rock: 0.05,
            bush: 0.20,
            sign: 0.02,
            skip: 0.08,
        },
        fences: false,
        poles: false,
    },
    crate::geometry::Zone {
        name: "village",
        start: 480.0,
        length: 440.0,
        grass_tint: [0.36, 0.50, 0.26, 1.0],
        sky_tint: [0.66, 0.72, 0.78, 1.0],
        backdrop_amp: 0.9,
        dressing: crate::geometry::DressingMix {
            palm: 0.0,
            tree: 0.06,
            building: 0.36,
            rock: 0.0,
            bush: 0.10,
            sign: 0.26,
            skip: 0.22,
        },
        fences: true,
        poles: true,
    },
    crate::geometry::Zone {
        name: "jungle",
        start: 920.0,
        length: 360.0,
        grass_tint: [0.14, 0.36, 0.18, 1.0],
        sky_tint: [0.46, 0.62, 0.80, 1.0],
        backdrop_amp: 1.2,
        dressing: crate::geometry::DressingMix {
            palm: 0.10,
            tree: 0.70,
            building: 0.0,
            rock: 0.06,
            bush: 0.10,
            sign: 0.0,
            skip: 0.04,
        },
        fences: false,
        poles: false,
    },
    crate::geometry::Zone {
        name: "summit",
        start: 1_280.0,
        length: 320.0,
        grass_tint: [0.44, 0.46, 0.34, 1.0],
        sky_tint: [0.74, 0.82, 0.92, 1.0],
        backdrop_amp: 1.7,
        dressing: crate::geometry::DressingMix {
            palm: 0.0,
            tree: 0.08,
            building: 0.0,
            rock: 0.48,
            bush: 0.14,
            sign: 0.04,
            skip: 0.26,
        },
        fences: false,
        poles: false,
    },
];

/// Island Hop's landmarks, in the wave-7 `LandmarkKind` set: a windmill over
/// the lagoon, a water tower in the village, and a billboard cluster on the
/// summit run-in.
const ISLAND_HOP_LANDMARKS: &[Landmark] = &[
    Landmark {
        kind: LandmarkKind::Windmill,
        distance: 300.0,
        lateral: -16.0,
    },
    Landmark {
        kind: LandmarkKind::WaterTower,
        distance: 850.0,
        lateral: 18.0,
    },
    Landmark {
        kind: LandmarkKind::Billboards,
        distance: 1_420.0,
        lateral: -15.0,
    },
];

/// Island Hop's corner composition: a lagoon sweeper, a village kink, and a
/// jungle/summit hairpin pair steeper than Coast Run's — the shorter lap
/// pays for it with tighter bends.
const ISLAND_HOP_CURVES: &[CurveSegment] = &[
    CurveSegment {
        start: 140.0,
        length: 160.0,
        // Lagoon sweeper: an early right that sets the lap's rhythm.
        curvature: 0.011,
    },
    CurveSegment {
        start: 500.0,
        length: 120.0,
        // Village kink left: shallow, but the fences punish a wide exit.
        curvature: -0.007,
    },
    CurveSegment {
        start: 880.0,
        length: 130.0,
        // Jungle hairpin right: the lap's main stop — brake or run wide
        // into the trees.
        curvature: 0.018,
    },
    CurveSegment {
        start: 1_180.0,
        length: 120.0,
        // Summit hairpin left, climbing: mirror of the jungle bend on
        // the way up to the crest.
        curvature: -0.018,
    },
];

/// Island Hop's hill profile: flat lagoon, a village rise, a jungle dip, and
/// the summit crest at 18 m — back down toward the line to keep the lap
/// wrap from popping.
const ISLAND_HOP_ELEVATION: &[ElevationKey] = &[
    ElevationKey {
        distance: 0.0,
        elevation: 0.0,
    },
    ElevationKey {
        distance: 200.0,
        elevation: 0.0,
    },
    ElevationKey {
        distance: 430.0,
        elevation: 5.0,
    },
    ElevationKey {
        distance: 680.0,
        elevation: 11.0,
    },
    ElevationKey {
        distance: 900.0,
        elevation: 4.0,
    },
    ElevationKey {
        distance: 1_150.0,
        elevation: 12.0,
    },
    ElevationKey {
        distance: 1_420.0,
        elevation: 18.0,
    },
    ElevationKey {
        distance: 1_600.0,
        elevation: 6.0,
    },
];

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
