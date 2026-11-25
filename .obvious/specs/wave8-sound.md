# Wave 8.3 — Strategic sound design: Ninja H2/S engine synthesis

Status: SPEC (read-only on main code; this PR carries only this spec + failing
tests). Base: `wave-7-world` @ `b0b83d1`. Branch: `spec/wave8-sound`.

## Goal and decision to approve

**Decision:** replace the single-sawtooth engine tone with a **layered
synthesis graph modeled on the Kawasaki Ninja H2/S supercharged inline-4** —
inline-4 core throb + centrifugal supercharger whine — driven by
**RPM/throttle/load** (not speed alone), plus a complete **event cue set**
(crash, combat hits, recovery, zone transitions) with locked mix budgets. The
reader should approve the layer model, the input contract with the Wave 8.2
drivetrain spec, and the two numeric budgets (loudness, harshness band).

Why strategic, not piecemeal: PRs #19 and #23 patched one oscillator's
filter/LFO chain in place. Each fix moved one curve. This spec designs the
whole system at once — layers, parameter mapping, cues, mix policy, mute —
so later tuning touches mappings, not structure.

## Current state and evidence

- **`web/audio.js`** (289 lines, b0b83d1): one sawtooth oscillator through a
  speed-tightening lowpass + highshelf/lowshelf, two LFOs (breathing,
  firing-pulse) and a pitch-drift LFO. All three mapping curves —
  `engineFreq`, `engineCutoff`, `engineBaseGain` — are keyed on **speed alone**
  (`engineFreq(speedKmh)`, L189-191). Cues: crash (crashed-bit rising edge),
  hit (stamina drop > 0.08/frame), countdown beeps + green chirp.
  `diagnostics()` reports state/engineFreq/engineCutoff/muted; `bandEnergy`
  exists for probes. *Changed the design:* the layer graph replaces the
  single osc; the speed-only mappings are the core defect the RPM input fixes;
  the existing analyser tap and diagnostics surface are kept and extended.
- **`web/loader.js`** (L125-130, L247, L393-402): `audio` is module-scope;
  `unlock()` rides the CLICK TO RIDE engagement gestures; the mute button
  toggles `setMuted`; the 60 Hz loop calls `audio.update({ speedKmh, state,
  stamina, countingDown, countdownSecs, paused, finished })` — the only call
  site. *Changed the design:* new inputs (`rpm`, `throttle`, `zone`) ride the
  same single `update()` call; loader keeps its ownership of sim-state
  polling.
- **`crates/web/src/lib.rs`** (L280-343): glue exposes `speed_kmh()`,
  `rider_state()`, `player_stamina()`. There is **no gear/RPM getter yet** —
  that is Wave 8.2's drivetrain deliverable. *Changed the design:* the audio
  input contract accepts `rpm: number | null` and falls back to a
  speed-derived estimate until 8.2 ships the getter, so audio never blocks on
  8.2 and never reads crates/ itself.
- **`crates/game/src/bike.rs`** (L73-90): `LAUNCH_SPEED` 33 m/s (~119 km/h)
  launch-to-top-gear shift point with a `SHIFT_ACCEL` tail; `Control` enum is
  `Coast | Accelerate | Brake | Crashed`. No RPM model exists in sim today.
  *Changed the design:* the audio `throttle` field mirrors `Control`'s
  vocabulary (`'coast' | 'accelerate' | 'brake'`; crashed state arrives via
  the rider-state bitmask), and the ~119 km/h shift point is where the 8.2
  RPM bands will dip — the audio must follow that dip down and up (the
  audible "shift").
- **README** (Gate section): `cargo fmt --check && cargo clippy --all-targets
  -- -D warnings && cargo test --all-targets`. **`docs/perf-baselines.md`**:
  prior audio verification ran as out-of-repo headless-Chrome probes.
  *Changed the design:* the web gates stay rust-side and untouched; the
  committed harness is dependency-free Node (`node --test web/tests/`), so it
  runs anywhere the repo clones — including this sandbox, which has Node
  20.20.2 and no browser.
- **Calibration measurements (this session, `node calibrate.mjs` against
  b0b83d1)**: rendered output RMS across the speed sweep — 0 km/h: 0.01685,
  45: 0.02189, 90: 0.02638, 135: 0.03018, **180: 0.03333**; 1–3 kHz band RMS
  at 180 km/h full throttle: **0.001026**. Harness self-check: a known
  1 kHz unit sine reads 0.7052 band RMS (expected 0.7071). *Changed the
  design:* the two mix budgets below are set just under today's measured
  peaks — the layered graph must rebalance within a louder-single-layer's
  headroom, which is the strategic discipline "no harshness, no static
  drone" translates to when you cannot master by ear in CI.

