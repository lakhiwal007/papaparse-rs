# papaparse-node

The `papaparse` Rust crate as a native Node.js addon (Node-API, built with
[NAPI-RS](https://napi.rs)), as a drop-in `Papa` object.

```sh
npm install papaparse-node    # installs the prebuilt addon for your platform
```

```js
const Papa = require('papaparse-node');

const { data, errors, meta } = Papa.parse(csvText, { header: true, dynamicTyping: true });
const csv = Papa.unparse(data);
```

- **Same results as PapaParse 5.7.0 and `papaparse-wasm`.** Same API and coverage as
  `papaparse-wasm` (callbacks, streams, `unparse`); both share `papaparse-tape`. CI runs
  upstream's fixtures, streaming traces and differential fuzzing against the addon on Linux,
  macOS and Windows.
- **Speed.** The fastest way to parse CSV in Node with this project; see `bench/RESULTS.md`.
- **Memory.** Higher steady-state RSS than PapaParse or wasm, because native buffers come on
  top of V8's heap. Prefer `papaparse-wasm` when memory matters more than speed.
- **Safety.** No hand-written `unsafe` (`unsafe_code = "deny"`; NAPI-RS generates the FFI
  glue). Built with `panic = "unwind"` and `#[napi(catch_unwind)]`, so a panic surfaces as a
  JS exception rather than aborting the Node process. Exceptions thrown by your callbacks
  propagate out of `Papa.parse` as in PapaParse.

## Platforms and packaging

`papaparse-node` holds the JavaScript; the addon itself comes from one small package per
platform, listed as optional dependencies so npm installs only the matching one:
`papaparse-node-{darwin-arm64, darwin-x64, linux-x64-gnu, linux-arm64-gnu, linux-x64-musl,
linux-arm64-musl, win32-x64-msvc, win32-arm64-msvc}`. Linux builds target glibc 2.17+ (or
musl). `scripts/npm-packages.mjs` assembles them and the release workflow publishes them.

Building from source for the current platform: `just node` (→ `crates/papaparse-node/pkg`).
