# Frame time

## How it is measured

`cargo run --release -p runtime --bin shot -- --bench <w> <h>` loads each
track, lets the race run for 8 s with the player on autopilot, then times 30
frames end to end (simulation, effects, culling, GPU submit and a blocking
wait for the GPU). It uses the same renderer and WebGL2-level device limits as
the browser build. `RENDER_SCALE=0.5` sets the internal resolution the way the
page's dynamic resolution does under load; `WGPU_BACKEND=gl` uses the GLES
path the WebGL2 bundle uses.

The numbers below come from a GPU-less container, so they are **software
rendering on the CPU** (Mesa llvmpipe over Vulkan). They show relative cost
and scaling, not what a real GPU does; a discrete or recent integrated GPU is
one to two orders of magnitude faster.

## After (this branch), llvmpipe, 2026-11

| Resolution | Golden Coast | Alpine Lake | Dusk Canyon |
|---|---|---|---|
| 640×360 | 54 ms | 58 ms | 51 ms |
| 1280×720 | 98 ms | 113 ms | 106 ms |
| 1920×1080 | 174 ms | 192 ms | 200 ms |
| 1920×1080 at 0.5 render scale | 80 ms | 95 ms | 90 ms |

Per frame: 79–92 draw calls, 155–241 k triangles (27–100 k of them scenery
instances), 12–23 k shadow-pass triangles, 650–3,100 visible props.
Simulation plus effects: about 0.05 ms per frame.

Scenery level of detail and dropping terrain from the shadow pass took Alpine
Lake from 99 ms to 58 ms at 640×360 (605 k to 238 k triangles).

The design-system pass (ink outlines, speed lines, impact frames, split tone,
grain) runs inside the existing composite pass. Re-measured afterwards:
640×360 54–61 ms, 1280×720 108–117 ms, same draw and triangle counts.

## Round 2: new bike and rider, contact shadows, combat (2026-11)

Measured the same way at 1280×720, alternating three runs of `main` (75b7ca9)
and this branch on the same machine, averaged:

| | Golden Coast | Alpine Lake | Dusk Canyon |
|---|---|---|---|
| main | 97 ms | 104 ms | 101 ms |
| this branch | 104 ms (+7 %) | 114 ms (+9 %) | 129 ms (+29 %) |

Where it goes:

- The bike and rider are now about 8,400 triangles each (lofted fairing,
  round tyres, a jointed rider), drawn instanced in one call per pass for all
  eight bikes. That is about 50 k more triangles in the main pass.
- The vertex shader poses the rig (suspension, steering on the rake axis,
  hang-off, tuck, kick and guard).
- The contact shadow under each bike runs in the ground shader, only within
  70 m of the camera and only for bikes in that range; turning it off saves a
  few milliseconds on llvmpipe.
- The bench frame is no longer the same picture: the race plays out
  differently by the 8 s mark (launch control, combat, difficulty), and on
  Dusk Canyon eight more draw calls are in view. So part of that track's
  jump is a different scene, not just cost.

This is CPU software rendering, where every vertex and pixel costs the same
CPU time; on a real GPU this much geometry is small.

## Round 3: traffic, weapons, hunters, season (2026-11)

Same method at 1280×720, alternating two runs of `main` (e4c17e3) and this
branch:

| | Golden Coast | Alpine Lake | Dusk Canyon |
|---|---|---|---|
| main | 96 ms | 102 ms | 117 ms |
| this branch | 98 ms (+2 %) | 105 ms (+3 %) | 101 ms (−13 %) |

- Traffic adds 6 to 30 cars per track, drawn through the bike's instanced
  pipeline: one extra draw per vehicle kind in view (hatch, saloon, van,
  truck), about 40 k more triangles at the bench frame, culled past 320 m.
- Clubs and chains are extra parts of the rider mesh, collapsed to nothing
  in the vertex shader unless carried, so they cost no draw calls.
- Lane paint is a few instructions in the road shader.
- The simulation (traffic, hunting AI, contacts) went from about 0.05 ms to
  0.07 ms per frame.
- The Dusk Canyon drop is a different scene at the bench mark, not a
  speed-up: the race plays out differently by 8 s.

## Round 4: ramps, wide straights, hairpins, fight waves (2026-11)

Same method at 1280×720, alternating two runs of the round 3 build
(c2203dc) and this one. The machine ran slower than in round 3, so compare
the rows with each other, not with the tables above:

| | Golden Coast | Alpine Lake | Dusk Canyon |
|---|---|---|---|
| round 3 | 126 ms | 138 ms | 133 ms |
| this build | 128 ms (+1 %) | 136 ms (−1 %) | 141 ms (+6 %) |

- Kickers are a few dozen triangles each in the road chunk they sit in, so
  they add no draw calls.
- The road's width now varies along the track; it is carried in each road
  vertex rather than a uniform, which costs nothing per pixel.
- Dusk Canyon's top straight and hairpin are new, so the bench frame there
  is a different scene; the change is within run-to-run noise elsewhere.
- The simulation is unchanged at about 0.08 ms per frame.

## Before (main @ 53aa156)

The previous build was a 2D sprite scaler (two draw calls), measured by its
authors in headless Chrome on SwiftShader (also CPU rendering), recorded in
the old `docs/perf-baselines.md`:

| Resolution | Median frame |
|---|---|
| 640×360 | 16.7 ms (p95 66.6 ms) |
| 1920×1080 | 383–517 ms (p95 467–633 ms) |

The two harnesses differ (headless Chrome + SwiftShader vs native llvmpipe),
so these are not a like-for-like comparison. The page now holds frame rate
with dynamic resolution (`web/hud.js`, `ResolutionGovernor`): it lowers the
internal render scale in 5 % steps, down to 50 %, when frames run over
~17.6 ms and climbs back after three calm seconds. The device pixel ratio is
capped at 1.5.

## Not measured

No browser run was possible in this environment (no browser or GPU), so
in-browser frame times on real hardware are still to be recorded.
