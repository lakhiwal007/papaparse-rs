//! Port of `Papa.parse` for in-memory string input (papaparse.js `CsvToJson`,
//! `StringStreamer` and `ParserHandle`).

use std::borrow::Cow;
use std::sync::Arc;

use crate::config::{
    Delimiter, DynamicTyping, DynamicTypingHook, Field, ParseConfig, SkipEmptyLines,
    TransformHeaderHook, TransformHook,
};
use crate::core::{Flow, Parser, RawOutput};
use crate::error::{ConfigError, ErrorCode, ErrorType, ParseError};
use crate::guess::{guess_delimiter, guess_line_endings, is_empty_line};
use crate::search::Finder;
use crate::typing::parse_dynamic;
use crate::value::{Header, Meta, PARSED_EXTRA, Record, Row, Value};
use crate::{BYTE_ORDER_MARK, DEFAULT_DELIMITER};

/// Result of [`parse`]. JS: the `results` object.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ParseOutput<'a> {
    /// Parsed rows. Text borrows from the input where possible.
    pub data: Vec<Row<'a>>,
    /// Recoverable problems found while parsing.
    pub errors: Vec<ParseError>,
    /// Information about the run.
    pub meta: Meta,
}

impl ParseOutput<'_> {
    /// Copies borrowed text so the output no longer borrows the input.
    pub fn into_owned(self) -> ParseOutput<'static> {
        ParseOutput {
            data: self.data.into_iter().map(Row::into_owned).collect(),
            errors: self.errors,
            meta: self.meta,
        }
    }
}

/// One row delivered to a step callback. JS: the `results` passed to `config.step`.
///
/// `meta.fields` is left unset to avoid copying the header for every row; a
/// [`Row::Record`] carries its header ([`Record::header`]).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Step<'a> {
    /// The row (JS: `results.data`).
    pub row: Row<'a>,
    /// Problems found in this row.
    pub errors: Vec<ParseError>,
    /// Information about the run so far; `cursor` is just past this row.
    pub meta: Meta,
}

/// Callbacks for one parse that may borrow local state, unlike the
/// `Send + Sync + 'static` callbacks of [`ParseConfig`]. A hook that is set replaces
/// the matching [`ParseConfig`] option. Language bindings use these to call into their host.
#[derive(Default)]
pub struct Hooks<'h> {
    /// Replaces [`ParseConfig::transform`].
    pub transform: Option<TransformHook<'h>>,
    /// Replaces [`ParseConfig::transform_header`].
    pub transform_header: Option<TransformHeaderHook<'h>>,
    /// Replaces [`ParseConfig::dynamic_typing`] with a per-column decision.
    pub dynamic_typing: Option<DynamicTypingHook<'h>>,
}

impl std::fmt::Debug for Hooks<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set = |b: bool| if b { "Some(<fn>)" } else { "None" };
        f.debug_struct("Hooks")
            .field("transform", &set(self.transform.is_some()))
            .field("transform_header", &set(self.transform_header.is_some()))
            .field("dynamic_typing", &set(self.dynamic_typing.is_some()))
            .finish()
    }
}

/// Parses CSV text. JS: `Papa.parse(string, config)`.
///
/// Parse problems (bad quotes, field-count mismatches, undetectable delimiter) are
/// reported in [`ParseOutput::errors`]; only an invalid configuration returns `Err`.
pub fn parse<'a>(input: &'a str, config: &ParseConfig) -> Result<ParseOutput<'a>, ConfigError> {
    parse_with_hooks(input, config, Hooks::default())
}

/// [`parse`] with [`Hooks`] that replace the matching config callbacks.
pub fn parse_with_hooks<'a>(
    input: &'a str,
    config: &ParseConfig,
    hooks: Hooks<'_>,
) -> Result<ParseOutput<'a>, ConfigError> {
    ParserHandle::new(config.clone()).parse(parsed_slice(input, config), 0, false, hooks, None)
}