Outside knowledge (from training data, not checked this session, used only
for timbre direction): the Ninja H2/S is a 998 cc supercharged inline-4;
Kawasaki's centrifugal supercharger gives it a characteristic high-pitched
whine that builds with revs on top of the inline-4's even-firing throb. No
numeric claims in this spec rest on it.

## What we build

All synthesis lives in **`web/audio.js` and new `web/` audio files only**.
No `crates/` changes; camera files untouched.

### 1. Synthesis graph (replaces `startEngine()`'s single osc)

- **Layer `core` — inline-4 throb.** Sawtooth oscillator (the existing one),
  frequency set from **RPM** (mapping below). The existing firing-pulse LFO
  (rate tracks layer frequency) and breathing LFO stay, attached to this
  layer, so the tone never sits still (PR #23's anti-drone work is
  preserved, now per-layer).
- **Layer `supercharger` — whine.** A second oscillator at a fixed harmonic
  **order 8** of the core (allowed 6–10), landing ~1–2 kHz at speed. Its gain
  is small (sub-mix; budget-bounded) and **rises with load and RPM** — a
  centrifugal supercharger makes boost with revs under throttle.
- **Shared tone stack** (kept from PR #23): speed-tightening lowpass,
  −7.5 dB highshelf @ 1.1 kHz, +2 dB lowshelf @ 160 Hz — with one change:
  the lowpass cutoff also opens under throttle (load-dependent timbre).
- **Master**: unchanged 0.5 gain to destination; every layer flows through
  it, so **mute behavior is exactly PR #19's** (`setMuted` → 0, 0.02 s
  smoothing; cues and engine both silence).

### 2. Parameter mapping (per-frame, from the same `update()` call)

Input contract — `update()` gains three optional fields (unknown fields are
ignored today, so this is additive and backward-compatible):

| Field | Type | Source | Fallback when absent |
|---|---|---|---|
| `rpm` | `number \| null` | 8.2 drivetrain glue getter (name TBD by 8.2) | speed-derived estimate; tests always pass explicit `rpm` |
| `throttle` | `'coast' \| 'accelerate' \| 'brake'` | mirrors `bike::Control` via glue | `'coast'` |
| `zone` | `string` (zone id, already derivable HUD-side) | loader zone logic | no zone cues |

Mappings (spec-locked):

- **Core pitch follows RPM, not speed:**
  `coreFreq(rpm) = 40 + 170 · min(rpm, 12000) / 12000` Hz (idle 40 Hz →
  210 Hz at 12k). When `rpm` is provided, speed does **not** move pitch —
  gear changes become audible dips. When `rpm` is `null`, pitch falls back to
  today's `engineFreq(speedKmh)`.
- **Load timbre:** `cutoff(speed, throttle) = engineCutoff(speed) +
  (throttle === 'accelerate' ? 250 : 0)` Hz — full throttle opens the filter
  ≥150 Hz at fixed speed; coasting darkens.
- **Whine level:** rises with both rpm and throttle (≥20% louder at full
  throttle vs coast at fixed rpm), total mix held under the budgets below.
