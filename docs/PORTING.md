# Porting guide

How the JavaScript source maps onto this repository, what is done, what is next, and where the
Rust version deliberately differs.

## Source map

| papaparse.js (v5.7.0) | Rust |
|---|---|
| `function Parser` | `crates/papaparse/src/core.rs` (`Parser`, `Run`) |
| `ParserHandle.parse`, `processResults`, `applyHeaderAndDynamicTypingAndTransformation` | `crates/papaparse/src/handle.rs` |
| `CsvToJson` + `StringStreamer` (string input, `skipFirstNLines`, BOM) | `crates/papaparse/src/handle.rs` (`parse`) |
| `guessDelimiter`, `guessLineEndings` | `crates/papaparse/src/guess.rs` |
| `indexOf` (as used by the parser) | `crates/papaparse/src/search.rs` (memoized SIMD search) |
| `parseDynamic`, `testFloat`, `FLOAT`, `ISO_DATE` | `crates/papaparse/src/typing.rs` (no regex dependency) |
| `ParserHandle` (state across chunks) | `crates/papaparse/src/handle.rs` (`ParserHandle`) |
| `JsonToCsv` (`Papa.unparse`) | `crates/papaparse/src/unparse.rs` |
| `ChunkStreamer`, `ReadableStreamStreamer` (for Rust users) | `crates/papaparse/src/stream.rs` (`ChunkParser`, `parse_reader*`) |
| `CsvToJson`, `ChunkStreamer`, `StringStreamer`, `ReadableStreamStreamer`, `DuplexStreamStreamer` (JS packages) | `crates/papaparse-tape/js/papa.cjs`, ported nearly verbatim on top of `Session` (`src/session.rs`) |
| `FileStreamer`, `NetworkStreamer`, workers | not ported: browser-only (`File`, XHR, Web Workers) |

Keep the Rust structure close to the JS so that upstream fixes can be ported by diffing.
Comments that quote the JS (`// Closing quote at EOF`) are kept on purpose.

## Conformance testing

1. `scripts/extract-upstream-fixtures.mjs` loads upstream `tests/test-cases.js` in a Node VM and
   writes every suite to `conformance/fixtures/*.json`. Values JSON can't hold are tagged
   (`{"$fn": …}`, `{"$date": …}`, `{"$num": "NaN"}`, `{"$undefined": true}`).
2. `crates/papaparse/tests/conformance.rs` replays them. Cases with a JS callback in the
   config are skipped there and ported by hand in `tests/callbacks.rs`.
3. `conformance/expected-failures.txt` lists known failures. The test fails both on new
   failures and when a listed case starts passing.

To move to a new upstream release: `just sync-upstream 5.8.0`, then fix or list the failures.

## Testing the JS packages

`tests/js/conformance.cjs` runs the `core_parser`, `parse`, `unparse` and `custom` suites
against a built package, reviving the fixtures' JS functions (`{"$fn": source}`) so callback
cases run as written; the `custom` suite's test bodies run with the package as `Papa`.
`tests/js/streaming.cjs` compares event traces with PapaParse on large inputs (pause / resume,
abort, `chunkSize`, Node streams, `NODE_STREAM_INPUT`), and `tests/js/differential.cjs` fuzzes
everything, including callbacks and `unparse`, against PapaParse.

## Roadmap

1. **Browser / ESM build** of `papaparse-wasm` (wasm-bindgen `web` target), and the
   browser-only inputs (`File`, `download`) on top of it.
2. **wasm quoted-data speed**: the one parse case where wasm trails PapaParse (≈0.86×). The
   cost is per-row allocation inside wasm; a tokenizer that writes the tape directly is the
   next step.
3. **Async I/O** (optional): an `AsyncRead` counterpart of `parse_reader`.

## Architecture notes

