//! A Rust port of [PapaParse](https://www.papaparse.com/), the in-browser CSV parser.
//!
//! The goal is behavioural parity with the JavaScript library: the same inputs and
//! options produce the same rows, errors and metadata. Parity is enforced by running
//! PapaParse's own test suite against this crate (see `conformance/` in the repository).
//!
//! # Quick start
//!
//! ```
//! use papaparse::{parse, ParseConfig, Row, Value};
//!
//! let out = parse("name,age\nAda,36\n", &ParseConfig::default().header(true).dynamic_typing(true))?;
//! assert_eq!(out.meta.fields.as_deref(), Some(&["name".to_string(), "age".to_string()][..]));
//! let Row::Record(first) = &out.data[0] else { unreachable!() };
//! assert_eq!(first["age"], Value::Number(36.0));
//! assert_eq!(first.get("missing"), None);
//! # Ok::<(), papaparse::ConfigError>(())
//! ```
//!
//! # Layers
//!
//! * [`parse`] mirrors `Papa.parse(string, config)`: delimiter/newline detection,
//!   headers, dynamic typing, transforms and empty-line skipping.
//! * [`ChunkParser`], [`parse_reader`] and [`parse_reader_rows`] mirror `Papa.parse`
//!   on streams (`chunk` and `step` callbacks): input is parsed piece by piece in
//!   bounded memory.
//! * [`unparse`] and [`unparse_table`] mirror `Papa.unparse`: rows back to CSV.
//! * [`Parser`] mirrors `new Papa.Parser(config)`: the low-level tokenizer that yields
//!   raw string rows, with optional per-row stepping and early abort.
//!
//! # Differences from JavaScript
//!
//! Offsets (`meta.cursor`, `error.index`) are UTF-8 **byte** offsets, whereas PapaParse
//! reports UTF-16 code-unit offsets. They agree for ASCII input.
//!
//! # Performance and robustness
//!
//! Parsing is linear in the input size for every input: substring searches are SIMD
//! accelerated (`memchr`) and memoized, which removes the quadratic rescans present in
//! the JavaScript algorithm. The crate contains no `unsafe` code.

#![warn(missing_docs)]

mod config;
mod core;
mod error;
mod guess;
mod handle;
mod search;
mod stream;
mod typing;
mod unparse;
mod value;

pub use crate::config::{
    Comments, Delimiter, DelimiterFn, DynamicTyping, DynamicTypingFn, DynamicTypingHook, Field,
    Newline, ParseConfig, SkipEmptyLines, TransformFn, TransformHeaderFn, TransformHeaderHook,
    TransformHook,
};
pub use crate::core::{Flow, Parser, RawOutput};
pub use crate::error::{ConfigError, ErrorCode, ErrorType, ParseError};
pub use crate::guess::{guess_delimiter, guess_line_endings};
pub use crate::handle::{
    Hooks, ParseOutput, ParserHandle, Step, parse, parse_with_hooks, parsed_slice,
};
pub use crate::stream::{ChunkParser, DEFAULT_CHUNK_SIZE, Sink, parse_reader, parse_reader_rows};
pub use crate::unparse::{
    EscapeFormulae, FormulaFn, FormulaHook, Quotes, QuotesFn, QuotesHook, UnparseConfig,
    UnparseHooks, UnparseRow, unparse, unparse_table, unparse_with_hooks,
};
pub use crate::value::{Meta, PARSED_EXTRA, Record, Row, Value, as_safe_integer};

/// ASCII record separator, one of the delimiters tried during auto-detection.
pub const RECORD_SEP: &str = "\u{1e}";
/// ASCII unit separator, one of the delimiters tried during auto-detection.
pub const UNIT_SEP: &str = "\u{1f}";
/// UTF-8 byte order mark, stripped from the start of input.
pub const BYTE_ORDER_MARK: char = '\u{feff}';
/// Strings that can never be used as a delimiter or comment marker.
pub const BAD_DELIMITERS: [&str; 4] = ["\r", "\n", "\"", "\u{feff}"];
/// Whitespace as JavaScript's `String.prototype.trim` and regex `\s` define it
/// (ECMAScript `WhiteSpace` + `LineTerminator`). Differs from [`char::is_whitespace`]:
/// U+FEFF counts, U+0085 does not.
pub(crate) fn is_js_whitespace(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// Delimiter used when none is given and auto-detection fails.
pub const DEFAULT_DELIMITER: &str = ",";
