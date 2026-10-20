//! Port of `Papa.unparse` (papaparse.js `JsonToCsv`): rows to CSV text.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use crate::BAD_DELIMITERS;
use crate::config::SkipEmptyLines;
use crate::error::ConfigError;
use crate::value::{Record, Row, Value};

/// Callback type for [`Quotes::Dynamic`]: `(value, column index) -> quote it?`.
pub type QuotesFn = Arc<dyn Fn(&Value<'_>, usize) -> bool + Send + Sync>;
/// Callback type for [`EscapeFormulae::Custom`]: `(text) -> is it a formula?`.
pub type FormulaFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Borrowed counterpart of [`QuotesFn`], for [`UnparseHooks`]: `(value, column index,
/// row index)`, where the row is `None` for the header and otherwise indexes the rows
/// passed in (so a binding can hand its callback the original value).
pub type QuotesHook<'h> = &'h mut dyn FnMut(&Value<'_>, usize, Option<usize>) -> bool;
/// Borrowed counterpart of [`FormulaFn`], for [`UnparseHooks`].
pub type FormulaHook<'h> = &'h mut dyn FnMut(&str) -> bool;

/// Which cells to quote even when they don't need it. JS: `config.quotes`.
#[derive(Clone, Default)]
pub enum Quotes {
    /// Only cells that need it (`false`).
    #[default]
    Needed,
    /// Every non-empty cell (`true`).
    All,
    /// Columns whose entry is `true` (`[true, false, ...]`).
    Columns(Vec<bool>),
    /// Decided per cell by a callback.
    Dynamic(QuotesFn),
}

/// Guarding cells that spreadsheets would run as formulas. JS: `config.escapeFormulae`.
///
/// A guarded cell gets a leading `'` and is quoted.
#[derive(Clone, Default)]
pub enum EscapeFormulae {
    /// Write cells as they are (`false`).
    #[default]
    Off,
    /// Guard text starting with `=`, `+`, `-`, `@`, tab or carriage return (`true`).
    Default,
    /// Guard text the callback matches (JS: a `RegExp`).
    Custom(FormulaFn),
}

/// Options for [`unparse`] and [`unparse_table`]. Mirrors PapaParse's unparse config.
#[derive(Clone)]
#[non_exhaustive]
pub struct UnparseConfig {
    /// Which cells to quote beyond those that need it.
    pub quotes: Quotes,
    /// Write the header row (default `true`).
    pub header: bool,
    /// Field separator (default `,`). One containing `\r`, `\n`, `"` or a BOM is ignored.
    pub delimiter: String,
    /// Row separator (default `\r\n`); any string.
    pub newline: String,
    /// Quote (default `"`).
    pub quote_char: String,
    /// Written before a quote inside a quoted cell; `None` doubles the quote.
    pub escape_char: Option<String>,
    /// Which empty rows to leave out.
    pub skip_empty_lines: SkipEmptyLines,
    /// Columns to write for keyed rows, instead of the first row's keys.
    pub columns: Option<Vec<String>>,
    /// Guard cells that spreadsheets would run as formulas.
    pub escape_formulae: EscapeFormulae,
}

impl Default for UnparseConfig {
    fn default() -> Self {
        Self {
            quotes: Quotes::Needed,
            header: true,
            delimiter: ",".to_owned(),
            newline: "\r\n".to_owned(),
            quote_char: "\"".to_owned(),
            escape_char: None,
            skip_empty_lines: SkipEmptyLines::No,
            columns: None,
            escape_formulae: EscapeFormulae::Off,
        }
    }
}

impl UnparseConfig {
    /// Choose which cells to quote.
    #[must_use]
    pub fn quotes(mut self, quotes: Quotes) -> Self {
        self.quotes = quotes;
        self
    }

    /// Write the header row or not.
    #[must_use]
    pub fn header(mut self, header: bool) -> Self {
        self.header = header;
        self
    }

    /// Field separator.
    #[must_use]
    pub fn delimiter(mut self, delimiter: impl Into<String>) -> Self {
        self.delimiter = delimiter.into();
        self
    }

    /// Row separator.
    #[must_use]
    pub fn newline(mut self, newline: impl Into<String>) -> Self {
        self.newline = newline.into();
        self
    }

    /// Quote.
    #[must_use]
    pub fn quote_char(mut self, quote: impl Into<String>) -> Self {
        self.quote_char = quote.into();
        self
    }

    /// Escape for quotes inside quoted cells.
    #[must_use]
    pub fn escape_char(mut self, escape: impl Into<String>) -> Self {
        self.escape_char = Some(escape.into());
        self
    }

    /// Which empty rows to leave out.
    #[must_use]
    pub fn skip_empty_lines(mut self, mode: SkipEmptyLines) -> Self {
        self.skip_empty_lines = mode;
        self
    }

    /// Columns to write for keyed rows.
    #[must_use]
    pub fn columns<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.columns = Some(columns.into_iter().map(Into::into).collect());
        self
    }

    /// Guard cells that spreadsheets would run as formulas.
    #[must_use]
    pub fn escape_formulae(mut self, mode: EscapeFormulae) -> Self {
        self.escape_formulae = mode;
        self
    }
}

impl fmt::Debug for Quotes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Quotes::Needed => f.write_str("Needed"),
            Quotes::All => f.write_str("All"),
            Quotes::Columns(c) => f.debug_tuple("Columns").field(c).finish(),
            Quotes::Dynamic(_) => f.write_str("Dynamic(<fn>)"),
        }
    }
}