/// Parse state that outlives one chunk of a stream. JS: `ParserHandle`.
///
/// The building block of [`crate::ChunkParser`], which also buffers the row a chunk
/// cuts in two; use this directly only to drive chunking yourself (as the JS
/// bindings do). The delimiter and newline are fixed by the first chunk, the header by
/// the first row.
pub struct ParserHandle {
    /// The caller's config; `delimiter` and `newline` are fixed by the first chunk.
    config: ParseConfig,
    /// Header row, once seen (only with `config.header`).
    header: Option<Arc<Header>>,
    /// Per-column "apply dynamic typing?" answers.
    typing: ColumnTyping,
    /// Rows processed so far, for `FieldMismatch` row numbers (JS: `_rowCounter`).
    row_counter: usize,
    /// Rows handed to a step callback (JS: `_stepCounter`).
    step_counter: usize,
    /// A step callback ran past `preview`.
    preview_reached: bool,
}

impl std::fmt::Debug for ParserHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParserHandle")
            .field("config", &self.config)
            .field("header", &self.header.as_ref().map(|h| h.names()))
            .field("row_counter", &self.row_counter)
            .field("step_counter", &self.step_counter)
            .finish_non_exhaustive()
    }
}

impl ParserHandle {
    /// A handle for a new stream.
    pub fn new(config: ParseConfig) -> Self {
        Self {
            config,
            header: None,
            typing: ColumnTyping::default(),
            row_counter: 0,
            step_counter: 0,
            preview_reached: false,
        }
    }

    /// The configuration, with the delimiter and newline filled in once the first
    /// chunk has been parsed.
    pub fn config(&self) -> &ParseConfig {
        &self.config
    }

    /// The header row, once parsed (with [`ParseConfig::header`]).
    pub fn header(&self) -> Option<&[String]> {
        self.header.as_deref().map(Header::names)
    }

    /// Whether a step callback was stopped because `preview` rows were delivered.
    pub fn preview_reached(&self) -> bool {
        self.preview_reached
    }

    /// Forgets the last `rows` rows handed to the step callback, for a caller that
    /// buffers rows and stops part-way through its buffer: the next chunk must then start
    /// after the last row it kept, and those rows are parsed and delivered again. Row
    /// numbers in errors and the `preview` count continue from the last row kept.
    pub fn unstep(&mut self, rows: usize) {
        // A row stopped by `preview` was counted but never delivered.
        let rows = rows + usize::from(std::mem::take(&mut self.preview_reached));
        self.row_counter = self.row_counter.saturating_sub(rows);
        self.step_counter = self.step_counter.saturating_sub(rows);
    }

    fn needs_header_row(&self) -> bool {
        self.config.header && self.header.is_none()
    }

