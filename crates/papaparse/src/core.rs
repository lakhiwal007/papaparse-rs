//! Port of `Papa.Parser`, the low-level tokenizer (papaparse.js `function Parser`).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::config::{Delimiter, ParseConfig, TransformHeaderFn, TransformHeaderHook};
use crate::error::{ConfigError, ErrorCode, ErrorType, ParseError};
use crate::search::{self, Finder};
use crate::value::Meta;
use crate::{BAD_DELIMITERS, BYTE_ORDER_MARK, DEFAULT_DELIMITER};

/// Returned by a step callback to continue or stop parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Keep parsing.
    Continue,
    /// Stop parsing; the output's `meta.aborted` is set.
    Abort,
    /// Stop parsing without marking it aborted, so the rest of the input can be parsed
    /// later. A [`crate::ChunkParser`] keeps the unparsed text and continues with it
    /// on the next push. JS: `handle.pause()`.
    Pause,
}

/// Raw output of the core [`Parser`]: rows of unprocessed fields.
///
/// Fields borrow from the input where possible; only quoted fields containing escaped
/// quotes (and renamed headers) are allocated. Use [`RawOutput::into_owned`] to detach
/// the output from the input's lifetime.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawOutput<'a> {
    /// Parsed rows, each a list of fields.
    pub data: Vec<Vec<Cow<'a, str>>>,
    /// Recoverable problems found while parsing.
    pub errors: Vec<ParseError>,
    /// Information about the run.
    pub meta: Meta,
}

impl RawOutput<'_> {
    /// Copies every borrowed field so the output no longer borrows the input.
    pub fn into_owned(self) -> RawOutput<'static> {
        RawOutput {
            data: self
                .data
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|f| Cow::Owned(f.into_owned()))
                        .collect()
                })
                .collect(),
            errors: self.errors,
            meta: self.meta,
        }
    }
}

/// The core CSV tokenizer. JS: `new Papa.Parser(config)`.
///
/// Uses only the delimiter, newline, quote/escape, comments, preview, fast-mode and
/// header-renaming options of [`ParseConfig`]. An auto delimiter falls back to `,`
/// and an auto newline to `\n`; use [`crate::parse`] for detection.
///
/// Runs in time linear in the input length for every input.
#[derive(Debug)]
pub struct Parser {
    delim: String,
    newline: &'static str,
    comments: Option<String>,
    preview: usize,
    fast_mode: Option<bool>,
    quote: String,
    escape: String,
    header: bool,
    transform_header: Option<HeaderFn>,
    header_parsed: bool,
    renamed_headers: Option<Vec<(String, String)>>,
}

struct HeaderFn(TransformHeaderFn);

impl std::fmt::Debug for HeaderFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<fn>")
    }
}

impl Parser {
    /// Validates `config` and creates a parser.
    pub fn new(config: &ParseConfig) -> Result<Self, ConfigError> {
        let delim = match &config.delimiter {
            Delimiter::Fixed(d) if !d.is_empty() && !BAD_DELIMITERS.contains(&d.as_str()) => {
                d.clone()
            }
            _ => DEFAULT_DELIMITER.to_owned(),
        };

        let comments = match config.comments.as_deref() {
            Some(c) if c == delim => return Err(ConfigError::CommentSameAsDelimiter),
            Some(c) if c.is_empty() || BAD_DELIMITERS.contains(&c) => None,
            other => other.map(str::to_owned),
        };

        let quote = config.quote();
        Ok(Self {
            delim,
            newline: config.newline.unwrap_or_default().as_str(),
            comments,
            preview: config.preview,
            fast_mode: config.fast_mode,
            quote: quote.to_string(),
            escape: config.escape_char.unwrap_or(quote).to_string(),
            header: config.header,
            transform_header: config.transform_header.clone().map(HeaderFn),
            header_parsed: false,
            renamed_headers: None,
        })
    }