impl fmt::Debug for EscapeFormulae {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EscapeFormulae::Off => "Off",
            EscapeFormulae::Default => "Default",
            EscapeFormulae::Custom(_) => "Custom(<fn>)",
        })
    }
}

impl fmt::Debug for UnparseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnparseConfig")
            .field("quotes", &self.quotes)
            .field("header", &self.header)
            .field("delimiter", &self.delimiter)
            .field("newline", &self.newline)
            .field("quote_char", &self.quote_char)
            .field("escape_char", &self.escape_char)
            .field("skip_empty_lines", &self.skip_empty_lines)
            .field("columns", &self.columns)
            .field("escape_formulae", &self.escape_formulae)
            .finish()
    }
}

/// Callbacks for one [`unparse_with_hooks`] call that may borrow local state, unlike
/// the `Send + Sync + 'static` ones in [`UnparseConfig`]. A hook that is set replaces
/// the matching config option.
#[derive(Default)]
pub struct UnparseHooks<'h> {
    /// Replaces [`UnparseConfig::quotes`]: `(value, column index, row index) -> quote it?`.
    pub quotes: Option<QuotesHook<'h>>,
    /// Replaces [`UnparseConfig::escape_formulae`]: `(text) -> is it a formula?`.
    pub escape_formulae: Option<FormulaHook<'h>>,
}

impl fmt::Debug for UnparseHooks<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let set = |b: bool| if b { "Some(<fn>)" } else { "None" };
        f.debug_struct("UnparseHooks")
            .field("quotes", &set(self.quotes.is_some()))
            .field("escape_formulae", &set(self.escape_formulae.is_some()))
            .finish()
    }
}

/// A row that [`unparse`] can write: a list of cells (JS array) or cells keyed by
/// column name (JS object).
///
/// Implemented for `Vec<Value>`, `[Value]`, [`Row`], [`Record`] and
/// `Vec<(name, Value)>`.
pub trait UnparseRow {
    /// Column names in order for a keyed row; `None` for a list.
    fn keys(&self) -> Option<Vec<&str>>;
    /// Number of cells (of keys, for a keyed row).
    fn width(&self) -> usize;
    /// The cell at `index` of a list.
    fn at(&self, index: usize) -> Option<&Value<'_>>;
    /// The cell named `name` of a keyed row.
    fn named(&self, name: &str) -> Option<&Value<'_>>;
    /// The cell named `name`, which is header column `col`; a row that stores cells in
    /// header order can answer by position.
    fn named_at(&self, name: &str, col: usize) -> Option<&Value<'_>> {
        let _ = col;
        self.named(name)
    }
}

