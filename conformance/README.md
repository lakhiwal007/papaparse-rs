# Conformance fixtures

`fixtures/*.json` are generated from PapaParse's `tests/test-cases.js` at the version in
`UPSTREAM_VERSION`. **Do not edit them by hand**; regenerate with `just sync-upstream`.

| File | Upstream suite | Runner |
|---|---|---|
| `core_parser.json` | `CORE_PARSER_TESTS` | `tests/conformance.rs` (core `Parser`) |
| `parse.json` | `PARSE_TESTS` | `tests/conformance.rs` (`parse`) |
| `unparse.json` | `UNPARSE_TESTS` | pending: `unparse` not ported |
| `parse_async.json` | `PARSE_ASYNC_TESTS` | pending: file/stream input not ported |
| `custom.json` | `CUSTOM_TESTS` | pending: callback-driven, needs hand ports |

`expected-failures.txt` lists `suite::description` IDs that are known to fail.
