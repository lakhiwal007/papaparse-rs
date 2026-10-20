#!/usr/bin/env bash
# Builds the Node-API addon into crates/papaparse-node/pkg (or $1), for the current platform
# or for the Rust target in $TARGET. Release packaging: scripts/npm-packages.mjs.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/crates/papaparse-node/pkg}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
target="${TARGET:-}"

if [ -n "$target" ]; then
  cargo build -p papaparse-node --profile node-release --target "$target"
  build_dir="$target_dir/$target/node-release"
else
  cargo build -p papaparse-node --profile node-release
  build_dir="$target_dir/node-release"
fi
case "${target:-$(uname -s)}" in
  *apple* | Darwin) lib="libpapaparse_node.dylib" ;;
  *windows* | MINGW* | MSYS* | CYGWIN*) lib="papaparse_node.dll" ;;
  *) lib="libpapaparse_node.so" ;;
esac

rm -rf "$out" && mkdir -p "$out"
cp "$build_dir/$lib" "$out/papaparse.node"
cp "$root"/crates/papaparse-node/js/{index.cjs,binding.cjs} "$root"/crates/papaparse-tape/js/{decode.cjs,papa.cjs,index.d.ts} "$out/"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
cat > "$out/package.json" <<JSON
{
  "name": "papaparse-node",
  "version": "$version",
  "description": "PapaParse-compatible CSV parser: native Node.js addon built from Rust with NAPI-RS",
  "license": "MIT",
  "type": "commonjs",
  "main": "index.cjs",
  "types": "index.d.ts",
  "engines": { "node": ">=18" }
}
JSON
echo "built $out ($(wc -c < "$out/papaparse.node") bytes addon)"