impl UnparseRow for [Value<'_>] {
    fn keys(&self) -> Option<Vec<&str>> {
        None
    }
    fn width(&self) -> usize {
        self.len()
    }
    fn at(&self, index: usize) -> Option<&Value<'_>> {
        self.get(index)
    }
    fn named(&self, _: &str) -> Option<&Value<'_>> {
        None
    }
}

impl UnparseRow for Vec<Value<'_>> {
    fn keys(&self) -> Option<Vec<&str>> {
        None
    }
    fn width(&self) -> usize {
        self.len()
    }
    fn at(&self, index: usize) -> Option<&Value<'_>> {
        self.get(index)
    }
    fn named(&self, _: &str) -> Option<&Value<'_>> {
        None
    }
}

impl<K: AsRef<str>> UnparseRow for Vec<(K, Value<'_>)> {
    fn keys(&self) -> Option<Vec<&str>> {
        Some(self.iter().map(|(k, _)| k.as_ref()).collect())
    }
    fn width(&self) -> usize {
        self.len()
    }
    fn at(&self, _: usize) -> Option<&Value<'_>> {
        None
    }
    fn named(&self, name: &str) -> Option<&Value<'_>> {
        // Like assigning to a JS object, a later duplicate key wins.
        self.iter()
            .rev()
            .find(|(k, _)| k.as_ref() == name)
            .map(|(_, v)| v)
    }
}

impl UnparseRow for Record<'_> {
    fn keys(&self) -> Option<Vec<&str>> {
        Some(self.iter().map(|(k, _)| k).collect())
    }
    fn width(&self) -> usize {
        self.len()
    }
    fn at(&self, _: usize) -> Option<&Value<'_>> {
        None
    }
    fn named(&self, name: &str) -> Option<&Value<'_>> {
        self.get(name)
    }
    fn named_at(&self, name: &str, col: usize) -> Option<&Value<'_>> {
        self.get_at(name, col)
    }
}

impl UnparseRow for Row<'_> {
    fn keys(&self) -> Option<Vec<&str>> {
        match self {
            Row::Fields(v) => v.keys(),
            Row::Record(r) => r.keys(),
        }
    }
    fn width(&self) -> usize {
        match self {
            Row::Fields(v) => v.width(),
            Row::Record(r) => r.width(),
        }
    }
    fn at(&self, index: usize) -> Option<&Value<'_>> {
        match self {
            Row::Fields(v) => v.get(index),
            Row::Record(_) => None,
        }
    }
    fn named(&self, name: &str) -> Option<&Value<'_>> {
        match self {
            Row::Fields(_) => None,
            Row::Record(r) => r.get(name),
        }
    }
    fn named_at(&self, name: &str, col: usize) -> Option<&Value<'_>> {
        match self {
            Row::Fields(_) => None,
            Row::Record(r) => r.get_at(name, col),
        }
    }
}

impl<T: UnparseRow + ?Sized> UnparseRow for &T {
    fn keys(&self) -> Option<Vec<&str>> {
        (**self).keys()
    }
    fn width(&self) -> usize {
        (**self).width()
    }
    fn at(&self, index: usize) -> Option<&Value<'_>> {
        (**self).at(index)
    }
    fn named(&self, name: &str) -> Option<&Value<'_>> {
        (**self).named(name)
    }
    fn named_at(&self, name: &str, col: usize) -> Option<&Value<'_>> {
        (**self).named_at(name, col)
    }
}

/// Writes rows as CSV. JS: `Papa.unparse(array, config)`.
///
/// Keyed rows get a header row: [`UnparseConfig::columns`], or the first row's keys.
/// Lists are written as they are.
///
/// ```
/// use papaparse::{UnparseConfig, Value, unparse};
///
/// let rows = vec![
///     vec![("name", Value::from("Ada")), ("age", Value::Number(36.0))],
///     vec![("name", Value::from("Lin, Bo")), ("age", Value::Null)],
/// ];
/// let csv = unparse(&rows, &UnparseConfig::default())?;
/// assert_eq!(csv, "name,age\r\nAda,36\r\n\"Lin, Bo\",");
/// # Ok::<(), papaparse::ConfigError>(())
/// ```
///
/// Cells are written as JavaScript would print them: numbers like `Number.toString`,
/// [`Value::Date`] text as is (empty text stands for an invalid date, which is written
/// empty but does not make a row empty), [`Value::Null`] as an empty cell.
///
/// # Errors
/// [`ConfigError::EmptyColumns`] if `columns` is set but empty.
pub fn unparse<R: UnparseRow>(rows: &[R], config: &UnparseConfig) -> Result<String, ConfigError> {
    check(config)?;
    let fields: Option<Vec<&str>> =
        rows.first()
            .and_then(UnparseRow::keys)
            .map(|keys| match &config.columns {
                Some(columns) => columns.iter().map(String::as_str).collect(),
                None => keys,
            });
    Ok(Writer::new(config, UnparseHooks::default())
        .serialize(fields.as_deref().unwrap_or_default(), rows))
}

