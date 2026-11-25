//! The circuit catalogue.

use crate::theme::{ALPINE_LAKE, DUSK_CANYON, GOLDEN_COAST};
use crate::track::TrackDef;

const GOLDEN_COAST_POINTS: &[[f32; 2]] = &[
    [-320.0, -210.0],
    [60.0, -238.0],
    [330.0, -205.0],
    [455.0, -95.0],
    [425.0, 55.0],
    [290.0, 118.0],
    [228.0, 232.0],
    [300.0, 342.0],
    [236.0, 452.0],
    [88.0, 470.0],
    [2.0, 372.0],
    [-108.0, 302.0],
    [-238.0, 352.0],
    [-382.0, 292.0],
    [-462.0, 140.0],
    [-452.0, -62.0],
];

const ALPINE_LAKE_POINTS: &[[f32; 2]] = &[
    [0.0, -335.0],
    [232.0, -302.0],
    [382.0, -182.0],
    [432.0, 12.0],
    [362.0, 168.0],
    [418.0, 318.0],
    [298.0, 452.0],
    [118.0, 402.0],
    [-18.0, 298.0],
    [-168.0, 362.0],
    [-332.0, 302.0],
    [-422.0, 128.0],
    [-382.0, -62.0],
    [-262.0, -232.0],
];

const DUSK_CANYON_POINTS: &[[f32; 2]] = &[
    [-400.0, -100.0],
    [-150.0, -262.0],
    [150.0, -282.0],
    [382.0, -182.0],
    [302.0, -22.0],
    [422.0, 142.0],
    [332.0, 332.0],
    [102.0, 302.0],
    [2.0, 162.0],
    [-128.0, 322.0],
    [-332.0, 302.0],
    [-262.0, 122.0],
];

pub const CATALOG: [TrackDef; 3] = [
    TrackDef {
        slug: "golden-coast",
        name: "Golden Coast",
        blurb: "Sunset run along the sea cliffs. Long straight, fast sweepers, one nasty hairpin.",
        points: GOLDEN_COAST_POINTS,
        half_width: 7.0,
        laps: 3,
        theme: GOLDEN_COAST,
    },
    TrackDef {
        slug: "alpine-lake",
        name: "Alpine Lake",
        blurb: "Clear afternoon around a mountain lake. Flowing esses under the peaks.",
        points: ALPINE_LAKE_POINTS,
        half_width: 6.5,
        laps: 3,
        theme: ALPINE_LAKE,
    },
    TrackDef {
        slug: "dusk-canyon",
        name: "Dusk Canyon",
        blurb: "Red rock and street lamps at last light. Tight, technical, unforgiving.",
        points: DUSK_CANYON_POINTS,
        half_width: 6.5,
        laps: 3,
        theme: DUSK_CANYON,
    },
];

pub fn catalog() -> &'static [TrackDef] {
    &CATALOG
}

pub fn by_slug(slug: &str) -> Option<TrackDef> {
    CATALOG.iter().copied().find(|d| d.slug == slug)
}