    /// Parses the whole input in one go.
    pub fn parse<'a>(&mut self, input: &'a str) -> RawOutput<'a> {
        self.parse_chunk(input, 0, false, None)
    }

    /// Parses the input, handing each row to `step` as soon as it is complete.
    /// The returned output holds whatever rows were not delivered (normally none).
    pub fn parse_with_step<'a>(
        &mut self,
        input: &'a str,
        step: &mut dyn FnMut(RawOutput<'a>) -> Flow,
    ) -> RawOutput<'a> {
        self.parse_chunk(input, 0, false, Some(step))
    }

    /// Parses one chunk of a larger input. `base_index` is the offset of `input` within
    /// the whole stream; `ignore_last_row` withholds a possibly incomplete final row.
    pub fn parse_chunk<'a>(
        &mut self,
        input: &'a str,
        base_index: usize,
        ignore_last_row: bool,
        step: Option<&mut dyn FnMut(RawOutput<'a>) -> Flow>,
    ) -> RawOutput<'a> {
        self.parse_chunk_with(input, base_index, ignore_last_row, step, None, false)
    }

    /// [`Parser::parse_chunk`] with a header transform that replaces the configured one.
    /// `empty_row` makes an empty `input` one empty row: the stream so far ended with a
    /// row terminator, which a whole-input parse turns into a final empty row.
    pub(crate) fn parse_chunk_with<'a>(
        &mut self,
        input: &'a str,
        base_index: usize,
        ignore_last_row: bool,
        step: Option<&mut dyn FnMut(RawOutput<'a>) -> Flow>,
        transform_header: Option<TransformHeaderHook<'_>>,
        empty_row: bool,
    ) -> RawOutput<'a> {
        let mut run = Run {
            parser: self,
            input,
            base_index,
            ignore_last_row,
            step,
            transform_header,
            cursor: 0,
            last_cursor: 0,
            data: Vec::new(),
            errors: Vec::new(),
            row: Vec::new(),
            width_hint: 0,
            aborted: false,
        };
        if empty_row && input.is_empty() && !ignore_last_row {
            return run.finish(None);
        }
        run.parse()
    }
}

/// State of a single `parse` call (JS: the locals of `this.parse`).
struct Run<'p, 'i, 's, 'h> {
    parser: &'p mut Parser,
    input: &'i str,
    base_index: usize,
    ignore_last_row: bool,
    step: Option<&'s mut dyn FnMut(RawOutput<'i>) -> Flow>,
    /// Overrides `parser.transform_header` for this run.
    transform_header: Option<TransformHeaderHook<'h>>,
    cursor: usize,
    last_cursor: usize,
    data: Vec<Vec<Cow<'i, str>>>,
    errors: Vec<ParseError>,
    row: Vec<Cow<'i, str>>,
    /// Width of the previous row, used to pre-size the next one.
    width_hint: usize,
    aborted: bool,
}