    /// Parses one chunk. JS: `ParserHandle.parse`.
    ///
    /// `input` is the stream text from offset `base_index` (in bytes) on; with
    /// `ignore_last_row` a final row that may be incomplete is left unparsed, and
    /// `meta.cursor` says where the next chunk must start. With `step`, rows go to the
    /// callback as they are parsed and the returned output holds no rows.
    ///
    /// # Errors
    /// An invalid configuration (see [`ConfigError`]).
    pub fn parse<'a>(
        &mut self,
        input: &'a str,
        base_index: usize,
        ignore_last_row: bool,
        hooks: Hooks<'_>,
        step: Option<&mut dyn FnMut(Step<'a>) -> Flow>,
    ) -> Result<ParseOutput<'a>, ConfigError> {
        self.parse_impl(input, base_index, ignore_last_row, hooks, step, false)
    }

    /// [`ParserHandle::parse`]; see `Parser::parse_chunk_with` for `empty_row`.
    pub(crate) fn parse_impl<'a>(
        &mut self,
        input: &'a str,
        base_index: usize,
        ignore_last_row: bool,
        hooks: Hooks<'_>,
        step: Option<&mut dyn FnMut(Step<'a>) -> Flow>,
        empty_row: bool,
    ) -> Result<ParseOutput<'a>, ConfigError> {
        let mut delimiter_error = self.resolve_format(input);

        // Tell the parser about the header instead of re-guessing on each chunk, and
        // compensate `preview` for the header row.
        let (header, preview) = (self.config.header, self.config.preview);
        self.config.header = self.needs_header_row();
        if preview > 0 && header {
            self.config.preview = preview.saturating_add(1);
        }
        let parser = Parser::new(&self.config);
        (self.config.header, self.config.preview) = (header, preview);
        let mut parser = parser?;

        let Hooks {
            transform,
            transform_header,
            dynamic_typing,
        } = hooks;
        let mut cells = Cells {
            transform,
            dynamic_typing,
        };

        let Some(user_step) = step else {
            let raw = parser.parse_chunk_with(
                input,
                base_index,
                ignore_last_row,
                None,
                transform_header,
                empty_row,
            );
            return Ok(self.process(raw, delimiter_error, &mut cells, true));
        };

        let mut on_row = |raw: RawOutput<'a>| {
            let was_header = self.needs_header_row();
            let mut out =
                self.process(raw, std::mem::take(&mut delimiter_error), &mut cells, false);
            // Only call the user's step after the header row; the line may also have been
            // an empty one that was skipped.
            let Some(row) = out.data.pop().filter(|_| !was_header) else {
                return Flow::Continue;
            };
            self.step_counter += 1;
            if self.config.preview > 0 && self.step_counter > self.config.preview {
                self.preview_reached = true;
                return Flow::Abort;
            }
            user_step(Step {
                row,
                errors: out.errors,
                meta: out.meta,
            })
        };
        let raw = parser.parse_chunk_with(
            input,
            base_index,
            ignore_last_row,
            Some(&mut on_row),
            transform_header,
            empty_row,
        );
        Ok(self.process(raw, delimiter_error, &mut cells, true))
    }

    /// Fixes the newline and delimiter on the first chunk. Returns `true` when the
    /// delimiter could not be detected.
    fn resolve_format(&mut self, input: &str) -> bool {
        let config = &mut self.config;
        let newline = match config.newline {
            Some(n) => n,
            None => *config
                .newline
                .insert(guess_line_endings(input, config.quote())),
        };
        let delimiter = match &config.delimiter {
            Delimiter::Fixed(d) if !d.is_empty() => return false,
            Delimiter::Dynamic(f) => Some(f(input)),
            _ => guess_delimiter(input, newline, config),
        };
        let failed = delimiter.is_none();
        config.delimiter =
            Delimiter::Fixed(delimiter.unwrap_or_else(|| DEFAULT_DELIMITER.to_owned()));
        failed
    }

    /// Applies empty-line skipping, the header, `transform` and dynamic typing
    /// (JS: `processResults`).
    fn process<'a>(
        &mut self,
        raw: RawOutput<'a>,
        delimiter_error: bool,
        cells: &mut Cells<'_>,
        with_fields: bool,
    ) -> ParseOutput<'a> {
        let mut errors = raw.errors;
        let mut meta = raw.meta;
        if delimiter_error {
            errors.push(ParseError {
                kind: ErrorType::Delimiter,
                code: ErrorCode::UndetectableDelimiter,
                message: format!(
                    "Unable to auto-detect delimiting character; defaulted to '{DEFAULT_DELIMITER}'"
                ),
                row: None,
                index: None,
            });
        }

        let skip_empty = self.config.skip_empty_lines;
        let mut rows = raw
            .data
            .into_iter()
            .filter(|r| skip_empty == SkipEmptyLines::No || !is_empty_line(r, skip_empty));

        if self.needs_header_row() {
            if let Some(first) = rows.next() {
                self.header = Some(Header::new(
                    first.into_iter().map(Cow::into_owned).collect(),
                ));
            }
        }

        let config = &self.config;
        let plain = !config.header && !cells.has_transform(config) && !cells.has_typing(config);
        let row_counter = self.row_counter;
        let data: Vec<Row<'a>> = rows
            .enumerate()
            .map(|(i, row)| {
                if plain {
                    Row::Fields(row.into_iter().map(Value::String).collect())
                } else {
                    process_row(
                        row,
                        row_counter + i,
                        config,
                        self.header.as_ref(),
                        cells,
                        &mut self.typing,
                        &mut errors,
                    )
                }
            })
            .collect();
        self.row_counter += data.len();

        if config.header && with_fields {
            meta.fields = Some(
                self.header
                    .as_ref()
                    .map(|h| h.names().to_vec())
                    .unwrap_or_default(),
            );
        }
        ParseOutput { data, errors, meta }
    }
}

