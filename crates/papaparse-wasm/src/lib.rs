//! WebAssembly bindings. The public API is `js/index.cjs`, which mirrors `Papa.parse`:
//!
//! ```js
//! const { parse } = require('papaparse-wasm');
//! const { data, errors, meta } = parse('a,b\n1,2', { header: true, dynamicTyping: true });
//! ```
//!
//! This crate only reads JS values into `papaparse-tape` types and calls back into JS;
//! parsing, the result tape and the JS layer (`papa.cjs`) are shared with the Node-API
//! package.

use std::cell::RefCell;

use js_sys::{Array, Function, JsString, Object, Reflect};
use papaparse_tape::papaparse::{Hooks, UnparseHooks, Value};
use papaparse_tape::{
    BatchFlow, BoolOrString, Options, Session, StepBatch, Tape, Typing, UnparseCells,
    UnparseOptions,
};
use wasm_bindgen::prelude::*;

/// Parse result in transport form; consumed once by `js/index.cjs`.
#[wasm_bindgen]
#[derive(Debug)]
pub struct ParseTape {
    tape: Tape,
    info: String,
}

#[wasm_bindgen]
impl ParseTape {
    /// Cell tags (see `papaparse_tape::tag`).
    pub fn take_tags(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.tape.tags)
    }
    /// UTF-16 `(start, end)` offsets into the input for text cells.
    pub fn take_spans(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.tape.spans)
    }
    /// Values of number cells.
    pub fn take_numbers(&mut self) -> Vec<f64> {
        std::mem::take(&mut self.tape.numbers)
    }
    /// Cells per row.
    pub fn take_row_lens(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.tape.row_lens)
    }
    /// Side table for text that is not a slice of the input.
    pub fn take_strings(&mut self) -> String {
        std::mem::take(&mut self.tape.strings)
    }
    /// UTF-16 `(start, end)` offsets into the side table.
    pub fn take_string_spans(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.tape.string_spans)
    }
    /// `{"errors":[...],"meta":{...}}` as JSON.
    pub fn take_info(&mut self) -> String {
        std::mem::take(&mut self.info)
    }
}

/// Parses CSV text into a [`ParseTape`]. Use the `parse` export of `index.cjs` instead.
#[wasm_bindgen]
pub fn parse_tape(input: &str, config: &JsValue) -> Result<ParseTape, JsError> {
    let (tape, info) =
        papaparse_tape::run(input, read_options(config)).map_err(|e| JsError::new(&e))?;
    Ok(ParseTape { tape, info })
}

/// Parse state across the chunks of one streamed `Papa.parse` call (see `papa.cjs`).
#[wasm_bindgen]
#[derive(Debug)]
pub struct ParseSession {
    inner: Session,
}

#[wasm_bindgen]
impl ParseSession {
    /// A session for the (callback-free) config.
    #[wasm_bindgen(constructor)]
    pub fn new(config: &JsValue) -> Result<ParseSession, JsError> {
        Ok(Self {
            inner: Session::new(read_options(config)).map_err(|e| JsError::new(&e))?,
        })
    }

