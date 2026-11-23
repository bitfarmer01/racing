//! `assets-pack <in-dir> <out-prefix>` — pack PNG sprites into one atlas
//! PNG plus an index JSON (`<out-prefix>.png` / `<out-prefix>.json`).

#![forbid(unsafe_code)]

use std::path::Path;
use std::process::ExitCode;

use assets::{pack_directory, write_outputs, PackOptions};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (in_dir, out_prefix) = match args.as_slice() {
        [in_dir, out_prefix] => (in_dir.clone(), out_prefix.clone()),
        _ => {
            eprintln!("usage: assets-pack <in-dir> <out-prefix>");
            eprintln!("  packs PNG sprites from <in-dir> into <out-prefix>.png");
            eprintln!("  and writes the sprite index to <out-prefix>.json");
            return ExitCode::from(2);
        }
    };

    match run(Path::new(&in_dir), Path::new(&out_prefix)) {
        Ok(count) => {
            println!("packed {count} sprites");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("assets-pack: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(in_dir: &Path, out_prefix: &Path) -> Result<usize, assets::PackError> {
    let packed = pack_directory(in_dir, PackOptions::default())?;
    write_outputs(&packed, out_prefix)?;
    Ok(packed.index.len())
}
