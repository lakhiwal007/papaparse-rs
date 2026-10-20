# Developer tasks. Install with `cargo install just`, then run `just <task>`.

default: check

# Format, lint and test — what CI runs.
check: fmt-check lint test

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace --all-features

# Print the upstream conformance scoreboard with every failure and skip.
conformance:
    CONFORMANCE_REPORT=1 cargo test -p papaparse --test conformance -- --nocapture

# Re-extract fixtures from upstream PapaParse (needs node and git).
sync-upstream version="":
    scripts/sync-upstream.sh {{version}}

# Criterion micro-benchmarks of the Rust crate.
bench:
    cargo bench -p papaparse

# Build the npm package into crates/papaparse-wasm/pkg.
# Needs wasm-bindgen-cli (same version as Cargo.lock) and binaryen's wasm-opt.
wasm:
    scripts/build-wasm.sh

# Build the Node-API addon for this platform into crates/papaparse-node/pkg.
node:
    scripts/build-node.sh

# Assemble the npm packages from this platform's addon into target/npm (release CI does all platforms).
npm-pack: node
    mkdir -p target/npm-bin
    cp crates/papaparse-node/pkg/papaparse.node "target/npm-bin/papaparse.$(rustc -vV | sed -n 's/^host: //p').node"
    node scripts/npm-packages.mjs target/npm-bin target/npm

# Upstream fixtures (incl. callbacks and unparse), streaming checks and 20k differential cases,
# against both npm packages.
js-test: wasm node
    cd bench && npm ci --no-audit --no-fund
    for pkg in wasm node; do \
        node tests/js/conformance.cjs crates/papaparse-$pkg/pkg && \
        node tests/js/streaming.cjs crates/papaparse-$pkg/pkg && \
        node tests/js/differential.cjs crates/papaparse-$pkg/pkg 20000 || exit 1; \
    done

# PapaParse (JS) vs papaparse-rs (native, wasm, napi-rs addon); writes bench/RESULTS.md.
bench-compare: wasm node
    cd bench && npm ci --no-audit --no-fund && node gen-data.mjs
    rm -rf bench/pkg-wasm bench/pkg-node
    cp -R crates/papaparse-wasm/pkg bench/pkg-wasm && cp -R crates/papaparse-node/pkg bench/pkg-node
    cargo build --release -p papaparse-bench
    node bench/run.mjs

# Undefined-behaviour check of the unit tests (needs nightly + miri).
miri:
    cargo +nightly miri test -p papaparse --lib --test callbacks

# Coverage-guided fuzzing of every target: parse, chunked (pieces = whole), unparse (round trip).
# Needs nightly and `cargo install cargo-fuzz`.
fuzz seconds="60":
    for target in parse chunked unparse; do \
        cargo +nightly fuzz run $target -- -max_total_time={{seconds}} || exit 1; \
    done

# Needs `cargo install cargo-deny`.
deny:
    cargo deny check

doc:
    cargo doc --workspace --no-deps --open
