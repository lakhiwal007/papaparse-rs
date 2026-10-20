//! Chunk-by-chunk parsing for the JS streamers (callbacks, `chunkSize`, Node streams).
//!
//! `js/papa.cjs` ports PapaParse's `ChunkStreamer` family verbatim; for each chunk it
//! calls [`Session::parse`] the way PapaParse calls `ParserHandle.parse`. Offsets cross
//! the boundary in UTF-16 code units, like every JS string index.
//!
//! In step mode rows travel in batches ([`StepBatch`]): one call into JS per few hundred
//! rows instead of one per row. JS hands rows to the user's `step` one at a time and says
//! where it stopped, so `abort` and `pause` behave as if rows had come singly.

use papaparse::{Flow, Hooks, Meta, ParseError, ParseOutput, ParserHandle, Row, Step};

use crate::info::{errors_json, meta_json};
use crate::options::Options;
use crate::tape::{Tape, Utf16Offsets};

/// Rows per [`StepBatch`].
const STEP_BATCH: usize = 512;

/// Rows for a JS `step` callback.
#[derive(Debug)]
pub struct StepBatch {
    /// The rows, encoded against the chunk's input (as in [`ChunkResult`]).
    pub tape: Tape,
    /// Per row: stream offset just past it (JS `meta.cursor`), in UTF-16 units.
    pub cursors: Vec<f64>,
    /// Per row: offset just past it within the chunk's input (JS `getCharIndex()`,
    /// used by `pause`), in UTF-16 units.
    pub char_indexes: Vec<u32>,
    /// `{"<row in batch>":[errors...]}` for the rows that have errors, if any.
    pub errors: Option<String>,
    /// `meta` without `cursor`, as JSON (the same for every row of a chunk).
    pub meta: String,
}

/// What JS did with a [`StepBatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchFlow {
    /// All rows delivered; go on.
    Continue,
    /// The user aborted.
    Abort,
    /// The user paused after the row at this index; later rows must come again.
    Pause(usize),
}

/// A chunk's rows (none in step mode) and its `{"errors":[...],"meta":{...}}` JSON.
#[derive(Debug)]
pub struct ChunkResult {
    /// The rows, encoded against the chunk's input.
    pub tape: Tape,
    /// Errors and meta of the chunk.
    pub info: String,
}

/// Parse state across the chunks of one JS `Papa.parse` call.
#[derive(Debug)]
pub struct Session {
    handle: ParserHandle,
    /// Byte offset in the stream of the next chunk's input.
    base: usize,
}

impl Session {
    /// A session for `options`; `Err` carries a message for a JS `Error`.
    pub fn new(options: Options) -> Result<Self, String> {
        let config = options.into_config()?;
        papaparse::Parser::new(&config).map_err(|e| e.to_string())?;
        Ok(Self {
            handle: ParserHandle::new(config),
            base: 0,
        })
    }

    /// Parses one chunk. `js_base` is where `input` starts in the stream (JS
    /// `_baseIndex`, UTF-16 units). With `step`, rows go to the callback in batches; after
    /// a pause, the rest of `input` (from the paused row's `char_index`) must start the
    /// next call.
    pub fn parse<'a>(
        &mut self,
        input: &'a str,
        js_base: usize,
        ignore_last_row: bool,
        hooks: Hooks<'_>,
        step: Option<&mut dyn FnMut(StepBatch) -> BatchFlow>,
    ) -> Result<ChunkResult, String> {
        let base = self.base;
        let mut paused = None;

        let out = match step {
            None => self
                .handle
                .parse(input, base, ignore_last_row, hooks, None)
                .map_err(|e| e.to_string())?,
            Some(user_step) => {
                let mut batch = Batch::new(input, base, js_base);
                let mut stopped = false;
                let mut on_row = |s: Step<'a>| {
                    batch.push(s);
                    if batch.rows.len() < STEP_BATCH {
                        return Flow::Continue;
                    }
                    match batch.flush(user_step) {
                        Ok(()) => Flow::Continue,
                        Err(pause) => {
                            stopped = true;
                            paused = pause;
                            if paused.is_some() {
                                Flow::Pause
                            } else {
                                Flow::Abort
                            }
                        }
                    }
                };
                let out = self
                    .handle
                    .parse(input, base, ignore_last_row, hooks, Some(&mut on_row))
                    .map_err(|e| e.to_string())?;
                if !stopped && !batch.rows.is_empty() {
                    paused = batch.flush(user_step).err().flatten();
                }
                out
            }
        };

        match paused {
            Some(Paused {
                cursor,
                undelivered,
            }) => {
                self.base = cursor;
                self.handle.unstep(undelivered);
            }
            None => self.base = out.meta.cursor,
        }
        let mut to_utf16 = Utf16Offsets::new(input);
        let cursor = js_base + to_utf16.convert(out.meta.cursor - base);
        let fields = out.meta.fields.as_deref();
        let info = format!(
            "{{\"errors\":{},\"meta\":{}}}",
            errors_json(&out.errors, &mut |b| to_utf16.convert(b)),
            meta_json(&out.meta, Some(cursor), fields)
        );
        Ok(ChunkResult {
            tape: Tape::encode(input, &out),
            info,
        })
    }

    /// Whether a step callback was stopped because `preview` rows were delivered.
    pub fn preview_reached(&self) -> bool {
        self.handle.preview_reached()
    }
}

