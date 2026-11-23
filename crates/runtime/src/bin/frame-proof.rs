//! The offscreen frame-render proof: compose a frame from a live race and
//! render it to `target/frame-proof.png`.
//!
//! Run with `cargo run -p runtime --bin frame-proof`. On a host with no GPU
//! adapter (not even a software one) this prints the explicit failure and
//! exits nonzero — it never fakes success. No tests are attached by design:
//! the GPU path cannot run on the GPU-less host, so the blueprint's test
//! gate is intentionally unmet for this code.

use std::path::PathBuf;

use game::input::GameInput;
use runtime::frame::{demo_race, AtlasMapping, GameFrame};
use runtime::offscreen::{render_proof, ProofScene};
use runtime::sprites::pack_placeholder_atlas;

/// `crates/runtime` → workspace root → `target/frame-proof.png`.
fn proof_output_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("runtime crate lives under the workspace root")
        .join("target")
        .join("frame-proof.png")
}

fn main() {
    if let Err(err) = run() {
        eprintln!("offscreen frame render FAILED: {err}");
        eprintln!("no PNG was produced — the GPU path is unproven on this host");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let packed = pack_placeholder_atlas()?;
    let mapping = AtlasMapping::from_packed(&packed);
    let (mut race, geometry) = demo_race();

    // Four seconds of full throttle from the grid: the camera is chasing at
    // speed, rivals are strung out down the road, and one composed tick
    // builds the frame.
    for _ in 0..239 {
        race.push_input(GameInput::Accelerate);
        race.tick();
    }
    let frame = GameFrame::advance(&mut race, &geometry, &mapping, &[GameInput::Accelerate]);
    println!(
        "composed frame: {} road strips, {} batched sprites",
        frame.road.vertices.len() / 4,
        frame.batch.len()
    );

    let scene = ProofScene {
        frame: &frame,
        atlas_rgba: packed.atlas.as_raw(),
        atlas_dims: mapping.dims(),
    };
    let out = proof_output_path();
    let rendered = render_proof(&scene, &out)?;
    println!(
        "wrote {} ({}x{} px)",
        out.display(),
        rendered.width,
        rendered.height
    );
    Ok(())
}
