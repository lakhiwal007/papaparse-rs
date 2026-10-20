//! Native Node.js addon built with NAPI-RS. The public API is `js/index.cjs`, which mirrors
//! `Papa.parse`:
//!
//! ```js
//! const { parse } = require('papaparse-node');
//! const { data, errors, meta } = parse('a,b\n1,2', { header: true, dynamicTyping: true });
//! ```
//!
//! Parsing runs natively; the result crosses into JS as a tape of typed arrays whose
//! memory is handed over without copying, and the shared decoder rebuilds rows with
//! `substring` slices of the original string (see `papaparse-tape`).

use std::cell::RefCell;
use std::collections::HashMap;

use napi::Either;
use napi::bindgen_prelude::{Float64Array, FnArgs, Function, Uint8Array, Uint32Array};
use napi_derive::napi;
use papaparse_tape::papaparse::{Hooks, UnparseHooks, Value};
use papaparse_tape::{
    BatchFlow, BoolOrString, ChunkResult, Options, Session, StepBatch, Tape, Typing, UnparseCells,
    UnparseOptions,
};

/// The non-callback options of `Papa.parse` (field names are converted to camelCase).
#[napi(object)]
#[derive(Debug, Default)]
pub struct ParseOptions {
    pub delimiter: Option<String>,
    pub newline: Option<String>,
    pub quote_char: Option<String>,
    pub escape_char: Option<String>,
    pub header: Option<bool>,
    pub dynamic_typing: Option<Either<bool, HashMap<String, bool>>>,
    pub preview: Option<f64>,
    pub comments: Option<Either<bool, String>>,
    pub skip_empty_lines: Option<Either<bool, String>>,
    #[napi(js_name = "skipFirstNLines")]
    pub skip_first_n_lines: Option<f64>,
    pub fast_mode: Option<bool>,
    pub delimiters_to_guess: Option<Vec<String>>,
}

/// A parse result in transport form; consumed by the shared `decode.cjs`.
#[napi(object)]
#[allow(missing_debug_implementations)] // NAPI-RS typed arrays don't implement Debug
pub struct NativeTape {
    pub tags: Uint8Array,
    pub spans: Uint32Array,
    pub numbers: Float64Array,
    pub row_lens: Uint32Array,
    pub strings: String,
    pub string_spans: Uint32Array,
    pub info: String,
}

fn bool_or_string(v: Option<Either<bool, String>>) -> Option<BoolOrString> {
    v.map(|v| match v {
        Either::A(b) => BoolOrString::Bool(b),
        Either::B(s) => BoolOrString::String(s),
    })
}

impl From<ParseOptions> for Options {
    fn from(o: ParseOptions) -> Self {
        Options {
            delimiter: o.delimiter,
            newline: o.newline,
            quote_char: o.quote_char,
            escape_char: o.escape_char,
            header: o.header.unwrap_or(false),
            dynamic_typing: o.dynamic_typing.map(|t| match t {
                Either::A(b) => Typing::All(b),
                Either::B(fields) => Typing::Fields(fields),
            }),
            preview: o.preview,
            comments: bool_or_string(o.comments),
            skip_empty_lines: bool_or_string(o.skip_empty_lines),
            skip_first_n_lines: o.skip_first_n_lines,
            fast_mode: o.fast_mode,
            delimiters_to_guess: o.delimiters_to_guess.unwrap_or_default(),
        }
    }
}

impl NativeTape {
    // `TypedArray::new` takes ownership of each Vec: JS sees the same memory, no copy.
    fn new(tape: Tape, info: String) -> Self {
        Self {
            tags: Uint8Array::new(tape.tags),
            spans: Uint32Array::new(tape.spans),
            numbers: Float64Array::new(tape.numbers),
            row_lens: Uint32Array::new(tape.row_lens),
            strings: tape.strings,
            string_spans: Uint32Array::new(tape.string_spans),
            info,
        }
    }
}