impl<'i> Run<'_, 'i, '_, '_> {
    fn parse(&mut self) -> RawOutput<'i> {
        let input = self.input;
        if input.is_empty() {
            return self.returnable(false);
        }

        let use_fast = self
            .parser
            .fast_mode
            .unwrap_or_else(|| Finder::new(input, &self.parser.quote).find(0).is_none());
        if use_fast {
            return self.parse_fast();
        }

        // Cheap clones of short strings, so the loop can borrow `self` mutably.
        let delim = self.parser.delim.clone();
        let newline = self.parser.newline;
        let quote = self.parser.quote.clone();
        let escape = self.parser.escape.clone();
        let comments = self.parser.comments.clone();
        let preview = self.parser.preview;
        let escaped_quote = format!("{escape}{quote}");

        // JS keeps `nextDelim`/`nextNewline` and re-runs `indexOf` from the cursor; a memoizing
        // finder returns the same positions without rescanning (see `search`).
        let mut delims = Finder::new(input, &delim);
        let mut newlines = Finder::new(input, newline);
        let mut quotes = Finder::new(input, &quote);
        let mut escaped_quotes = Finder::new(input, &escaped_quote);

        loop {
            // Field has opening quote
            if input[self.cursor..].starts_with(&*quote) {
                self.cursor += quote.len();
                let mut search_from = self.cursor;

                loop {
                    // Find closing quote
                    let Some(quote_pos) = quotes.find(search_from) else {
                        if !self.ignore_last_row {
                            self.push_error(ErrorCode::MissingQuotes, "Quoted field unterminated");
                        }
                        return self.finish(None);
                    };
                    let after = quote_pos + quote.len();

                    // Closing quote at EOF
                    if after == input.len() {
                        let value =
                            unescape(input, self.cursor, quote_pos, &mut escaped_quotes, &quote);
                        return self.finish(Some(value));
                    }

                    // Quote escaped by a following escape char (quote == escape): `""`
                    if quote == escape && input[after..].starts_with(&*escape) {
                        search_from = after + escape.len();
                        continue;
                    }

                    // Quote escaped by a preceding escape char (quote != escape): `\"`
                    if quote != escape && input[..quote_pos].ends_with(&*escape) {
                        search_from = after;
                        continue;
                    }

                    let next_delim = delims.find(after);
                    let next_newline = newlines.find(after);
                    // Check up to next_delim or next_newline, whichever is closest.
                    // (JS `Math.min(-1, n)` is -1, so a missing delimiter wins.)
                    let check_up_to = match (next_delim, next_newline) {
                        (d, None) => d,
                        (Some(d), Some(n)) => Some(d.min(n)),
                        (None, Some(_)) => None,
                    };

                    // Closing quote followed by delimiter or 'unnecessary spaces + delimiter'
                    let spaces = extra_spaces(input, after, check_up_to);
                    if input[after + spaces..].starts_with(&*delim) {
                        let value =
                            unescape(input, self.cursor, quote_pos, &mut escaped_quotes, &quote);
                        self.row.push(value);
                        self.cursor = after + spaces + delim.len();
                        break;
                    }

                    // Closing quote followed by newline or 'unnecessary spaces + newline'
                    let spaces = extra_spaces(input, after, next_newline);
                    if input[after + spaces..].starts_with(newline) {
                        let value =
                            unescape(input, self.cursor, quote_pos, &mut escaped_quotes, &quote);
                        self.row.push(value);
                        self.save_row(after + spaces + newline.len());

                        if self.do_step_if_any() {
                            return self.returnable(false);
                        }
                        if preview > 0 && self.data.len() >= preview {
                            return self.returnable(true);
                        }
                        break;
                    }

                    // Not escaped and not followed by delimiter/newline/EOF: malformed.
                    self.push_error(
                        ErrorCode::InvalidQuotes,
                        "Trailing quote on quoted field is malformed",
                    );
                    // JS resumes the search two code units after the quote.
                    search_from = after + input[after..].chars().next().map_or(0, char::len_utf8);
                }
                continue;
            }

            let next_newline = newlines.find(self.cursor);

            // Comment found at start of new line
            if let Some(c) = &comments {
                if self.row.is_empty() && input[self.cursor..].starts_with(c.as_str()) {
                    let Some(n) = next_newline else {
                        // Comment ends at EOF
                        return self.returnable(false);
                    };
                    self.cursor = n + newline.len();
                    continue;
                }
            }

            // Next delimiter comes before next newline, so we've reached end of field
            if let Some(d) = delims.find(self.cursor) {
                if next_newline.is_none_or(|n| d < n) {
                    self.row.push(Cow::Borrowed(&input[self.cursor..d]));
                    self.cursor = d + delim.len();
                    continue;
                }
            }

            // End of row
            if let Some(n) = next_newline {
                self.row.push(Cow::Borrowed(&input[self.cursor..n]));
                self.save_row(n + newline.len());

                if self.do_step_if_any() {
                    return self.returnable(false);
                }
                if preview > 0 && self.data.len() >= preview {
                    return self.returnable(true);
                }
                continue;
            }

            break;
        }

        self.finish(None)
    }

    /// Quote-free path: plain splitting on newline and delimiter.
    fn parse_fast(&mut self) -> RawOutput<'i> {
        let input = self.input;
        let newline = self.parser.newline;
        let delim = self.parser.delim.clone();
        let comments = self.parser.comments.clone();
        let preview = self.parser.preview;

        let mut rows = search::split(input, newline).enumerate().peekable();
        while let Some((i, row)) = rows.next() {
            self.cursor += row.len();
            if rows.peek().is_some() {
                self.cursor += newline.len();
            } else if self.ignore_last_row {
                return self.returnable(false);
            }
            if comments.as_deref().is_some_and(|c| row.starts_with(c)) {
                continue;
            }
            let mut fields = Vec::with_capacity(self.width_hint);
            fields.extend(search::split(row, &delim).map(Cow::Borrowed));
            self.width_hint = fields.len();
            if self.step.is_some() {
                self.data.clear();
                self.push_row(fields);
                if self.do_step_if_any() {
                    return self.returnable(false);
                }
            } else {
                self.push_row(fields);
            }
            if preview > 0 && i >= preview {
                self.data.truncate(preview);
                return self.returnable(true);
            }
        }
        self.returnable(false)
    }

    fn push_error(&mut self, code: ErrorCode, message: &str) {
        self.errors.push(ParseError {
            kind: ErrorType::Quotes,
            code,
            message: message.to_owned(),
            row: Some(self.data.len()), // row has yet to be inserted
            index: Some(self.cursor),
        });
    }

    fn push_row(&mut self, row: Vec<Cow<'i, str>>) {
        self.data.push(row);
        self.last_cursor = self.cursor;
    }

    /// Appends the current row to the results and moves the cursor.
    fn save_row(&mut self, new_cursor: usize) {
        self.cursor = new_cursor;
        self.width_hint = self.row.len();
        let row = std::mem::replace(&mut self.row, Vec::with_capacity(self.width_hint));
        self.push_row(row);
    }

    /// Appends the rest of the input (or `value`) as the last field and returns.
    fn finish(&mut self, value: Option<Cow<'i, str>>) -> RawOutput<'i> {
        if self.ignore_last_row {
            return self.returnable(false);
        }
        let value = value.unwrap_or(Cow::Borrowed(&self.input[self.cursor..]));
        self.row.push(value);
        self.cursor = self.input.len();
        let row = std::mem::take(&mut self.row);
        self.push_row(row);
        self.do_step_if_any();
        self.returnable(false)
    }

    /// Runs the step callback, if any, handing it (and thereby clearing) data and errors.
    /// Returns `true` when parsing should stop.
    fn do_step_if_any(&mut self) -> bool {
        if self.step.is_none() {
            return false;
        }
        let results = self.returnable(false);
        let flow = self
            .step
            .as_mut()
            .map_or(Flow::Continue, |step| step(results));
        self.aborted |= flow == Flow::Abort;
        flow != Flow::Continue
    }

    fn returnable(&mut self, stopped: bool) -> RawOutput<'i> {
        if self.parser.header && self.base_index == 0 && !self.parser.header_parsed {
            if let Some(first) = self.data.first_mut() {
                let mut configured;
                let transform: Option<TransformHeaderHook<'_>> =
                    match (&mut self.transform_header, &self.parser.transform_header) {
                        (Some(f), _) => Some(&mut **f),
                        (None, Some(HeaderFn(f))) => {
                            configured = |h: &str, i| f(h, i);
                            Some(&mut configured)
                        }
                        (None, None) => None,
                    };
                rename_headers(first, transform, &mut self.parser.renamed_headers);
                self.parser.header_parsed = true;
            }
        }
        RawOutput {
            data: std::mem::take(&mut self.data),
            errors: std::mem::take(&mut self.errors),
            meta: Meta {
                delimiter: self.parser.delim.clone(),
                linebreak: self.parser.newline.to_owned(),
                aborted: self.aborted,
                truncated: stopped,
                cursor: self.last_cursor + self.base_index,
                renamed_headers: self.parser.renamed_headers.clone(),
                fields: None,
            },
        }
    }
}

