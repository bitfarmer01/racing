# Wave 8.1 — Multiple tracks on the 7c zone system

**Status:** spec for review — implementation must not start from anything but this file.
**Base:** `wave-7-world` (b0b83d1). This branch carries the spec and the failing acceptance tests only.

## Problem frame

The game has exactly one course. Every lap of every race rides the same 2 000 m
coast → town → forest → ridge loop, and the web shell hardcodes that loop's
presentation. The product goal for wave 8 is **multiple distinct courses built on
the wave-7c zone system, plus a track-selection UI in the web shell**.

**Chosen direction:** courses become *pure data* entries in a course catalog in
`crates/world`, each carrying its own zone table, landmarks, corner composition,
hill profile, and lap length. The wave-7 demo course becomes the catalog's first
entry; a second, structurally different course ("Island Hop") is added beside it.
The web shell grows a track-select control on the boot overlay; the selected
course id flows into wasm boot, and the HUD's lap bar and zone markers follow the
selected course. The renderer needs no changes: the frame builder already renders
whatever `ZoneLayout` it is handed through `sample()` (evidence below), so a
second course with a different zone table renders as a different place with zero
new render code.

**What needs approval:** the course-definition format (below), the two-course
roster for wave 8, and boot-time selection (no mid-race course switching).

**Non-goals:** per-course rival AI tuning, new landmark sprite kinds (the wave-7
`LandmarkKind` set is reused), mid-race track switching, course unlocking or
persistence, and any change to simulation behavior (`crates/game/src/bike.rs`,
`crates/game/src/race.rs`) or audio (`web/audio.js`).

## Current state and evidence

I read:

- `crates/world/src/geometry.rs` — the 7c zone system. `ZoneLayout` holds
  `lap_length`, a private `blend` window, and a private `zones: &'static [Zone]`
  slice; the only instance is the hardcoded `FOR_TRACK` const behind
  `for_track()`. `sample(distance)` resolves and blends the zone look per
  distance; `course_landmarks()` is a free function returning the demo course's
  three landmarks; `pick_dressing` and `RoadGeometry` are fully generic over any
  data. **What this changed:** the zone/landmark/dressing machinery is already
  course-agnostic — the missing piece is a registry of courses, not new zone
  mechanics. `ZoneLayout`'s private fields also mean a catalog must live in the
  same crate to construct layouts from data.
- `crates/runtime/src/frame.rs` — `track_geometry()` (lines ~560–622) hardcodes
  the demo course's curve segments (opening left, medium right, the two 0.016
  hairpins) and elevation keys; `demo_race()` (~626) wires them into
  `RaceConfig` with `track_length: LAP_LENGTH` (const 2 000, line 125). The
  frame builder calls `world::ZoneLayout::for_track()` at five call sites
  (lines 926, 1247, 1484, 1570, 1618) and renders landmarks from
  `world::course_landmarks()` (1486–1490). **What this changed:** the runtime
  consumes zones only through `sample()` and the landmark list, so course
  selection is a data-plumbing change in `demo_race`/`track_geometry`, not a
  renderer change. The demo course's corner/elevation tables should move into
  the catalog so runtime becomes course-agnostic.
- `crates/web/src/lib.rs` — `BrowserGame::boot(canvas, low_end_device)` boots
  "the demo race" via `demo_race()` with no course parameter (lines ~178–210);
  `lap_length()` (line 317) already forwards `race.lap_length()` to the HUD.
  **What this changed:** boot-time selection needs a course id on the
  wasm-boundary — `boot` gains one parameter, and the shell reads it before
  booting. Mid-race switching would need far more glue and is excluded.
- `web/loader.js` — boots wasm, appends zone markers from
  `buildZoneMarkers(DEMO_ZONES, game.lap_length())` (line 179), and highlights
  the current zone per frame with `highlightZone(zoneMarkers, DEMO_ZONES, ...)`
  (line ~381). **What this changed:** the shell's zone presentation is already
  parameterized by a zone table + lap length — selection only needs to swap
  which table feeds those two calls.