    /// Parses one chunk. In step mode `on_rows(batch)` receives [`StepRows`] and returns
    /// -1 (all delivered), -2 (aborted) or the index of the row it paused after.
    #[allow(clippy::needless_pass_by_value)] // wasm-bindgen hands over owned JS handles
    pub fn parse(
        &mut self,
        input: &str,
        js_base: f64,
        ignore_last_row: bool,
        transform_header: Option<Function>,
        on_rows: Option<Function>,
    ) -> Result<ParseTape, JsValue> {
        let mut thrown: Option<JsValue> = None;
        let mut header_hook = |h: &str, i: usize| -> String {
            let Some(f) = &transform_header else {
                return h.to_owned();
            };
            if thrown.is_some() {
                return String::new();
            }
            match f.call2(&JsValue::UNDEFINED, &h.into(), &(i as f64).into()) {
                Ok(v) => js_to_string(&v),
                Err(e) => {
                    thrown = Some(e);
                    String::new()
                }
            }
        };
        let hooks = Hooks {
            transform_header: transform_header.is_some().then_some(&mut header_hook as _),
            ..Hooks::default()
        };
        let mut row_error: Option<JsValue> = None;
        let mut step = |b: StepBatch| -> BatchFlow {
            let Some(f) = &on_rows else {
                return BatchFlow::Continue;
            };
            match f.call1(&JsValue::UNDEFINED, &StepRows::from(b).into()) {
                Ok(v) => batch_flow(v.as_f64()),
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
            .parse(input, js_offset(js_base), ignore_last_row, hooks, step)
            .map_err(|e| JsValue::from(JsError::new(&e)));
        if let Some(e) = thrown.or(row_error) {
            return Err(e);
        }
        let chunk = result?;
        Ok(ParseTape {
            tape: chunk.tape,
            info: chunk.info,
        })
    }
}

/// Rows for a JS `step` callback (see `papaparse_tape::StepBatch`); consumed once.
#[wasm_bindgen]
#[derive(Debug)]
pub struct StepRows {
    batch: StepBatch,
}

impl From<StepBatch> for StepRows {
    fn from(batch: StepBatch) -> Self {
        Self { batch }
    }
}

#[wasm_bindgen]
impl StepRows {
    /// Cell tags.
    pub fn take_tags(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.batch.tape.tags)
    }
    /// UTF-16 `(start, end)` offsets into the chunk's input.
    pub fn take_spans(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.batch.tape.spans)
    }
    /// Values of number cells.
    pub fn take_numbers(&mut self) -> Vec<f64> {
        std::mem::take(&mut self.batch.tape.numbers)
    }
    /// Cells per row.
    pub fn take_row_lens(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.batch.tape.row_lens)
    }
    /// Side table for text that is not a slice of the input.
    pub fn take_strings(&mut self) -> String {
        std::mem::take(&mut self.batch.tape.strings)
    }
    /// UTF-16 `(start, end)` offsets into the side table.
    pub fn take_string_spans(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.batch.tape.string_spans)
    }
    /// Per row: stream offset just past it.
    pub fn take_cursors(&mut self) -> Vec<f64> {
        std::mem::take(&mut self.batch.cursors)
    }
    /// Per row: offset just past it within the chunk's input.
    pub fn take_char_indexes(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.batch.char_indexes)
    }
    /// `{"<row>":[errors]}` JSON, if any row has errors.
    pub fn take_errors(&mut self) -> Option<String> {
        self.batch.errors.take()
    }
    /// `meta` JSON without `cursor`.
    pub fn take_meta(&mut self) -> String {
        std::mem::take(&mut self.batch.meta)
    }
}

/// Decodes `on_rows`' answer: -1 all delivered, -2 aborted, else the paused row.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn batch_flow(v: Option<f64>) -> BatchFlow {
    match v {
        Some(i) if i >= 0.0 => BatchFlow::Pause(i as usize),
        Some(i) if i <= -2.0 => BatchFlow::Abort,
        _ => BatchFlow::Continue,
    }
}

/// A JS string offset (a non-negative integer below 2^53) as `usize`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn js_offset(n: f64) -> usize {
    n.max(0.0) as usize
}

/// JS `String(v)`.
fn js_to_string(v: &JsValue) -> String {
    v.as_string()
        .unwrap_or_else(|| JsString::from(v.clone()).into())
}

/// Detects the line ending like PapaParse's `guessLineEndings` (for `skipFirstNLines`).
#[wasm_bindgen]
pub fn guess_line_endings(input: &str, quote_char: &str) -> String {
    let quote = quote_char.chars().next().unwrap_or('"');
    papaparse_tape::papaparse::guess_line_endings(input, quote)
        .as_str()
        .to_owned()
}