- **Linear-time search.** PapaParse calls `indexOf(delimiter | newline | quote, cursor)` after
  almost every field. When the needle no longer occurs, each call rescans to the end of the
  input, so some inputs are quadratic (measured: 0.4 MB → 0.33 s, 0.8 MB → 1.3 s, 1.6 MB → 5.1 s
  in PapaParse 5.7.0). `search::Finder` memoizes the last answer per needle and uses SIMD
  `memchr`, so every byte is scanned at most once per needle. This is behaviour-preserving:
  JS's `nextDelim`/`nextNewline` variables always hold the first occurrence at or after the
  cursor, which is exactly what the memoized finder returns.
- **Zero-copy fields.** The tokenizer yields `Cow<'a, str>`; only quoted fields containing
  escape sequences, `transform` output and renamed headers are allocated.
- **Shared headers.** Records share one `Arc` header with a name→column index; a record is
  just a `Vec` of values.
- **JS whitespace.** `trim()` and regex `\s` in JS use ECMAScript's WhiteSpace + LineTerminator
  sets, which differ from `char::is_whitespace` (U+FEFF counts, U+0085 does not). All
  whitespace tests go through `is_js_whitespace`. Differential fuzzing found this.
- **JS packages.** `papaparse-wasm` and `papaparse-node` share `papaparse-tape` (option
  mapping, tape encoding, `js/decode.cjs`, `js/papa.cjs`), so they return identical results by
  construction. The addon hands the tape's `Vec`s to V8 as external ArrayBuffers (no copy).
- **Callbacks across the boundary.** `ParseConfig` callbacks are `Send + Sync + 'static`;
  JS functions are neither. `Hooks` (and `UnparseHooks`) pass borrowed `FnMut`s for one call
  instead. `transformHeader` and the `quotes` / `escapeFormulae` callbacks of `unparse` run
  through hooks; `transform` and a `dynamicTyping` function run in JS while rows are rebuilt,
  because they may return any JS value.
- **Step batching.** In step mode Rust sends rows to JS a few hundred at a time; JS calls
  `step` per row and reports where it stopped. `ParserHandle::unstep` rewinds the row and
  preview counters so a pause part-way through a batch resumes exactly where PapaParse would.
- **wasm transport.** `papaparse-wasm` returns a tape: one tag byte per cell, UTF-16
  `(start, end)` offsets into the original JS string, numbers, and a side table for text
  that is not a slice of the input. `js/index.cjs` rebuilds rows with `substring`. This is
  about 3× faster than one `JSON.parse` and much faster than per-cell boundary crossings.

## Deliberate differences from JavaScript

| Topic | JavaScript | Rust crate | npm (wasm) package |
|---|---|---|---|
| `meta.cursor`, `error.index` | UTF-16 code units | UTF-8 byte offsets (equal for ASCII) | UTF-16, same as JS |
| Dates under `dynamicTyping` | `Date` object | `Value::Date`, the original text | `Date` object, same as JS |
| Invalid config types (`comments: 5`) | silently ignored | not expressible | ignored |
| Comment marker equal to a guessed delimiter | throws | that candidate is skipped | same as Rust |
| Duplicate-header warning | `console.warn` | none (see `meta.renamed_headers`) | none |
| `quoteChar` / `escapeChar` | any string | one `char` | one character |
| Worst-case time | quadratic on some inputs | linear | linear |
| Unpaired UTF-16 surrogates | kept | not representable (`&str`) | kept where text is a slice of the input; U+FFFD in header names and rebuilt text, and in `unparse` output |
| Streams: byte order mark | stripped from strings only | `ChunkParser` strips it from the first piece | as JS |
| Streams: input ending in a newline | `ReadableStreamStreamer` drops the final empty row a string parse has | `ChunkParser` keeps it, so pieces = whole | as JS |
| `worker: true` | parses in a Web Worker | n/a | parses on the calling thread |
| Workers, `download`, `File` | supported in browsers | n/a | not supported (throws) |
| A `quotes` function in `unparse` | called with the cell | `Quotes::Dynamic` gets the `Value` | called with the original JS value, as in JS |

PapaParse limitations kept on purpose (the fuzzers document them): `unparse` quotes a cell only
when it contains the delimiter, so multi-character delimiters can produce ambiguous rows, and it
escapes quotes but not the escape character, so with `escapeChar` a cell ending in the escape
character does not round-trip.
