# Benchmarks: PapaParse vs papaparse-rs

```sh
just bench-compare
```

This builds the wasm package and the native runner, generates deterministic datasets
(`gen-data.mjs`), and runs every case in a fresh process per implementation (`run.mjs`):

- **PapaParse 5.7.0** in Node (`runners/js.mjs`)
- **papaparse-rs wasm** in Node through the npm entry point (`runners/wasm.mjs`)
- **papaparse-rs napi-rs addon** in Node through its entry point (`runners/node.mjs`)
- **papaparse-rs native** (`rust/`)

All four use the same protocol: one warm-up, then N timed runs on an in-memory string.
`mode` in a case's config picks what is timed: a plain parse, a parse delivering rows one by
one to a `step` callback, or `unparse` of already-parsed rows. Each run prints the times, row
and error counts, and an FNV-1a checksum of `JSON.stringify(data)` (of the CSV for `unparse`),
computed outside the timed region.
The checksums must agree across implementations, so a speedup can't come from a wrong
answer. Peak RSS comes from `/usr/bin/time`. Results are written to `RESULTS.md`.