/// The per-cell callbacks of one parse: hooks, falling back to the config.
struct Cells<'h> {
    transform: Option<TransformHook<'h>>,
    dynamic_typing: Option<DynamicTypingHook<'h>>,
}

impl Cells<'_> {
    fn has_transform(&self, config: &ParseConfig) -> bool {
        self.transform.is_some() || config.transform.is_some()
    }

    fn has_typing(&self, config: &ParseConfig) -> bool {
        self.dynamic_typing.is_some() || !matches!(config.dynamic_typing, DynamicTyping::Off)
    }

    fn transform<'a>(
        &mut self,
        config: &ParseConfig,
        value: Cow<'a, str>,
        field: Field<'_>,
    ) -> Cow<'a, str> {
        match (&mut self.transform, &config.transform) {
            (Some(f), _) => Cow::Owned(f(&value, field)),
            (None, Some(f)) => Cow::Owned(f(&value, field)),
            (None, None) => value,
        }
    }
}

/// The part of `input` that [`parse`] tokenizes: without a leading byte order mark and
/// without the first [`ParseConfig::skip_first_n_lines`] lines. Byte offsets in the output
/// (`meta.cursor`, `error.index`) are relative to this slice, as in PapaParse.
pub fn parsed_slice<'a>(input: &'a str, config: &ParseConfig) -> &'a str {
    let input = input.strip_prefix(BYTE_ORDER_MARK).unwrap_or(input);
    if config.skip_first_n_lines == 0 {
        return input;
    }
    let newline = config
        .newline
        .unwrap_or_else(|| guess_line_endings(input, config.quote()));
    skip_lines(input, newline.as_str(), config.skip_first_n_lines)
}

/// `input` without its first `n` lines (JS: `split(newline).slice(n).join(newline)`),
/// without copying.
fn skip_lines<'a>(input: &'a str, newline: &str, n: usize) -> &'a str {
    let mut finder = Finder::new(input, newline);
    let mut start = 0;
    for _ in 0..n {
        match finder.find(start) {
            Some(pos) => start = pos + newline.len(),
            None => return "",
        }
    }
    &input[start..]
}

/// Applies header mapping, `transform` and dynamic typing to one row
/// (JS: `applyHeaderAndDynamicTypingAndTransformation` / `processRow`).
fn process_row<'a>(
    source: Vec<Cow<'a, str>>,
    row_index: usize,
    config: &ParseConfig,
    header: Option<&Arc<Header>>,
    cells: &mut Cells<'_>,
    typing: &mut ColumnTyping,
    errors: &mut Vec<ParseError>,
) -> Row<'a> {
    let width = source.len();
    let mut convert = |value: Cow<'a, str>, column: usize, field: Field<'_>| {
        let value = cells.transform(config, value, field);
        if typing.applies(column, field, config, cells) {
            parse_dynamic(value)
        } else {
            Value::String(value)
        }
    };

    let Some(header) = header else {
        let values = source
            .into_iter()
            .enumerate()
            .map(|(j, v)| convert(v, j, Field::Index(j)))
            .collect();
        return Row::Fields(values);
    };

    let names = header.names();
    let mut values = Vec::with_capacity(width.min(names.len()));
    let mut extra = Vec::new();
    for (j, value) in source.into_iter().enumerate() {
        match names.get(j) {
            Some(name) => values.push(convert(value, j, Field::Name(name))),
            None => extra.push(convert(value, names.len(), Field::Name(PARSED_EXTRA))),
        }
    }

    if width != names.len() {
        let (code, adjective) = if width > names.len() {
            (ErrorCode::TooManyFields, "Too many")
        } else {
            (ErrorCode::TooFewFields, "Too few")
        };
        errors.push(ParseError {
            kind: ErrorType::FieldMismatch,
            code,
            message: format!(
                "{adjective} fields: expected {} fields but parsed {width}",
                names.len()
            ),
            row: Some(row_index),
            index: None,
        });
    }
    Row::Record(Record::new(Arc::clone(header), values, extra))
}