- **Loudness budget (locked):** rendered output RMS ≤ **0.030** sustained at
  every speed 0–180 km/h (today's chain measures 0.0333 max).
- **Harshness budget (locked):** 1–3 kHz band RMS ≤ **0.0009** at 180 km/h
  full throttle (today: 0.001026) — the whine layer must live below this
  ceiling, enforced by mixing it low and under the highshelf.

### 3. Event cue set

Edge-detected in `update()`, exactly once per event, never per frame; a
per-name refractory of 150 ms guards against double-fires:

| Cue | Trigger | Sound |
|---|---|---|
| `crash` | crashed bit rising edge (exists today) | filtered noise burst (kept) |
| `hit` | stamina drop > 0.08/frame (exists today) | 110 Hz thud (kept) |
| `recovery` | crashed bit **clearing** after a crash — the rider picks the bike up (new) | short rising chirp |
| `zone` | zone id change between frames (new) | soft two-note marker |
| `countdown` / `green` | countdown second change / green light (kept) | 440 Hz beep / 880 Hz chirp |

### 4. Probe surface (what the tests assert)

`RaceAudio.diagnostics()` gains:
- `layers`: array of `{ name, freq, gain }` — at least `core` and
  `supercharger`;
- `cues`: ordered log of `{ name, at }` for every cue fired since unlock.

Both are additive to the existing diagnostics object. The `analyser` tap and
`bandEnergy` stay for browser-side probes.

## Verification — the failing acceptance tests

Committed on this branch under `web/tests/` (dependency-free,
`npm test` or `node --test web/tests/`; `web/package.json` exists only to
make `web/*.js` ESM for Node and carry the test script). The harness renders
the actual node graph offline (44.1 kHz, RBJ biquads, `OfflineAudioContext`-
shaped probe in `helpers/test-context.mjs`) — measurements, not mock
inspection. **Every listed test fails today for the stated reason; none
passes, none errors on import or collection.**

| Test (file) | Asserts | Verdict today | Failing assertion (measured) |
|---|---|---|---|
| T1 `audio.mapping.test.mjs` | Core pitch = f(rpm) at fixed speed; pitch does not move when speed changes at fixed rpm | **FAIL** | `diagnostics().layers` has no `core` layer — no RPM input exists in `web/audio.js` |
| T2 `audio.mapping.test.mjs` | Whine layer exists at 6–10× core order; ≥20% louder under throttle | **FAIL** | no `supercharger` layer |
| T3 `audio.mapping.test.mjs` | Lowpass cutoff opens ≥150 Hz under throttle at fixed speed | **FAIL** | cutoff 1010 Hz (throttle) = 1010 Hz (coast) — mapping is speed-only |
| T4 `audio.cues.test.mjs` | Exactly 1 `recovery` cue per crash recovery; 1 `crash` cue at the rising edge | **FAIL** | cue log absent — 0 cues ≠ 1 |
| T5 `audio.cues.test.mjs` | Exactly 1 `zone` cue per zone crossing, held frames add none | **FAIL** | cue log absent — 0 cues ≠ 1 |
| T6 `audio.render.test.mjs` | Rendered RMS ≤ 0.030 at every speed | **FAIL** | 0.0333 RMS at 180 km/h > 0.030 budget |
| T7 `audio.render.test.mjs` | 1–3 kHz band RMS ≤ 0.0009 at 180 km/h full throttle | **FAIL** | 0.001026 > 0.0009 budget |

Implementation-phase checks (same suite once implemented): all seven pass;
`cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
cargo test --all-targets` stays green (no rust changes); both wasm builds and
the 2 MB gzip budget gate stay green per repo convention. A real-browser
`OfflineAudioContext` cross-check of T6/T7 is recommended at implementation
time (the committed harness is the Node-render equivalent, validated to 0.2%
against a known sine).

## Delivery

One PR from `spec/wave8-sound` into `wave-7-world`, containing **only** the
spec (`.obvious/specs/wave8-sound.md`), the failing tests
(`web/tests/**`), and `web/package.json` (ESM for the harness). No
implementation. The 8.3 implementation PR follows against `wave-7-world`,
turning T1–T7 green.

## Locked decisions

- **File ownership:** `web/audio.js` + new `web/` audio files only; no
  `crates/sim` (bike.rs, race.rs), no camera files, no wasm glue changes in
  this wave's audio work (the `rpm` getter lands with 8.2).
- **Layer model:** `core` (inline-4 throb, RPM-pitched) + `supercharger`
  (whine, order 8, allowed 6–10) + the PR #23 tone stack, under a single
  master with unchanged mute semantics.
- **Input contract:** `update()` accepts `rpm`, `throttle`, `zone`;
  `rpm: null` falls back to speed-derived pitch so audio never blocks on 8.2.
- **Budgets:** rendered RMS ≤ 0.030 at every speed; 1–3 kHz band RMS
  ≤ 0.0009 at top speed; measured via the committed offline-render probe.
- **Cue semantics:** once per event, edge-detected, 150 ms per-name
  refractory; probe surface `diagnostics().cues`.
- **Non-goals:** no audio recordings/samples (pure synthesis), no music, no
  positional audio, no changes to sim or camera.

## Open questions

None for this phase. The one external dependency — the name of 8.2's
glue getter for crankshaft RPM — does not block: the audio contract takes a
plain `rpm` number and the loader wires it whenever 8.2 lands.