- `web/zones.js` — `DEMO_ZONES` is a hand-duplicated presentation slice of
  `ZoneLayout::FOR_TRACK` (name + start/end in meters), with an in-file warning
  that drift between the two cannot be caught by build gates. **What this
  changed:** adding per-course tables as more hand-copied constants multiplies
  that drift risk; the catalog should expose course zone data to the shell
  through one exported per-course catalog so a course's markers come from one
  place.
- `web/index.html` — boot overlay (`#overlay` with `#status`), HUD strip, lap
  bar (`#lap-bar`, `#lap-track`, `#zone-markers`), no track-select control
  exists. **What this changed:** the selection UI is a new control on the
  existing overlay, styled with the shell's existing overlay/glass idioms.
- `web/badge.js`, `web/race-timer.js` — precedent for extracting pure,
  headless-probe-assertable modules in the shell. **What this changed:** the new
  shell acceptance tests follow the same pattern (plain ES modules asserted by
  `node --test`, no DOM needed for the catalog contract).

## What we should build

**Decision this section carries: a pure-data course catalog in `crates/world`
with two courses at wave 8, boot-time selection in the web shell, and HUD/zone
markers that follow the selected course.**

### Course definition format

New module `crates/world/src/courses.rs`, re-exported from `crates/world/src/lib.rs`:

```rust
/// Stable selector for a course, mirrored in the web shell's track-select UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CourseId { CoastRun, IslandHop }

/// One selectable course: pure data end to end.
pub struct Course {
    pub id: CourseId,
    pub name: &'static str,
    pub blurb: &'static str,
    pub layout: ZoneLayout,           // zone table + lap_length + blend
    pub landmarks: &'static [Landmark],
    pub curves: &'static [CurveSegment],
    pub elevation: &'static [ElevationKey],
    pub road_width: f32,
}

/// Every selectable course, in selection-UI order.
pub fn catalog() -> &'static [Course];

/// Resolves a course by id for boot and selection.
pub fn for_course(id: CourseId) -> Option<&'static Course>;
```

`ZoneLayout` gains a data-facing constructor (crate-visible or `pub`) so a
course's zone table builds from plain `Zone` values; `for_track()` remains as an
alias for the demo course so existing call sites keep compiling until the
runtime moves over. A course's `RoadGeometry` is built from its `curves`,
`elevation`, and `road_width` — the same `RoadGeometry::new` the demo uses.

### The courses

