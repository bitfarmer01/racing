#!/usr/bin/env bash
# Builds both wasm bundles (WebGPU and WebGL2) into web/pkg-* and prints a
# gzip-size report against a 2 MB budget per bundle.
#
# The `wasm` cargo profile (root Cargo.toml) is release with opt-level "s".
# wasm-opt runs when binaryen is on PATH.
#
# Serve from the repo root (python3 -m http.server) and open
# http://localhost:8000/web/ — loader.js picks pkg-webgpu or
# pkg-webgl2. The built bundles are committed so the page runs as-is.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET=wasm32-unknown-unknown
BUDGET_BYTES=$((2 * 1024 * 1024))
OUT=web

if command -v rustup >/dev/null 2>&1; then
  rustup target add "$TARGET" >/dev/null
fi

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "wasm-bindgen-cli not found; installing the version matching the"
  echo "wasm-bindgen crate in Cargo.lock (one-time, several minutes)..."
  WBG_VERSION="$(cargo pkgid -p wasm-bindgen | sed 's/.*#\([0-9.]*\).*/\1/')"
  cargo install wasm-bindgen-cli --version "$WBG_VERSION"
fi

build_bundle() {
  local features="$1" out_name="$2"
  echo "=== building $out_name (features: $features) ==="
  cargo build -p web --target "$TARGET" --profile wasm --no-default-features --features "$features"

  local wasm="target/$TARGET/wasm/web.wasm"
  if command -v wasm-opt >/dev/null 2>&1; then
    local optimized="target/$TARGET/wasm/web.opt.wasm"
    wasm-opt -Os --strip-debug "$wasm" -o "$optimized"
    wasm="$optimized"
  else
    echo "(wasm-opt not on PATH; skipping — install binaryen for a few more percent)"
  fi

  wasm-bindgen "$wasm" --target web --out-dir "$OUT/$out_name" --out-name racing_web
}

build_bundle webgpu pkg-webgpu
build_bundle webgl2 pkg-webgl2

echo
echo "=== gzip-size report (budget: 2 MB = $BUDGET_BYTES bytes) ==="
overall=0
for wasm_file in "$OUT"/pkg-*/racing_web_bg.wasm; do
  name="$(basename "$(dirname "$wasm_file")")"
  raw_bytes=$(stat -c%s "$wasm_file")
  gz_file="$(mktemp)"
  gzip -9 -c "$wasm_file" > "$gz_file"
  gz_bytes=$(stat -c%s "$gz_file")
  rm -f "$gz_file"
  if [ "$gz_bytes" -le "$BUDGET_BYTES" ]; then
    verdict="within budget"
  else
    verdict="OVER BUDGET"
    overall=1
  fi
  printf '%-12s raw %8d B   gzip %8d B   %s\n' "$name" "$raw_bytes" "$gz_bytes" "$verdict"
done
echo
if [ "$overall" -eq 0 ]; then
  echo "all bundles within the 2 MB gzipped budget"
else
  echo "at least one bundle exceeds the 2 MB gzipped budget"
fi
exit "$overall"
