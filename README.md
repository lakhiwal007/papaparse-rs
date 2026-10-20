# papaparse-rs

A Rust port of [PapaParse](https://github.com/mholt/PapaParse), the CSV parser for JavaScript.

- **The whole API.** `parse` and `unparse`, streaming with `step` / `chunk` callbacks,
  pause / resume / abort, Node.js streams, and every option, in Rust and as two drop-in npm
  packages.
- **Same results as PapaParse.** PapaParse 5.7.0's own test suite runs on every commit (all of
  its parse, unparse and callback suites), and a differential fuzzer compares hundreds of
  thousands of random inputs, configs and callback sequences against the real library.
- **Fast.** Natively 2.4–7× faster than PapaParse; in Node, the NAPI-RS addon is 1.2–3× faster
  and the WebAssembly build up to 2.1× on most workloads (see [Performance](#performance)).
- **Linear time on every input.** PapaParse is quadratic on some inputs (a 1.5 MB file can take
  5 s); this port is not.
- **Small and safe.** No `unsafe` code; one runtime dependency (`memchr`); 81 KB gzipped wasm.

| Crate | What it is |
|---|---|
| [`papaparse`](crates/papaparse) | The library: `parse()`, `ChunkParser` / `parse_reader()` (streaming), `unparse()`, `Parser` |
| [`papaparse-cli`](crates/papaparse-cli) | `papaparse` binary: CSV → JSON / NDJSON (streaming), JSON → CSV |
| [`papaparse-node`](crates/papaparse-node) | Native Node.js addon (NAPI-RS): a drop-in `Papa` object, prebuilt per platform |
| [`papaparse-wasm`](crates/papaparse-wasm) | WebAssembly build: a drop-in `Papa` object, one portable package |
| [`papaparse-tape`](crates/papaparse-tape) | Internal: the layer shared by the two JS packages, including PapaParse's streamers (`js/papa.cjs`) |

## Usage

```rust
use papaparse::{parse, ParseConfig, Row, Value};

let csv = std::fs::read_to_string("people.csv")?;
let out = parse(&csv, &ParseConfig::default().header(true).dynamic_typing(true))?;
for row in &out.data {
    let Row::Record(person) = row else { unreachable!() };
    if let Some(Value::Number(age)) = person.get("age") {
        println!("{age}");
    }
}
```

Results borrow from the input (`ParseOutput<'a>`), so unquoted and escape-free fields are never
copied. Call `.into_owned()` to keep a result after the input is gone.

Streaming a file of any size, one row at a time, in bounded memory (`Flow::Abort` stops,
`Flow::Pause` defers the rest to the next push):

```rust
use papaparse::{Flow, ParseConfig, parse_reader_rows};

let file = std::fs::File::open("big.csv")?;
parse_reader_rows(file, &ParseConfig::default().header(true), |step| {
    println!("{:?} (ends at byte {})", step.row, step.meta.cursor);
    Flow::Continue
})?;
```

`ChunkParser` takes pieces you push yourself (`feed` / `finish`, or `feed_rows` /
`finish_rows`), for sockets, async runtimes and other sources that aren't `Read`.

Back to CSV:

```rust
use papaparse::{UnparseConfig, unparse};

let csv = unparse(&out.data, &UnparseConfig::default())?; // `Papa.unparse(results.data)`
```

Node (`papaparse-node` or `papaparse-wasm`, same API as PapaParse):

```js
const Papa = require('papaparse-node');

const { data, errors, meta } = Papa.parse(csvText, { header: true, dynamicTyping: true });
Papa.parse(csvText, { step: (row, parser) => { /* ... */ }, complete: () => { /* ... */ } });
fs.createReadStream('big.csv').pipe(Papa.parse(Papa.NODE_STREAM_INPUT, { header: true }));
const csv = Papa.unparse(data, { quotes: true });
```

CLI:

```sh
papaparse data.csv --header --dynamic-typing --format ndjson   # streams: flat memory
cat data.tsv | papaparse -d $'\t'
papaparse unparse rows.json --escape-formulae > rows.csv
```

### Option mapping

| PapaParse | Rust (`ParseConfig`) |
|---|---|
| `delimiter: ";"` / function | `.delimiter(";")` / `.delimiter_fn(\|input\| …)` |
| `newline: "\r\n"` | `.newline(Newline::CrLf)` |
| `quoteChar`, `escapeChar` | `.quote_char('\'')`, `.escape_char('\\')` |
| `header`, `transformHeader` | `.header(true)`, `.transform_header(\|h, i\| …)` |
| `dynamicTyping: true / {…} / fn` | `.dynamic_typing(true)` / `.dynamic_typing_with(DynamicTyping::Fields(…) / Dynamic(…))` |
| `preview`, `comments`, `fastMode` | `.preview(n)`, `.comments("#")`, `.fast_mode(true)` |
| `skipEmptyLines: true / "greedy"` | `.skip_empty_lines(SkipEmptyLines::Yes / Greedy)` |
| `skipFirstNLines`, `delimitersToGuess` | `.skip_first_n_lines(n)`, `.delimiters_to_guess([…])` |
| `transform` | `.transform(\|value, field\| …)` |
| `step` / `chunk` / `chunkSize` | `ChunkParser::feed_rows` / `feed` with pieces of your size; `parse_reader_rows` / `parse_reader` |
| `Papa.unparse(data, { quotes, quoteChar, escapeChar, delimiter, header, newline, skipEmptyLines, columns, escapeFormulae })` | `unparse(&rows, &UnparseConfig::default().quotes(…)…)` |

## Performance

`just bench-compare` parses the same generated files with all four implementations. Each output
is checksummed, and all four agree on every case. Apple M4, Node 22, median of 5 runs:

| Case | PapaParse (JS) | wasm (Node) | napi-rs addon (Node) | Rust native |
|---|---:|---:|---:|---:|
| 19.6 MB unquoted, arrays | 248 ms | 174 ms (**1.4×**) | 143 ms (**1.7×**) | 48 ms (**5.2×**) |
| 19.6 MB unquoted, header + dynamicTyping | 512 ms | 243 ms (**2.1×**) | 166 ms (**3.1×**) | 74 ms (**6.9×**) |
| 15.9 MB quoted, arrays | 99 ms | 113 ms (0.88×) | 80 ms (**1.2×**) | 33 ms (**3.0×**) |
| 15.9 MB quoted, header + dynamicTyping | 197 ms | 134 ms (**1.5×**) | 92 ms (**2.1×**) | 48 ms (**4.1×**) |
| 0.8 MB single quoted column (adversarial) | 1.28 s | 25 ms (**52×**) | 21 ms (**60×**) | 11 ms (**118×**) |
| 15.9 MB quoted, header, row by row (`step`) | 168 ms | 172 ms (0.98×) | 115 ms (**1.5×**) | 69 ms (**2.4×**) |
| `unparse` of those 250k rows | 215 ms | 139 ms (**1.5×**) | 97 ms (**2.2×**) | 68 ms (**3.1×**) |

In Node, building the JS objects is most of the remaining cost (on the first file the addon spends
69 ms parsing and 117 ms creating rows). That's why the addon can't approach native Rust speed.

Memory when repeatedly parsing the quoted file (steady state, RSS): PapaParse 743 MB, wasm
510 MB, napi-rs addon 1,055 MB. The addon's native buffers come on top of V8's heap. Full
table with throughput and peak memory: [bench/RESULTS.md](bench/RESULTS.md).

**Which JS package?** `papaparse-node` for the fastest Node.js parsing (one prebuilt binary per
platform); `papaparse-wasm` for browsers, edge runtimes, lowest memory, or a single portable file.

How: substring searches use SIMD (`memchr`, including wasm `simd128`) and are memoized, which
removes the quadratic rescans in the JS algorithm. Fields borrow from the input. The wasm
build sends a compact tape of cell tags and string offsets to JS, which rebuilds rows with
`substring` slices of the original string.

## Quality gates

| Check | Command |
|---|---|
| Upstream test suite: parse, core and unparse (206 runnable cases) + 9 hand-ported callback cases | `cargo test` |
| Upstream suites against both npm packages, callbacks included (233 cases) | `just js-test` |
| Event traces vs PapaParse on large streamed inputs (pause, abort, streams) | `just js-test` |
| Differential fuzzing vs PapaParse (inputs, configs, callbacks, `unparse`) | `just js-test` |
| Coverage-guided fuzzing: no panics; pieces = whole; `unparse` round trip | `just fuzz` |
| Heap accounting: no bytes retained after parsing, streaming or unparsing | `cargo test --test no_leaks` |
| Undefined behaviour (Miri) | `just miri` |
| Advisories, licenses, sources | `just deny` |
| Lints: clippy pedantic, `-D warnings`, `missing_docs` | `just lint` |

## Status

| Area | Status |
|---|---|
| Core tokenizer (`Papa.Parser`) | ✅ |
| `Papa.parse` on strings | ✅ |
| `Papa.unparse` | ✅ |
| Streaming / chunked input (`step`, `chunk`, `Read` sources, pause / resume / abort) | ✅ |
| Callback options in the JS packages | ✅ |
| Node.js streams, `Papa.NODE_STREAM_INPUT` | ✅ |
| Per-platform npm packages of the addon | ✅ (release workflow) |
| Browser build of the wasm package, `download`, `File`, web workers | ⏳ planned (browser only) |

See [docs/PORTING.md](docs/PORTING.md) for the roadmap and the remaining differences from JavaScript.

## Development

The toolchain is pinned in `rust-toolchain.toml`; `rustup` installs it automatically.

```sh
cargo install just   # optional task runner
just check           # fmt + clippy + tests (what CI runs)
just conformance     # upstream test-suite scoreboard
just bench-compare   # PapaParse vs native vs wasm vs addon (needs node, wasm-bindgen-cli, binaryen)
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. PapaParse is © Matthew Holt, also MIT; see [LICENSE](LICENSE).
