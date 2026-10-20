#!/usr/bin/env bash
# Builds the npm package for Node into crates/papaparse-wasm/pkg (or $1).
# Needs: rustup target wasm32-unknown-unknown, wasm-bindgen-cli matching Cargo.lock, wasm-opt (binaryen).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/crates/papaparse-wasm/pkg}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"

cargo build -p papaparse-wasm --target wasm32-unknown-unknown --profile wasm-release
rm -rf "$out"
wasm-bindgen "$target_dir/wasm32-unknown-unknown/wasm-release/papaparse_wasm.wasm" \
  --target nodejs --out-dir "$out" --no-typescript
wasm-opt -Oz --strip-debug --strip-producers \
  --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals \
  "$out/papaparse_wasm_bg.wasm" -o "$out/papaparse_wasm_bg.wasm"

cp "$root/crates/papaparse-wasm/js/index.cjs" "$root"/crates/papaparse-tape/js/{decode.cjs,papa.cjs,index.d.ts} \
  "$root/crates/papaparse-wasm/README.md" "$root/LICENSE" "$out/"
repository="$(sed -n 's/^repository = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
cat > "$out/package.json" <<JSON
{
  "name": "papaparse-wasm",
  "version": "$version",
  "description": "PapaParse-compatible CSV parser compiled from Rust to WebAssembly",
  "license": "MIT",
  "repository": { "type": "git", "url": "git+$repository.git" },
  "keywords": ["csv", "parser", "papaparse", "wasm", "rust"],
  "type": "commonjs",
  "main": "index.cjs",
  "types": "index.d.ts",
  "files": ["index.cjs", "decode.cjs", "papa.cjs", "index.d.ts", "papaparse_wasm.js", "papaparse_wasm_bg.wasm", "README.md", "LICENSE"],
  "engines": { "node": ">=18" }
}
JSON
echo "built $out ($(wc -c < "$out/papaparse_wasm_bg.wasm") bytes wasm, $(gzip -9c "$out/papaparse_wasm_bg.wasm" | wc -c) gzipped)"