/// The quoted field `input[start..end]` with escaped quotes collapsed.
/// Borrows from the input unless the field contains an escape sequence.
fn unescape<'i>(
    input: &'i str,
    start: usize,
    end: usize,
    escaped: &mut Finder<'_>,
    quote: &str,
) -> Cow<'i, str> {
    let field = &input[start..end];
    if escaped.find(start).is_none_or(|p| p >= end) {
        return Cow::Borrowed(field);
    }
    let escaped_len = escaped.needle_len();
    let mut out = String::with_capacity(field.len());
    let mut pos = start;
    while let Some(p) = escaped.find(pos).filter(|&p| p + escaped_len <= end) {
        out.push_str(&input[pos..p]);
        out.push_str(quote);
        pos = p + escaped_len;
    }
    out.push_str(&input[pos..end]);
    Cow::Owned(out)
}

/// Number of whitespace bytes between the closing quote (ending at `after`) and `index`,
/// or 0 if anything other than whitespace sits in between.
fn extra_spaces(input: &str, after: usize, index: Option<usize>) -> usize {
    match index {
        Some(i) if i > after => {
            let between = &input[after..i];
            // Stops at the first non-whitespace char, so this is O(1) in the common case.
            if between
                .trim_start_matches(crate::is_js_whitespace)
                .is_empty()
            {
                between.len()
            } else {
                0
            }
        }
        _ => 0,
    }
}

