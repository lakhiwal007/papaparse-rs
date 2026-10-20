# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added (second pass: the remaining roadmap)
- `unparse`, `unparse_table`, `UnparseConfig` (`Papa.unparse`): quoting, custom quote and
  escape characters, `columns`, `skipEmptyLines`, `escapeFormulae`, numbers printed like
  JavaScript's `Number.toString`. Passes all 55 upstream unparse cases.
- Streaming (`ChunkParser`, `parse_reader`, `parse_reader_rows`, `ParserHandle`): input in
  pieces from any `std::io::Read`, `chunk` and `step` style callbacks, abort and pause, bounded
  memory. Fuzzed against whole-input parsing at random cut points.
- `Hooks` / `parse_with_hooks` and `UnparseHooks`: callbacks that borrow local state.
- `Flow::Pause`.
- JS packages are now drop-in `Papa` objects: `step`, `chunk`, `complete`, `error`,
  `beforeFirstChunk`, `chunkSize`, `transform`, `transformHeader`, `dynamicTyping` and
  `delimiter` functions, pause / resume / abort, Node.js readable streams,
  `Papa.NODE_STREAM_INPUT`, and `Papa.unparse`. PapaParse's streamer code is ported nearly
  verbatim (`papa.cjs`), so callbacks see exactly what PapaParse gives them; upstream's
  `custom` suite (22 runnable cases) runs against both packages.
- CLI: `papaparse unparse` (JSON to CSV); `--format ndjson` now streams in constant memory.
- Per-platform npm packages for the addon (`scripts/npm-packages.mjs`, release workflow):
  macOS, Linux glibc 2.17+ and musl, Windows; x64 and arm64.
- Fuzz targets `chunked` and `unparse`; the leak test covers streaming and unparse.

### Fixed (second pass)
- JS packages: a `skipEmptyLines` string other than `"greedy"` now counts as `true`, as in
  PapaParse.

### Changed (second pass)
- `napi-derive` without its `type-def` feature (the typings are hand-written).
- wasm size budget raised from 64 to 88 KiB gzipped for `unparse`, streaming and callbacks
  (82 KB now, of which about 14 KB raw is shortest float printing for `unparse`).

### Performance (second pass)
- JS `step`: rows cross into JS in batches of 512 as a tape, decoded lazily row by row;
  `ParserHandle::unstep` keeps pause / abort exact. 1.4× PapaParse with the addon.
- `unparse`: one scan per cell decides quoting in the common case, no per-number allocation,
  and records whose header matches the output columns are read by position. 3.2× PapaParse
  natively.

### Performance
- Memoized SIMD substring search: parsing is linear on every input. PapaParse (and the
  first version of this port) is quadratic on, e.g., single-column quoted files.
- Zero-copy parsing: fields are `Cow<'a, str>` borrowing from the input.
- Records share one header (`Arc`) instead of a hash map with cloned keys per row.
- Delimiter detection decides the quote-free fast path once instead of once per candidate.
- wasm: results travel as a typed-array tape and are rebuilt with `substring` in JS;
  `simd128` enabled; `opt-level = 3` (measured +40% speed for +2.6 KB gzipped).

### Fixed
- Whitespace tests now follow JavaScript (`trim`, `\s`): U+FEFF is whitespace, U+0085 is not.
  Found by differential fuzzing against PapaParse.
- `preview` combined with `header` can no longer overflow.

### Changed
- `ParseOutput`, `Row`, `Record`, `Value` and `RawOutput` borrow from the input (`'a`);
  use `into_owned()` for `'static` results.
- `Row::Record` holds a `Record` (`get`, `iter`, `values`, `header`, `Index<&str>`).
- `Meta::renamed_headers` is a `Vec<(String, String)>`; it still serializes as an object.
- Removed the `indexmap` and `thiserror` dependencies.
- wasm package: `meta.cursor`/`error.index` are UTF-16 offsets and dates are `Date` objects,
  matching PapaParse.

### Added
- `papaparse-node`: native Node.js addon built with NAPI-RS. 1.3–3× faster than PapaParse in
  Node, passes the upstream fixtures and differential fuzzing; CI on Linux, macOS, Windows.
  Built with `panic = "unwind"` + `catch_unwind` so a panic becomes a JS exception instead
  of aborting Node.
- `papaparse-tape`: option mapping, result tape and JS decoder shared by both JS packages.
- `parsed_slice()`: the part of the input that `parse()` tokenizes.
- Benchmarks against PapaParse (`just bench-compare`), wasm conformance and differential
  fuzzing in Node (`just wasm-test`), heap-accounting leak test, Miri job, size budget in CI.
- Core tokenizer ported from `Papa.Parser`: quotes, custom escape characters, comments,
  preview, fast mode, step callbacks with abort, duplicate-header renaming.
- `parse()` ported from `Papa.parse` for string input: delimiter and newline detection,
  headers, dynamic typing, `transform`, `transformHeader`, `skipEmptyLines`, `skipFirstNLines`.
- Conformance harness that runs PapaParse 5.7.0's test suite.
- `papaparse` CLI (JSON / NDJSON output) and the `papaparse-wasm` bindings.