/// Writes rows under the given header. JS: `Papa.unparse({ fields, data }, config)`.
///
/// Keyed rows are written in `fields` order, lists by position. With `fields` `None`,
/// the header comes from [`UnparseConfig::columns`] or the first keyed row.
///
/// # Errors
/// [`ConfigError::EmptyColumns`] if `columns` is set but empty.
pub fn unparse_table<R: UnparseRow, F: AsRef<str>>(
    fields: Option<&[F]>,
    rows: &[R],
    config: &UnparseConfig,
) -> Result<String, ConfigError> {
    unparse_with_hooks(fields, rows, config, UnparseHooks::default())
}

/// [`unparse_table`] with [`UnparseHooks`] that replace the matching config callbacks.
///
/// # Errors
/// [`ConfigError::EmptyColumns`] if `columns` is set but empty.
pub fn unparse_with_hooks<R: UnparseRow, F: AsRef<str>>(
    fields: Option<&[F]>,
    rows: &[R],
    config: &UnparseConfig,
    hooks: UnparseHooks<'_>,
) -> Result<String, ConfigError> {
    check(config)?;
    let fields: Vec<&str> = match (fields, &config.columns) {
        (Some(f), _) => f.iter().map(AsRef::as_ref).collect(),
        (None, Some(columns)) => columns.iter().map(String::as_str).collect(),
        (None, None) => rows.first().and_then(UnparseRow::keys).unwrap_or_default(),
    };
    Ok(Writer::new(config, hooks).serialize(&fields, rows))
}

fn check(config: &UnparseConfig) -> Result<(), ConfigError> {
    match &config.columns {
        Some(c) if c.is_empty() => Err(ConfigError::EmptyColumns),
        _ => Ok(()),
    }
}

struct Writer<'c, 'h> {
    config: &'c UnparseConfig,
    hooks: UnparseHooks<'h>,
    delimiter: &'c str,
    escaped_quote: String,
    scratch: String,
}

impl<'c, 'h> Writer<'c, 'h> {
    fn new(config: &'c UnparseConfig, hooks: UnparseHooks<'h>) -> Self {
        let delimiter = if BAD_DELIMITERS.iter().any(|b| config.delimiter.contains(b)) {
            ","
        } else {
            &config.delimiter
        };
        let escape = config.escape_char.as_deref().unwrap_or(&config.quote_char);
        Self {
            config,
            hooks,
            delimiter,
            escaped_quote: format!("{escape}{}", config.quote_char),
            scratch: String::new(),
        }
    }

