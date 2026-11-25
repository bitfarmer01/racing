# racing

A 3D motorcycle combat racer for the browser: eight riders, three laps,
three tracks, boost, slipstream and fists. Rust and wgpu compiled to
WebAssembly, rendered through WebGPU with a WebGL2 fallback.

![Alpine Lake](docs/screenshots/alpine-lake.jpg)

## Play

```
python3 -m http.server
```

Open http://localhost:8000/web/. The built bundles in `web/pkg-webgpu` and
`web/pkg-webgl2` are committed, so no build is needed to play. Add
`?backend=webgl2` to force the fallback renderer.

| Action | Keyboard | Gamepad |
|---|---|---|
| Throttle | W / ↑ | RT or A |
| Brake / reverse | S / ↓ | LT |
| Steer | A D / ← → | left stick |
| Boost | Shift or Space | LB / RB |
| Punch | X | X or B |
| Camera | C | Y |
| Pause | Esc or P | Start |
| Restart | R | |

## How it plays

- **Goal:** finish three laps ahead of seven rivals. Position, lap, lap time,
  best lap, a minimap and a results table track the race.
- **Handling:** speed-dependent steering with lean, tyre grip that lets go if
  you carry too much speed into a corner, braking, grass that slows you down
  and Armco barriers that crash you if you hit them hard.
- **Boost** drains while held and refills slowly; riding in a rival's wake
  gives a slipstream tow.
- **Combat:** punch a rider alongside to unsettle them; enough hits knock them
  off. Crashed riders get back on after a couple of seconds.
- **Rivals** race the same physics with their own lines, skill and aggression.

## Layout

- `crates/sim` — fixed-tick clock, deterministic RNG, small pools
- `crates/game` — tracks (spline centreline, banking, elevation, terrain),
  bike physics, race rules, AI rivals, scenery placement
- `crates/render` — the wgpu renderer: HDR scene with sun shadows, PBR-ish
  shading, sky and fog, water, instanced scenery with LODs, particles,
  bloom, tone mapping, colour grade and FXAA, with dynamic resolution
- `crates/runtime` — the session the browser and native tools share: fixed
  120 Hz simulation with interpolation, chase camera, effects, HUD snapshot;
  `shot` renders screenshots and frame-time benches natively
- `crates/web` — wasm-bindgen entry points for the page
- `web/` — page, loader, HUD, menus, audio

## Build and check

```
bash web/build.sh                          # both wasm bundles
cargo run --release -p runtime --bin shot -- shots 1280 720
cargo run --release -p runtime --bin shot -- --bench 1280 720
```

`WGPU_BACKEND=gl` runs the native tools through the GLES path the WebGL2
bundle uses. Frame-time notes are in [docs/perf.md](docs/perf.md).

Gate:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p web --target wasm32-unknown-unknown --no-default-features --features webgl2 -- -D warnings
cargo test --workspace
(cd web && npm test)
```
