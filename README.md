# racing

Road Rash-inspired 2.5D motorcycle action racer for the browser.
Rust + custom thin wgpu engine, compiled to WASM. Architecture per the
approved blueprint: fixed 60 Hz simulation, allocation-free frame loop,
sprite atlases, projected road strips, WebGPU with WebGL2 fallback.

## Layout

- `crates/sim` — fixed-tick simulation core: input ring, arena allocator, entity pools, deterministic clock
- `crates/world` — road and projection math: scanline road strips, curvature, hills, billboard placement, culling
- `crates/assets` — atlas packer: sprites in, packed atlas + index out
- `crates/render` — GPU layer (wgpu/WebGL2), added after first repo merge

## Gate

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets`