/// Where to resume after a pause part-way through a batch.
#[derive(Debug, Clone, Copy)]
struct Paused {
    /// Stream byte offset just past the last row delivered.
    cursor: usize,
    /// Rows parsed after it, which must be parsed and delivered again.
    undelivered: usize,
}

/// Step rows waiting to go to JS.
struct Batch<'a> {
    input: &'a str,
    base: usize,
    js_base: usize,
    rows: Vec<Row<'a>>,
    /// Stream byte offset just past each row.
    cursors: Vec<usize>,
    errors: Vec<(usize, Vec<ParseError>)>,
    meta: Option<(Meta, Option<Vec<String>>)>,
    to_utf16: Utf16Offsets<'a>,
    errors_to_utf16: Utf16Offsets<'a>,
}

impl<'a> Batch<'a> {
    fn new(input: &'a str, base: usize, js_base: usize) -> Self {
        Self {
            input,
            base,
            js_base,
            rows: Vec::with_capacity(STEP_BATCH),
            cursors: Vec::with_capacity(STEP_BATCH),
            errors: Vec::new(),
            meta: None,
            to_utf16: Utf16Offsets::new(input),
            errors_to_utf16: Utf16Offsets::new(input),
        }
    }

    fn push(&mut self, s: Step<'a>) {
        if self.meta.is_none() {
            let fields = match &s.row {
                Row::Record(r) => Some(r.header().to_vec()),
                Row::Fields(_) => None,
            };
            self.meta = Some((s.meta.clone(), fields));
        }
        if !s.errors.is_empty() {
            self.errors.push((self.rows.len(), s.errors));
        }
        self.cursors.push(s.meta.cursor);
        self.rows.push(s.row);
    }

    /// Hands the rows to JS. `Err(Some(_))` is a pause, `Err(None)` an abort.
    fn flush(
        &mut self,
        user_step: &mut dyn FnMut(StepBatch) -> BatchFlow,
    ) -> Result<(), Option<Paused>> {
        let rows = std::mem::take(&mut self.rows);
        // One converter for the spans and cursors of every batch: both only move forward.
        let tape = Tape::encode_with(
            &mut self.to_utf16,
            self.input,
            &ParseOutput {
                data: rows,
                ..ParseOutput::default()
            },
        );
        let mut char_indexes = Vec::with_capacity(self.cursors.len());
        let mut cursors = Vec::with_capacity(self.cursors.len());
        for &c in &self.cursors {
            let rel = self.to_utf16.convert(c - self.base);
            char_indexes.push(u32::try_from(rel).unwrap_or(u32::MAX));
            cursors.push((self.js_base + rel) as f64);
        }
        let errors = (!self.errors.is_empty()).then(|| {
            let conv = &mut self.errors_to_utf16;
            let parts: Vec<String> = self
                .errors
                .drain(..)
                .map(|(i, e)| format!("\"{i}\":{}", errors_json(&e, &mut |b| conv.convert(b))))
                .collect();
            format!("{{{}}}", parts.join(","))
        });
        let meta = match &self.meta {
            Some((m, fields)) => meta_json(m, None, fields.as_deref()),
            None => String::from("{}"),
        };

        let flow = user_step(StepBatch {
            tape,
            cursors,
            char_indexes,
            errors,
            meta,
        });
        let delivered = std::mem::take(&mut self.cursors);
        match flow {
            BatchFlow::Continue => Ok(()),
            BatchFlow::Abort => Err(None),
            BatchFlow::Pause(k) => {
                let k = k.min(delivered.len().saturating_sub(1));
                Err(Some(Paused {
                    cursor: delivered.get(k).copied().unwrap_or(self.base),
                    undelivered: delivered.len() - k - 1,
                }))
            }
        }
    }
}