    /// JS: `serialize(fields, data, skipEmptyLines)`.
    fn serialize<R: UnparseRow>(&mut self, fields: &[&str], data: &[R]) -> String {
        let skip = self.config.skip_empty_lines;
        let has_header = !fields.is_empty();
        let keyed = data.first().is_some_and(|r| r.keys().is_some());
        let mut csv = String::new();

        if has_header && self.config.header {
            for (i, field) in fields.iter().enumerate() {
                if i > 0 {
                    csv.push_str(self.delimiter);
                }
                self.safe(
                    &mut csv,
                    Some(&Value::String(Cow::Borrowed(field))),
                    i,
                    None,
                );
            }
            if !data.is_empty() {
                csv.push_str(&self.config.newline);
            }
        }

        let by_name = (has_header && keyed).then_some(fields);

        for (i, row) in data.iter().enumerate() {
            let max_col = if has_header {
                fields.len()
            } else if row.keys().is_some() {
                0 // JS: `object.length` is undefined
            } else {
                row.width()
            };
            let null_line = row.width() == 0;

            let empty_line = match skip {
                SkipEmptyLines::No => false,
                SkipEmptyLines::Greedy => {
                    let mut text = String::new();
                    for col in 0..max_col {
                        if let Some(v) = cell(row, col, by_name) {
                            js_text(&mut text, v);
                        }
                    }
                    text.trim_matches(crate::is_js_whitespace).is_empty()
                }
                SkipEmptyLines::Yes => {
                    !has_header
                        && row.width() == 1
                        && matches!(row.at(0), Some(Value::String(s)) if s.is_empty())
                }
            };
            if empty_line {
                continue;
            }
            for col in 0..max_col {
                if col > 0 && !null_line {
                    csv.push_str(self.delimiter);
                }
                self.safe(&mut csv, cell(row, col, by_name), col, Some(i));
            }
            if i + 1 < data.len() && (skip == SkipEmptyLines::No || (max_col > 0 && !null_line)) {
                csv.push_str(&self.config.newline);
            }
        }
        csv
    }

    /// Writes a cell, quoting it if needed. JS: `safe(str, col)`.
    fn safe(
        &mut self,
        out: &mut String,
        value: Option<&Value<'_>>,
        col: usize,
        row: Option<usize>,
    ) {
        let value = match value {
            None | Some(Value::Null) => return,
            Some(Value::Date(iso)) => {
                out.push_str(iso);
                return;
            }
            Some(v) => v,
        };
        // Non-string cells are printed into a buffer reused across cells.
        let mut scratch = std::mem::take(&mut self.scratch);
        let text: &str = if let Value::String(s) = value {
            s
        } else {
            scratch.clear();
            js_text(&mut scratch, value);
            &scratch
        };

        let formula = matches!(value, Value::String(_)) && self.is_formula(text);
        let forced = formula
            || match (&mut self.hooks.quotes, &self.config.quotes) {
                (Some(f), _) => f(value, col, row),
                (None, Quotes::Needed) => false,
                (None, Quotes::All) => true,
                (None, Quotes::Columns(c)) => c.get(col).copied().unwrap_or(false),
                (None, Quotes::Dynamic(f)) => f(value, col),
            };
        let quote = self.config.quote_char.as_str();

        if !formula && self.is_plain(text) {
            // No quote, delimiter or line break inside: nothing to escape.
            if forced || text.starts_with(' ') || text.ends_with(' ') {
                out.push_str(quote);
                out.push_str(text);
                out.push_str(quote);
            } else {
                out.push_str(text);
            }
        } else {
            let prefixed;
            let text = if formula {
                prefixed = format!("'{text}");
                prefixed.as_str()
            } else {
                text
            };
            let escaped = if text.contains(quote) {
                Cow::Owned(text.replace(quote, &self.escaped_quote))
            } else {
                Cow::Borrowed(text)
            };
            let needs_quotes = forced
                || BAD_DELIMITERS.iter().any(|b| escaped.contains(b))
                || escaped.contains(self.delimiter)
                || text.contains(quote)
                || escaped.starts_with(' ')
                || escaped.ends_with(' ');
            if needs_quotes {
                out.push_str(quote);
                out.push_str(&escaped);
                out.push_str(quote);
            } else {
                out.push_str(&escaped);
            }
        }
        self.scratch = scratch;
    }

    /// `true` if `text` holds no byte that could start a quote, the delimiter, a line
    /// break or a byte order mark: then it needs no escaping, and only spaces at its ends
    /// (or the config) can call for quotes. A `false` just means "check properly".
    fn is_plain(&self, text: &str) -> bool {
        let (Some(&q), Some(&d)) = (
            self.config.quote_char.as_bytes().first(),
            self.delimiter.as_bytes().first(),
        ) else {
            return false;
        };
        !text
            .bytes()
            .any(|b| matches!(b, b'\r' | b'\n' | b'"' | 0xEF) || b == q || b == d)
    }

