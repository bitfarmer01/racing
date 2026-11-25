# Wave 8.2 — Drivetrain physics: gearbox, RPM bands, torque curves

Status: spec + failing tests committed on `task/8-2-spec-drivetrain-physics-spec-failing-ejWHjQdo` (PR against `wave-7-world` @ `b0b83d1`). Implementation is queued behind the hairpin fix (`fix/hairpin-framing`, PR #26) merging into `wave-7-world`.

## Problem frame

The bike accelerates on a single-ratio thrust curve: a launch pull (`ACCEL = 18 m/s²`, bike.rs:23) tapering linearly to `SHIFT_ACCEL = 5 m/s²` past `LAUNCH_SPEED = 33 m/s` (bike.rs:73–75, bike.rs:218–224). It reads as one long gear: no shift points, no rev-band rhythm, a 0–100 km/h time of 2.38 s that feels like a scale model, and no engine readout to sell speed. The Wave 8 goal is an expressive drivetrain: a multi-gear box with RPM bands and a torque-shaped pull that replaces the single-ratio thrust while leaving top speed, cornering, and off-road behaviour untouched.

Decision requested: approve the gearbox table, torque shape, auto-shift policy, and the HUD surface below.

## Current state and evidence

- **`crates/game/src/bike.rs`** — owns the thrust model. `step_bike` (bike.rs:198–246) is a pure per-tick state machine; the pull is the piecewise launch/tail formula (bike.rs:218–224). Its test module already carries a red-first precedent (the Wave 7 audit block, bike.rs:786+). *Changed the design: the gearbox lives here, as a pure function of speed; `step_bike`'s signature does not change.*
- **`crates/game/src/race.rs`** — calls `step_bike` once per tick (race.rs:610) and uses `bike::ACCEL` directly for the rival rubber-band boost (race.rs:634). *Changed the design: `ACCEL` stays exported and keeps its meaning as the low-speed launch pull, so rival pacing is untouched by the gearbox.*
- **`crates/sim/src/clock.rs`** — fixed 60 Hz tick, `TICK_DURATION_SECS` (clock.rs:11–14). *Changed the design: the drivetrain is stepped per tick with no sub-stepping; the shift discontinuity lands on a tick boundary by construction.*
- **`crates/sim` generally (input.rs, pool.rs, rng.rs)** — plumbing only; `game` depends on `sim` (bike.rs uses `sim::clock`), never the reverse. *Changed the design: the spec tests live in `crates/game/src/bike.rs`'s test module — a `crates/sim` test module cannot see game code. This corrects the task's shorthand; ownership rules (no `camera.rs`, no `web/audio.js`, tests only in test modules) are honoured.*
- **`crates/web/src/lib.rs`** — the DOM HUD reads getters (lib.rs:270–322); `speed_kmh()` at lib.rs:280–282 is the pattern to follow. *Changed the design: the gear/RPM readout ships as two new getters here, not in `crates/runtime/src/frame.rs` — frame.rs composes sprites, the HUD data flows through the web shell.*
- **`web/audio.js`** — engine tone tracks the km/h figure the HUD renders (audio.js:1–2). *Untouched per ownership. The RPM signal this spec exposes is the future seam for rpm-tracked synthesis.*
- **`crates/game/src/input.rs`** — `GameInput`/`Intent` fold per tick. *Changed the design: no new input variants — the gearbox is automatic (see non-goals).*

## What we build

A pure per-speed drivetrain inside `bike.rs`, stepped by `step_bike`:

- **Six gears.** Band tops in m/s at the 60 Hz tick: **14, 22, 30, 39, 49, 60** (top gear tops at `MAX_SPEED`).
- **Per-gear peak pulls** (m/s²): **20.0, 16.5, 13.5, 10.5, 7.5**, and top gear carries the existing top-speed tail (`SHIFT_ACCEL`-shaped, → 0 at 60) so top speed and end-of-run pacing are unchanged.
- **Torque curve shape.** Within a gear's band, normalized engine speed `x ∈ [0, 1)`; torque factor `t(x) = 1 − 0.45·max(0, (x − 0.35)/0.65)²` — flat to 35% of the band, then falling quadratically to 55% of peak at redline. Wheel pull `a(v) = P_g · t(x)`.
- **RPM bands.** Redline 14,000 RPM (Ninja H2/S-inspired, matching the audio design intent in audio.js's header). Each gear's displayed RPM sweeps ~40% → 100% of redline across its band; `rpm` is carried as a 0–1 band fraction.
- **Shift handling.** Automatic: upshift when the engine reaches redline under throttle; downshift when braking/coasting drops engine speed below the band floor. Recovery after a crash starts in the gear whose band covers the current speed (design against the hairpin fix: PR #26 recovers riders at ≤ half max speed 3 m inside the road edge, so the box re-enters in gears 1–3; the off-road cap `OFFROAD_MAX_SPEED = 18 m/s` keeps dirt riding inside gears 1–2). `step_bike`'s signature and every grip/drift constant (`CORNER_FORCE`, `CORNER_DRIFT_FRACTION`, `FULL_LOCK_HEADING`) stay exactly as they are.
- **State.** `BikeSnapshot` gains `gear: u8` and `rpm: f32` (band fraction), `Default` = gear 1, rpm at band floor. Rivals share the same model through the same `step_bike`.
- **HUD readout.** New `gear()` and `rpm_percent()` getters in `crates/web/src/lib.rs` beside `speed_kmh()`, rendered by the browser shell next to the speed figure. `web/audio.js` is not modified this wave.

## UI

The HUD gain is a readout, not a control: gear number and RPM percentage rendered beside the existing km/h figure in the browser shell's HUD strip, using the same getter-per-figure pattern as `speed_kmh()` (lib.rs:280). No new screens, no settings.

## Non-goals

Manual shift inputs (clutch/shift keys) — automatic only this wave; engine-audio re-synthesis on the RPM signal; traction/wheelie modelling; changes to any camera file.

## Verification

Committed red-first in `crates/game/src/bike.rs`'s test module (they compile on the base, fail at assertion, and pass once the gearbox ships):

| Test | Fails today because | Passes when |
|---|---|---|
| `full_throttle_acceleration_rises_through_distinct_gear_bands` | "shifts at []" — the single-ratio thrust only tapers, the accel profile never recovers | ≥ 2 shift recoveries appear in the full-throttle accel profile (≥ 3 gears) |
| `gear_shifts_occur_at_speed_boundaries_not_at_the_extremes` | "nothing shifts on the single-ratio thrust" | Every detected shift lands in 12–50 m/s: no shifts at a crawl, none in the top-gear tail |
| `full_throttle_reaches_100_kmh_within_the_gearbox_launch_target` | "0-100 km/h in **2.38 s** misses the 2.2 s gearbox launch target" | First gear's 20 m/s² peak pull brings 0–100 km/h under 2.2 s |

Verdicts on the base commit (`b0b83d1`): 81 passed, 3 failed (only the three above), workspace otherwise green; `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` green.

Regression guards already green (they must stay green): `accelerating_from_rest_gains_speed_and_caps_at_max` (top speed unchanged at `MAX_SPEED`), `off_road_throttle_bleeds_speed_down_to_the_dirt_cap`.

## Delivery

One PR against `wave-7-world` containing the spec file (`.obvious/specs/wave8-drivetrain.md`) and the three failing tests — nothing else. Implementation is a later task, queued behind PR #26 (`fix/hairpin-framing`) merging into `wave-7-world`; its bounded-excursion/recovery outcome in `bike.rs`/`race.rs` is the state this design assumes. The red tests are appended at the end of the test module, clear of #26's production changes.

<locked-decisions>

- Automatic shifting only; no manual shift input this wave.
- Six gears; band tops 14/22/30/39/49/60 m/s; top speed stays `MAX_SPEED = 60` — guarded by an existing green test, never re-tuned.
- `step_bike`'s signature is unchanged; the gearbox is a pure function of speed (plus a small shift state on `BikeSnapshot`).
- `bike::ACCEL` stays exported with launch-pull meaning so the rival rubber-band boost (race.rs:634) is unaffected.
- The spec tests live in `crates/game/src/bike.rs`'s test module — `crates/sim` is plumbing and cannot see game code (evidence above); `camera.rs` and `web/audio.js` are untouched.
- No grip/drift/wind-up model changes: corner-force constants and heading steering are exactly as shipped in Wave 7.

</locked-decisions>