| id | name | lap | zones (start/length m) | landmarks |
|---|---|---|---|---|
| `CoastRun` | Coast Run | 2 000 | coast 0/520, town 520/560, forest 1080/520, ridge 1600/400 (today's `FOR_TRACK`) | today's three (water tower 380, windmill 940, billboards 1540) |
| `IslandHop` | Island Hop | 1 600 | lagoon 0/480, village 480/440, jungle 920/360, summit 1280/320 | two or three, fixed per-lap distances, in the `LandmarkKind` set |

Island Hop's corner and hill composition (its own hairpins and crest, distinct
from Coast Run's) is authored as data in the catalog during implementation; the
spec fixes only its lap length, zone sequence, and the tiling contract, so the
acceptance tests stay honest without predesigning every curve. The demo
course's corner/elevation tables move verbatim from `runtime::track_geometry`
into its catalog entry — no tuning changes ride along.

### Track-select flow (web shell)

1. `web/index.html`: the boot overlay gains a `<select id="track-select">`
   (labelled, styled with the overlay's existing idioms) listing every catalog
   course by display name, value = the course id string.
2. `web/loader.js`: reads the control's value before boot and passes it to the
   wasm boundary — `BrowserGame::boot(canvas, low_end_device, course_id)`
   (`crates/web/src/lib.rs` maps the id string to `CourseId`; an unknown id is
   a boot error on the loading screen, same path as today's boot failures).
3. The catalog course's `layout.lap_length` reaches the race via
   `RaceConfig::track_length`, so laps, progress, and finish all measure the
   selected course. `demo_race()` becomes course-aware (`demo_race_for(course)`)
   in `crates/runtime/src/frame.rs`, replacing the five `for_track()` call
   sites with the selected course's layout.
4. The restart path keeps the selected course (restart re-boots the same
   course, never falls back to the demo).

### HUD integration

- Zone markers: `web/zones.js` replaces the single hand-copied `DEMO_ZONES`
  with a per-course catalog export (`COURSES`, one entry per course: id, lap
  length, zone table). `buildZoneMarkers` and `highlightZone` stay unchanged —
  they already take the table as a parameter. The loader builds markers and
  highlights from the selected course's entry, retiring the demo-only
  duplication (and its drift warning).
- Lap display: `game.lap_length()` already reports the race's track length, so
  the `lap x of N · d / L m` read is per-course with no HUD changes.
- The zone name lit on the lap bar is the selected course's own zone sequence —
  lagoon/village/jungle/summit on Island Hop, coast/town/forest/ridge on Coast
  Run.

### Verification — named failing tests

These tests are committed on this branch and **fail on `wave-7-world` for
exactly the reason given; each passes unmodified once the feature is
implemented.** None fails on compile/import — a red-state catalog skeleton
(`courses.rs` with the API surface and only the demo course registered) makes
the Rust tests compile so they fail at assertion.

| id | test (file · name) | red on wave-7-world because | green when |
|---|---|---|---|
| T1 | `crates/world/src/courses.rs` · `wave8_catalog_lists_two_or_more_distinct_courses` | the catalog registers only the demo course | the catalog lists ≥ 2 courses with distinct ids |
| T2 | `crates/world/src/courses.rs` · `wave8_a_second_course_defines_its_own_zone_sequence` | `for_course(IslandHop)` is `None`; no second zone sequence exists | Island Hop parses, its zone sequence is its own (lagoon/village/jungle/summit), and mid-zone samples resolve to its zones |
| T3 | `crates/world/src/courses.rs` · `wave8_every_course_tiles_its_lap_and_carries_its_specd_lap_length` | Island Hop is absent, so its lap (1 600 m), contiguous zone tiling, and in-lap landmarks cannot check out | every spec'd course's zones tile contiguously, sum to its lap length, and its landmarks sit inside the lap |
| T4 | `crates/world/src/courses.rs` · `wave8_course_lookup_resolves_distinct_courses_for_distinct_ids` | the second id resolves to `None` | distinct ids resolve to distinct courses (different lap lengths, different zone sequences) — the selection semantic the shell consumes |
| T5 | `web/tracks.test.mjs` · "the shell exposes a per-course zone catalog" | `zones.js` exports no per-course `COURSES` catalog | the export exists with both courses, correct lap lengths and zone sequences, feeding `buildZoneMarkers`/`highlightZone` unchanged |
| T6 | `web/tracks.test.mjs` · "the shell offers a track-select control" | no `track-select` control in `web/index.html`, no reference in `web/loader.js` | the overlay carries the control and the loader reads its value to choose the booted course |

Gates: `cargo fmt --check`, `cargo clippy --workspace` clean, existing suite
stays green (`cargo test --workspace`), shell tests via `node --test
web/tracks.test.mjs` (the two wave-8 tests red, nothing else). Boot-time wasm
glue (`BrowserGame::boot`) is not unit-testable headless — it needs a canvas and
GPU context (`crates/web/src/lib.rs` boot) — so its contract is covered by T6 on
the shell side and the spec text, and verified in the wave-8 implementation's
browser playtest.

### Delivery

One PR against `wave-7-world` containing the spec, the failing tests, and the
minimal red-state catalog skeleton. The wave-8 implementation follows as its own
PR(s) making these tests pass.

## Locked decisions

- Courses are pure, compile-time data in `crates/world::courses`; no runtime
  mutation, no asset loading, no per-frame state.
- Wave 8 ships exactly two courses: Coast Run (the demo, relocated into the
  catalog unchanged) and Island Hop (1 600 m, lagoon/village/jungle/summit).
- Selection happens at boot only; no mid-race course switching. Restart keeps
  the selected course.
- The renderer and zone-blend machinery are reused unchanged; a new course is
  data, never render code.
- `crates/game/src/bike.rs`, `crates/game/src/race.rs`, and `web/audio.js` are
  out of bounds for this workstream.

**Open questions:** none — the second course's exact corner/elevation tuning is
deliberately left to the implementation, bounded by the tiling and lap-length
contract above.
