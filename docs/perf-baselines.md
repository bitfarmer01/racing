# Performance and memory baselines

Audit item 16 (Wave 7 verification). One engaged race per build, measured with a
headless-Chrome probe (`measure2.js`, puppeteer, SwiftShader software rasterization).
Recorded on 2026-11-25.

## Environment and method

- Headless Chrome (`--enable-unsafe-swiftshader`), puppeteer, deterministic input
  (held `ArrowUp` after the CLICK TO RIDE gate).
- The timing window opens only after the HUD reports real speed (`engaged: true`),
  then samples every rAF delta for 60 s of wall clock.
- **Caveat:** SwiftShader frame times are software-rasterizer times. They are a
  consistent relative baseline between builds, not representative of real-GPU
  frame times. The user's hardware reports much lower numbers (for example
  ~29 MB JS heap on a real run); the harness consistently reports ~2 MB
  `performance.memory.usedJSHeapSize` in headless mode. Treat absolute memory
  values as harness-specific.
- Reported `fps` field in the harness output is faulty (formula bug); FPS here is
  computed as frames / 60 s.
- Page shows a recurring 404 for a missing favicon/resource in both builds —
  cosmetic, identical on both sides.

## Matched comparison — 640×360 viewport

| Metric | main @ e982682 (Wave 6) | wave-7-world @ 8cb8fb7 (Wave 7 audit fixes) |
|---|---|---|
| Engaged | yes | yes |
| Frames in 60 s | 63 | 2,417 |
| FPS (frames/60 s) | ~1.05 | ~40.3 |
| Median frame | 933.3 ms | 16.7 ms |
| p95 frame | 1,116.6 ms | 66.6 ms |
| JS heap | 2.2 MB | 2.0 MB |
| Wasm linear memory | 1.3 MB | 1.3 MB |
| HUD speed at window end | 154 | 215 |

At 1920×1080 SwiftShader the Wave 6 build cannot finish its 3 s countdown within
a 4-minute engagement window (~0.6 fps; fixed-tick accumulator capped at 5 ticks
per frame), so no engaged race measurement was possible there. The Wave 7 build
measures at 1080p: 158–202 frames in 60 s, median 383–517 ms, p95 467–633 ms,
JS heap 1.9–2.1 MB, wasm 1.3 MB across two runs. The difference between the
1080p and 640×360 branch numbers is the quality-tier system (Wave 7 PR #20):
resolution scaling and MSAA tiers hold ~40 fps at 640×360 where the pre-tier
build was unusable.

## Sprite and road geometry counts (native probe)

- Steady-state sprite batch: **100 sprites per frame** (WebGpuCapable tier,
  `SpriteBatch::for_tier`), constant across 8 consecutive frames while
  accelerating — consistent with the batch-reuse regression contract (batch
  refills per frame, never grows).
- Road mesh quads: not counted (no public accessor at probe time); road strip is
  an indexed triangle list of quads per `RoadGeometry` segments.

## Gzipped bundle sizes

| Bundle | main @ e982682 | wave-7-world @ 8cb8fb7 |
|---|---|---|
| webgl2 | 656,134 B | (rebuild pending fix PRs) |
| webgpu | 93,625 B | (rebuild pending fix PRs) |

Both within the 2 MB gzipped budget. Branch-tip bundle sizes to be re-recorded
after the Wave 7.5 fix PRs merge.

## Re-measure

Run the two built bundles under static servers, then:

```
node measure2.js http://localhost:<port>/ "<label>"
```

with `VW`/`VH` env vars for the viewport (defaults 1920×1080).
