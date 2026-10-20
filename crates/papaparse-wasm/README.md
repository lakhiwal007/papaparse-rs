# papaparse-wasm

The `papaparse` Rust crate compiled to WebAssembly, as a drop-in `Papa` object for Node.js.

```sh
just wasm      # or: scripts/build-wasm.sh  → crates/papaparse-wasm/pkg
```

```js
const Papa = require('papaparse-wasm');

const { data, errors, meta } = Papa.parse(csvText, { header: true, dynamicTyping: true });

Papa.parse(csvText, { step: (row, parser) => { /* one row at a time */ } });
fs.createReadStream('big.csv').pipe(Papa.parse(Papa.NODE_STREAM_INPUT, { header: true }));

const csv = Papa.unparse(data);
```

Everything PapaParse 5.7.0 does with strings and Node.js streams works the same way: every
parse option, the `step` / `chunk` / `complete` / `error` / `beforeFirstChunk` callbacks,
`chunkSize`, `transform`, `transformHeader`, `dynamicTyping` and `delimiter` functions,
pause / resume / abort, readable streams, `Papa.NODE_STREAM_INPUT`, and `Papa.unparse` with
all its options. Results match PapaParse, including UTF-16 `meta.cursor` / `error.index` and
`Date` objects. Tests (`just js-test`) run upstream's fixtures (233 cases, including the
callback and `unparse` suites), compare event traces on large streamed inputs, and fuzz the
package against PapaParse.

Not supported: `download`, `File` input and web workers (browser features; `worker: true`
parses on the calling thread). Differences in edge cases are listed in
[docs/PORTING.md](../../docs/PORTING.md).

This is the portable choice: one file for every platform, and the lowest memory use. For the
fastest parsing in Node, use `papaparse-node`.

Requirements: Node ≥ 18. Size: ~171 KB wasm, ~81 KB gzipped.