/// Strips the BOM, applies `transform_header` and de-duplicates header names
/// (`a,a` becomes `a,a_1`), recording renames.
fn rename_headers(
    headers: &mut [Cow<'_, str>],
    mut transform: Option<TransformHeaderHook<'_>>,
    renamed: &mut Option<Vec<(String, String)>>,
) {
    let mut header_count: HashMap<String, usize> = HashMap::with_capacity(headers.len());
    let mut used: HashSet<String> = headers.iter().map(ToString::to_string).collect();

    for (i, slot) in headers.iter_mut().enumerate() {
        let stripped = slot.strip_prefix(BYTE_ORDER_MARK).unwrap_or(slot.as_ref());
        let header = match transform.as_mut() {
            Some(f) => f(stripped, i),
            None => stripped.to_owned(),
        };

        if let Some(count) = header_count.get_mut(&header) {
            let mut suffix = *count;
            let new_header = loop {
                let candidate = format!("{header}_{suffix}");
                suffix += 1;
                if !used.contains(&candidate) {
                    break candidate;
                }
            };
            used.insert(new_header.clone());
            *slot = Cow::Owned(new_header.clone());
            *count += 1;
            renamed
                .get_or_insert_with(Vec::new)
                .push((new_header, header.clone()));
        } else {
            header_count.insert(header.clone(), 1);
            if *slot != header {
                *slot = Cow::Owned(header.clone());
            }
        }
        used.insert(header);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows<'a>(input: &'a str, config: &ParseConfig) -> Vec<Vec<Cow<'a, str>>> {
        Parser::new(config).unwrap().parse(input).data
    }

    #[test]
    fn quoted_fields_with_escapes_and_newlines() {
        let out = rows("a,\"b \"\"x\"\"\nc\",d\ne,f,g", &ParseConfig::default());
        assert_eq!(out, vec![vec!["a", "b \"x\"\nc", "d"], vec!["e", "f", "g"]]);
    }

    #[test]
    fn custom_escape_char() {
        let cfg = ParseConfig::default().escape_char('\\');
        assert_eq!(rows("\"a\\\"b\",c", &cfg), vec![vec!["a\"b", "c"]]);
    }

    #[test]
    fn step_receives_each_row_and_can_abort() {
        let mut seen = Vec::new();
        let mut parser = Parser::new(&ParseConfig::default()).unwrap();
        parser.parse_with_step("a\nb\nc", &mut |r| {
            seen.extend(r.data);
            if seen.len() == 2 {
                Flow::Abort
            } else {
                Flow::Continue
            }
        });
        assert_eq!(seen, vec![vec!["a"], vec!["b"]]);
    }

    #[test]
    fn malformed_quote_before_multibyte_char_does_not_panic() {
        let out = Parser::new(&ParseConfig::default())
            .unwrap()
            .parse("\"é\"ü,x");
        let codes: Vec<_> = out.errors.iter().map(|e| e.code).collect();
        assert_eq!(codes, [ErrorCode::InvalidQuotes, ErrorCode::MissingQuotes]);
        assert_eq!(out.data, vec![vec!["é\"ü,x"]]);
    }

    #[test]
    fn comment_equal_to_delimiter_is_rejected() {
        let cfg = ParseConfig::default().delimiter("#").comments("#");
        assert_eq!(
            Parser::new(&cfg).unwrap_err(),
            ConfigError::CommentSameAsDelimiter
        );
    }

    /// Regression: JS-style `indexOf` rescans made this input quadratic.
    #[test]
    #[cfg_attr(miri, ignore = "too slow under the interpreter")]
    fn single_column_quoted_rows_are_linear() {
        let input = "\"x\"\n".repeat(200_000);
        let start = std::time::Instant::now();
        let out = rows(&input, &ParseConfig::default());
        assert_eq!(out.len(), 200_001);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "took {:?}",
            start.elapsed()
        );
    }
}