/// Parses CSV text into a [`NativeTape`]. Use the `parse` export of `index.cjs` instead.
#[napi(catch_unwind)]
#[allow(clippy::needless_pass_by_value)] // N-API hands us owned values
pub fn parse_tape(input: String, config: Option<ParseOptions>) -> napi::Result<NativeTape> {
    let (tape, info) = papaparse_tape::run(&input, config.unwrap_or_default().into())
        .map_err(napi::Error::from_reason)?;
    Ok(NativeTape::new(tape, info))
}

/// Rows for a JS `step` callback (see `papaparse_tape::StepBatch`).
#[napi(object)]
#[allow(missing_debug_implementations)] // NAPI-RS typed arrays don't implement Debug
pub struct StepRows {
    pub tags: Uint8Array,
    pub spans: Uint32Array,
    pub numbers: Float64Array,
    pub row_lens: Uint32Array,
    pub strings: String,
    pub string_spans: Uint32Array,
    pub cursors: Float64Array,
    pub char_indexes: Uint32Array,
    pub errors: Option<String>,
    pub meta: String,
}

impl From<StepBatch> for StepRows {
    fn from(b: StepBatch) -> Self {
        Self {
            tags: Uint8Array::new(b.tape.tags),
            spans: Uint32Array::new(b.tape.spans),
            numbers: Float64Array::new(b.tape.numbers),
            row_lens: Uint32Array::new(b.tape.row_lens),
            strings: b.tape.strings,
            string_spans: Uint32Array::new(b.tape.string_spans),
            cursors: Float64Array::new(b.cursors),
            char_indexes: Uint32Array::new(b.char_indexes),
            errors: b.errors,
            meta: b.meta,
        }
    }
}

/// Parse state across the chunks of one streamed `Papa.parse` call (see `papa.cjs`).
#[napi]
#[allow(missing_debug_implementations)] // holds no JS handles; Session is Debug
pub struct ParseSession {
    inner: Session,
}

#[napi]
impl ParseSession {
    /// A session for the (callback-free) config.
    #[napi(constructor)]
    pub fn new(config: Option<ParseOptions>) -> napi::Result<Self> {
        Ok(Self {
            inner: Session::new(config.unwrap_or_default().into())
                .map_err(napi::Error::from_reason)?,
        })
    }

    /// Parses one chunk. In step mode `on_rows(batch)` receives [`StepRows`] and returns
    /// -1 (all delivered), -2 (aborted) or the index of the row it paused after.
    #[napi(catch_unwind)]
    #[allow(clippy::needless_pass_by_value)] // N-API hands us owned values
    pub fn parse(
        &mut self,
        input: String,
        js_base: f64,
        ignore_last_row: bool,
        transform_header: Option<Function<'_, FnArgs<(String, u32)>, String>>,
        on_rows: Option<Function<'_, StepRows, i32>>,
    ) -> napi::Result<NativeTape> {
        // After a callback throws, skip further calls; the pending exception is rethrown.
        let mut thrown: Option<napi::Error> = None;
        let mut header_hook = |h: &str, i: usize| -> String {
            let Some(f) = &transform_header else {
                return h.to_owned();
            };
            if thrown.is_some() {
                return String::new();
            }
            f.call((h.to_owned(), u32::try_from(i).unwrap_or(u32::MAX)).into())
                .unwrap_or_else(|e| {
                    thrown = Some(e);
                    String::new()
                })
        };
        let hooks = Hooks {
            transform_header: transform_header.is_some().then_some(&mut header_hook as _),
            ..Hooks::default()
        };
        let mut row_error: Option<napi::Error> = None;
        let mut step = |b: StepBatch| -> BatchFlow {
            let Some(f) = &on_rows else {
                return BatchFlow::Continue;
            };
            match f.call(StepRows::from(b)) {
                Ok(i) if i >= 0 => BatchFlow::Pause(usize::try_from(i).unwrap_or(usize::MAX)),
                Ok(-1) => BatchFlow::Continue,
                Ok(_) => BatchFlow::Abort,
                Err(e) => {
                    row_error = Some(e);
                    BatchFlow::Abort
                }
            }
        };
        let step: Option<&mut dyn FnMut(StepBatch) -> BatchFlow> = if on_rows.is_some() {
            Some(&mut step)
        } else {
            None
        };
        let result = self
            .inner
            .parse(&input, js_offset(js_base), ignore_last_row, hooks, step);
        if let Some(e) = thrown.or(row_error) {
            return Err(e);
        }
        let ChunkResult { tape, info } = result.map_err(napi::Error::from_reason)?;
        Ok(NativeTape::new(tape, info))
    }
}