/// Per-column answer to "apply dynamic typing?" (JS: `shouldApplyDynamicTyping`).
///
/// A column's field key (index or header name) never changes, so each column is
/// decided once instead of once per cell (JS caches callback answers the same way).
#[derive(Default)]
struct ColumnTyping {
    columns: Vec<Option<bool>>,
}

impl ColumnTyping {
    /// `column` identifies the cache slot; extras share the slot one past the header.
    fn applies(
        &mut self,
        column: usize,
        field: Field<'_>,
        config: &ParseConfig,
        cells: &mut Cells<'_>,
    ) -> bool {
        let decide = match (&mut cells.dynamic_typing, &config.dynamic_typing) {
            (None, DynamicTyping::Off) => return false,
            (None, DynamicTyping::All) => return true,
            (Some(f), _) => Decide::Hook(&mut **f),
            (None, DynamicTyping::Fields(map)) => Decide::Fields(map),
            (None, DynamicTyping::Dynamic(f)) => Decide::Config(f),
        };
        if column >= self.columns.len() {
            self.columns.resize(column + 1, None);
        }
        *self.columns[column].get_or_insert_with(|| match decide {
            Decide::Hook(f) => f(field),
            Decide::Fields(map) => map.get(&field.to_string()).copied().unwrap_or(false),
            Decide::Config(f) => f(field),
        })
    }
}

enum Decide<'d> {
    Hook(&'d mut dyn FnMut(Field<'_>) -> bool),
    Fields(&'d std::collections::HashMap<String, bool>),
    Config(&'d crate::config::DynamicTypingFn),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_typing() {
        let cfg = ParseConfig::default()
            .delimiter(",")
            .header(true)
            .dynamic_typing(true);
        let out = parse("a,b\n1,true\n2", &cfg).unwrap();
        assert_eq!(out.meta.fields, Some(vec!["a".into(), "b".into()]));
        let Row::Record(r) = &out.data[0] else {
            panic!("expected record")
        };
        assert_eq!(r["a"], Value::Number(1.0));
        assert_eq!(r["b"], Value::Bool(true));
        assert_eq!(out.errors[0].code, ErrorCode::TooFewFields);
        assert_eq!(out.errors[0].row, Some(1));
    }

    #[test]
    fn detects_delimiter_and_reports_failure() {
        let out = parse("a\tb\n1\t2", &ParseConfig::default()).unwrap();
        assert_eq!(out.meta.delimiter, "\t");
        let out = parse("abc", &ParseConfig::default()).unwrap();
        assert_eq!(out.errors[0].code, ErrorCode::UndetectableDelimiter);
    }

    #[test]
    fn skip_lines_matches_split_join() {
        for (s, n) in [
            ("a\nb\nc", 1),
            ("a\nb\nc", 2),
            ("a\nb\nc", 3),
            ("a\nb\nc", 9),
            ("", 1),
            ("\n\n", 1),
        ] {
            let expected = s.split('\n').skip(n).collect::<Vec<_>>().join("\n");
            assert_eq!(skip_lines(s, "\n", n), expected, "{s:?} {n}");
        }
    }

    #[test]
    fn huge_preview_does_not_overflow() {
        let cfg = ParseConfig::default().header(true).preview(usize::MAX);
        assert_eq!(parse("a,b\n1,2", &cfg).unwrap().data.len(), 1);
    }
}