    fn is_formula(&mut self, text: &str) -> bool {
        if let Some(f) = &mut self.hooks.escape_formulae {
            return f(text);
        }
        match &self.config.escape_formulae {
            EscapeFormulae::Off => false,
            // JS: /^[=+\-@\t\r].*$/ — `.` stops at line terminators and `$` is the end.
            EscapeFormulae::Default => {
                let mut chars = text.chars();
                matches!(chars.next(), Some('=' | '+' | '-' | '@' | '\t' | '\r'))
                    && !chars.any(|c| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
            }
            EscapeFormulae::Custom(f) => f(text),
        }
    }
}

/// The cell of `row` in column `col`: by header name for keyed rows, else by position.
fn cell<'r, R: UnparseRow>(
    row: &'r R,
    col: usize,
    by_name: Option<&[&str]>,
) -> Option<&'r Value<'r>> {
    match by_name {
        Some(fields) => row.named_at(fields[col], col),
        None => row.at(col),
    }
}

/// Appends `value` as JavaScript's `String(value)` would print it (arrays join with `,`,
/// `null` inside an array is empty).
fn js_text(out: &mut String, value: &Value<'_>) {
    match value {
        Value::Date(s) if s.is_empty() => out.push_str("Invalid Date"),
        Value::String(s) | Value::Date(s) => out.push_str(s),
        Value::Number(n) => push_js_number(out, *n),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Null => {}
        Value::List(items) => {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                js_text(out, item);
            }
        }
    }
}

/// Appends `n` formatted like JavaScript's `Number.prototype.toString()`.
pub(crate) fn push_js_number(out: &mut String, n: f64) {
    use fmt::Write as _;
    if n.is_nan() {
        return out.push_str("NaN");
    }
    if n.is_infinite() {
        return out.push_str(if n > 0.0 { "Infinity" } else { "-Infinity" });
    }
    if n == 0.0 {
        return out.push('0'); // also -0
    }
    // Without an exponent JS and Rust print the same shortest round-trip digits.
    if (1e-6..1e21).contains(&n.abs()) {
        let _ = write!(out, "{n}");
        return;
    }
    if n < 0.0 {
        out.push('-');
    }
    // Shortest round-trip digits, as JS uses: `d.ddddde±x`.
    let sci = format!("{:e}", n.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let k = i64::try_from(digits.len()).unwrap_or(i64::MAX);
    let point = exp.parse::<i64>().unwrap_or(0) + 1; // JS's `n`: digits before the point

    if k <= point && point <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n(
            '0',
            usize::try_from(point - k).unwrap_or(0),
        ));
    } else if 0 < point && point <= 21 {
        let (int, frac) = digits.split_at(usize::try_from(point).unwrap_or(0));
        let _ = write!(out, "{int}.{frac}");
    } else if -6 < point && point <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n(
            '0',
            usize::try_from(-point).unwrap_or(0),
        ));
        out.push_str(&digits);
    } else {
        let (first, rest) = digits.split_at(1);
        out.push_str(first);
        if !rest.is_empty() {
            out.push('.');
            out.push_str(rest);
        }
        let e = point - 1;
        let _ = write!(out, "e{}{}", if e < 0 { '-' } else { '+' }, e.abs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_like_javascript() {
        for (n, js) in [
            (36.0, "36"),
            (-1.5, "-1.5"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (123_456_789_012_345_680_000.0, "123456789012345680000"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (-0.0, "0"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
        ] {
            let mut s = String::new();
            push_js_number(&mut s, n);
            assert_eq!(s, js, "{n:?}");
        }
    }

    #[test]
    fn round_trips_parse_output() {
        let input = "a,b\r\n\"x, y\",\"say \"\"hi\"\"\"\r\n1,";
        let cfg = crate::ParseConfig::default().header(true);
        let rows = crate::parse(input, &cfg).unwrap().data;
        assert_eq!(unparse(&rows, &UnparseConfig::default()).unwrap(), input);
    }

    #[test]
    fn rejects_empty_columns() {
        let rows: Vec<Vec<Value<'_>>> = Vec::new();
        let cfg = UnparseConfig::default().columns(Vec::<String>::new());
        assert_eq!(unparse(&rows, &cfg), Err(ConfigError::EmptyColumns));
    }
}