/// A JS string offset (a non-negative integer below 2^53) as `usize`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn js_offset(n: f64) -> usize {
    n.max(0.0) as usize
}

/// Detects the line ending like PapaParse's `guessLineEndings` (for `skipFirstNLines`).
#[napi]
#[allow(clippy::needless_pass_by_value)] // N-API hands us owned values
pub fn guess_line_endings(input: String, quote_char: String) -> String {
    let quote = quote_char.chars().next().unwrap_or('"');
    papaparse_tape::papaparse::guess_line_endings(&input, quote)
        .as_str()
        .to_owned()
}

/// The unparse options `papa.cjs` normalized.
#[napi(object)]
#[derive(Debug, Default)]
pub struct UnparseOptionsJs {
    pub quotes: Option<bool>,
    pub quote_columns: Option<Vec<bool>>,
    pub header: Option<bool>,
    pub delimiter: Option<String>,
    pub newline: Option<String>,
    pub quote_char: Option<String>,
    pub escape_char: Option<String>,
    pub skip_empty_lines: Option<Either<bool, String>>,
    pub escape_formulae: Option<bool>,
}

/// Writes CSV from cells flattened by `papa.cjs` (see `papaparse_tape::UnparseCells`).
/// `quotes(col, row)` (row -1 for the header) and `escape_formulae(text)` are the JS
/// callbacks, if any.
#[napi(catch_unwind)]
#[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)] // N-API ABI
pub fn unparse(
    fields: Vec<String>,
    text: String,
    lens: Uint32Array,
    kinds: Uint8Array,
    numbers: Float64Array,
    row_lens: Uint32Array,
    row_widths: Uint32Array,
    options: UnparseOptionsJs,
    quotes: Option<Function<'_, FnArgs<(u32, f64)>, bool>>,
    escape_formulae: Option<Function<'_, String, bool>>,
) -> napi::Result<String> {
    let thrown = RefCell::new(None::<napi::Error>);
    let failed = |r: napi::Result<bool>| {
        r.map_err(|e| *thrown.borrow_mut() = Some(e))
            .unwrap_or(false)
    };
    // `quotes(col, row)`: `papa.cjs` passes its callback the original cell value.
    let mut quotes_hook = |_: &Value<'_>, col: usize, row: Option<usize>| {
        let row = row.map_or(-1.0, |r| r as f64);
        quotes.as_ref().is_some_and(|f| {
            thrown.borrow().is_none()
                && failed(f.call((u32::try_from(col).unwrap_or(u32::MAX), row).into()))
        })
    };
    let mut formula_hook = |t: &str| {
        escape_formulae
            .as_ref()
            .is_some_and(|f| thrown.borrow().is_none() && failed(f.call(t.to_owned())))
    };
    let options = UnparseOptions {
        quotes: options.quotes,
        quote_columns: options.quote_columns,
        header: options.header,
        delimiter: options.delimiter,
        newline: options.newline,
        quote_char: options.quote_char,
        escape_char: options.escape_char,
        skip_empty_lines: bool_or_string(options.skip_empty_lines),
        escape_formulae: options.escape_formulae,
    };
    let result = papaparse_tape::unparse(
        &fields,
        &UnparseCells {
            text: &text,
            lens: &lens,
            kinds: &kinds,
            numbers: &numbers,
            row_lens: &row_lens,
            row_widths: &row_widths,
        },
        options,
        UnparseHooks {
            quotes: quotes.is_some().then_some(&mut quotes_hook as _),
            escape_formulae: escape_formulae.is_some().then_some(&mut formula_hook as _),
        },
    );
    if let Some(e) = thrown.into_inner() {
        return Err(e);
    }
    result.map_err(napi::Error::from_reason)
}
