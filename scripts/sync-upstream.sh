#!/usr/bin/env bash
# Fetches PapaParse at the pinned version and regenerates conformance fixtures.
# Usage: scripts/sync-upstream.sh [version]   (default: contents of conformance/UPSTREAM_VERSION)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
version="${1:-$(cat "$root/conformance/UPSTREAM_VERSION")}"
dest="$root/vendor/papaparse-$version"

if [ ! -d "$dest" ]; then
  mkdir -p "$root/vendor"
  git clone --quiet --depth 1 --branch "$version" https://github.com/mholt/PapaParse.git "$dest"
fi

node "$root/scripts/extract-upstream-fixtures.mjs" "$dest"
echo "$version" > "$root/conformance/UPSTREAM_VERSION"
echo "Fixtures regenerated from PapaParse $version. Run: cargo test -p papaparse --test conformance"