/// Writes CSV from cells flattened by `papa.cjs` (see `papaparse_tape::UnparseCells`).
/// `quotes(col, row)` (row -1 for the header) and `escape_formulae(text)` are the JS
/// callbacks, if any.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)] // wasm-bindgen ABI
pub fn unparse(
    fields: Vec<String>,
    text: &str,
    lens: &[u32],
    kinds: &[u8],
    numbers: &[f64],
    row_lens: &[u32],
    row_widths: &[u32],
    options: &JsValue,
    quotes: Option<Function>,
    escape_formulae: Option<Function>,
) -> Result<String, JsValue> {
    let thrown = RefCell::new(None::<JsValue>);
    // After a callback throws, skip the remaining calls; the error is rethrown below.
    let call = |f: &Function, a: &JsValue, b: Option<&JsValue>| -> bool {
        if thrown.borrow().is_some() {
            return false;
        }
        let r = match b {
            Some(b) => f.call2(&JsValue::UNDEFINED, a, b),
            None => f.call1(&JsValue::UNDEFINED, a),
        };
        r.map_err(|e| *thrown.borrow_mut() = Some(e))
            .is_ok_and(|v| v.is_truthy())
    };
    // `quotes(col, row)`: `papa.cjs` passes its callback the original cell value.
    let mut quotes_hook = |_: &Value<'_>, col: usize, row: Option<usize>| {
        let row = row.map_or(-1.0, |r| r as f64);
        quotes
            .as_ref()
            .is_some_and(|f| call(f, &(col as f64).into(), Some(&row.into())))
    };
    let mut formula_hook = |t: &str| {
        escape_formulae
            .as_ref()
            .is_some_and(|f| call(f, &t.into(), None))
    };
    let result = papaparse_tape::unparse(
        &fields,
        &UnparseCells {
            text,
            lens,
            kinds,
            numbers,
            row_lens,
            row_widths,
        },
        read_unparse_options(options),
        UnparseHooks {
            quotes: quotes.is_some().then_some(&mut quotes_hook as _),
            escape_formulae: escape_formulae.is_some().then_some(&mut formula_hook as _),
        },
    );
    if let Some(e) = thrown.into_inner() {
        return Err(e);
    }
    result.map_err(|e| JsError::new(&e).into())
}

/// Reads the unparse options `papa.cjs` normalized.
fn read_unparse_options(js: &JsValue) -> UnparseOptions {
    let string = |key| get(js, key).and_then(|v| v.as_string());
    let boolean = |key| get(js, key).and_then(|v| v.as_bool());
    UnparseOptions {
        quotes: boolean("quotes"),
        quote_columns: get(js, "quoteColumns").filter(Array::is_array).map(|v| {
            v.unchecked_into::<Array>()
                .iter()
                .map(|b| b.is_truthy())
                .collect()
        }),
        header: boolean("header"),
        delimiter: string("delimiter"),
        newline: string("newline"),
        quote_char: string("quoteChar"),
        escape_char: string("escapeChar"),
        skip_empty_lines: bool_or_string(get(js, "skipEmptyLines")),
        escape_formulae: boolean("escapeFormulae"),
    }
}

fn get(obj: &JsValue, key: &str) -> Option<JsValue> {
    Reflect::get(obj, &JsValue::from_str(key))
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null())
}

fn bool_or_string(v: Option<JsValue>) -> Option<BoolOrString> {
    let v = v?;
    v.as_bool()
        .map(BoolOrString::Bool)
        .or_else(|| v.as_string().map(BoolOrString::String))
}

/// Reads the non-callback options of PapaParse's config object.
fn read_options(js: &JsValue) -> Options {
    if !js.is_object() {
        return Options::default();
    }
    let string = |key| get(js, key).and_then(|v| v.as_string());
    let number = |key| get(js, key).and_then(|v| v.as_f64());

    let dynamic_typing = get(js, "dynamicTyping").and_then(|v| {
        if let Some(b) = v.as_bool() {
            Some(Typing::All(b))
        } else if v.is_object() && !v.is_function() {
            let fields = Object::keys(v.unchecked_ref::<Object>())
                .iter()
                .map(|key| {
                    let key = JsString::from(key).as_string().unwrap_or_default();
                    let on = get(&v, &key).is_some_and(|b| b.as_bool() == Some(true));
                    (key, on)
                })
                .collect();
            Some(Typing::Fields(fields))
        } else {
            None
        }
    });

    Options {
        delimiter: string("delimiter"),
        newline: string("newline"),
        quote_char: string("quoteChar"),
        escape_char: string("escapeChar"),
        header: get(js, "header").is_some_and(|v| v.is_truthy()),
        dynamic_typing,
        preview: number("preview"),
        comments: bool_or_string(get(js, "comments")),
        skip_empty_lines: bool_or_string(get(js, "skipEmptyLines")),
        skip_first_n_lines: number("skipFirstNLines"),
        fast_mode: get(js, "fastMode").and_then(|v| v.as_bool()),
        delimiters_to_guess: get(js, "delimitersToGuess")
            .filter(Array::is_array)
            .map(|v| {
                v.unchecked_into::<Array>()
                    .iter()
                    .filter_map(|d| d.as_string())
                    .collect()
            })
            .unwrap_or_default(),
    }
}
