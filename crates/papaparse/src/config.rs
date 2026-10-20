use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// Callback type for `transform`: `(value, field) -> new value`.
pub type TransformFn = Arc<dyn Fn(&str, Field<'_>) -> String + Send + Sync>;
/// Callback type for `transform_header`: `(header, column index) -> new header`.
pub type TransformHeaderFn = Arc<dyn Fn(&str, usize) -> String + Send + Sync>;
/// Callback type for a computed delimiter: `(input) -> delimiter`.
pub type DelimiterFn = Arc<dyn Fn(&str) -> String + Send + Sync>;
/// Callback type for per-field dynamic typing: `(field) -> should convert`.
pub type DynamicTypingFn = Arc<dyn Fn(Field<'_>) -> bool + Send + Sync>;

/// Borrowed counterpart of [`TransformFn`], for [`crate::Hooks`].
pub type TransformHook<'h> = &'h mut dyn FnMut(&str, Field<'_>) -> String;
/// Borrowed counterpart of [`TransformHeaderFn`], for [`crate::Hooks`].
pub type TransformHeaderHook<'h> = &'h mut dyn FnMut(&str, usize) -> String;
/// Borrowed counterpart of [`DynamicTypingFn`], for [`crate::Hooks`].
pub type DynamicTypingHook<'h> = &'h mut dyn FnMut(Field<'_>) -> bool;

/// Identifies a column: by index when parsing without a header, by name otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field<'a> {
    /// Zero-based column index (no header).
    Index(usize),
    /// Header name.
    Name(&'a str),
}

impl fmt::Display for Field<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Field::Index(i) => write!(f, "{i}"),
            Field::Name(n) => f.write_str(n),
        }
    }
}

/// How fields are separated. JS: `config.delimiter`.
#[derive(Clone, Default)]
pub enum Delimiter {
    /// Guess from the input (JS: `""` / unset).
    #[default]
    Auto,
    /// A fixed delimiter; may be longer than one character.
    Fixed(String),
    /// Computed from the input (JS: a function).
    Dynamic(DelimiterFn),
}

/// Line terminator. `None` in [`ParseConfig::newline`] means auto-detect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Newline {
    /// `\n`
    #[default]
    Lf,
    /// `\r`
    Cr,
    /// `\r\n`
    CrLf,
}

impl Newline {
    /// The terminator as text.
    pub fn as_str(self) -> &'static str {
        match self {
            Newline::Lf => "\n",
            Newline::Cr => "\r",
            Newline::CrLf => "\r\n",
        }
    }
}

/// Which lines count as empty and get skipped. JS: `config.skipEmptyLines`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkipEmptyLines {
    /// Keep every line (`false`).
    #[default]
    No,
    /// Skip lines with a single empty field (`true`).
    Yes,
    /// Skip lines whose fields are all whitespace (`"greedy"`).
    Greedy,
}

/// Comment marker. JS: `config.comments` (`true` means `"#"`).
pub type Comments = Option<String>;

/// Whether to convert numeric/boolean/date strings. JS: `config.dynamicTyping`.
#[derive(Clone, Default)]
pub enum DynamicTyping {
    /// Keep every cell a string (`false`).
    #[default]
    Off,
    /// Convert every column (`true`).
    All,
    /// Per column, keyed by header name or by column index as a string (`"0"`, `"1"`, ...).
    Fields(HashMap<String, bool>),
    /// Decided per column by a callback, called once per column.
    Dynamic(DynamicTypingFn),
}

/// Options for [`crate::parse`] and [`crate::Parser`]. Mirrors PapaParse's config object.
///
/// Built with chained setters from [`ParseConfig::default`]:
///
/// ```
/// # use papaparse::{ParseConfig, Newline};
/// let cfg = ParseConfig::default().delimiter(";").newline(Newline::CrLf).header(true);
/// ```
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct ParseConfig {
    /// Field separator; auto-detected by default.
    pub delimiter: Delimiter,
    /// `None` auto-detects.
    pub newline: Option<Newline>,
    /// `None` means `"`.
    pub quote_char: Option<char>,
    /// `None` means the quote character escapes itself (`""`).
    pub escape_char: Option<char>,
    /// Treat the first row as a header and produce [`crate::Record`]s.
    pub header: bool,
    /// Rewrites each header name before de-duplication.
    pub transform_header: Option<TransformHeaderFn>,
    /// Convert numbers, booleans, empty cells and ISO dates.
    pub dynamic_typing: DynamicTyping,
    /// Maximum number of rows to parse; `0` means no limit.
    pub preview: usize,
    /// Lines starting with this marker are skipped.
    pub comments: Comments,
    /// Which empty lines to drop.
    pub skip_empty_lines: SkipEmptyLines,
    /// Number of leading lines to drop before parsing.
    pub skip_first_n_lines: usize,
    /// `Some(true)` forces the quote-free fast path, `Some(false)` disables it,
    /// `None` picks it when the input contains no quote character.
    pub fast_mode: Option<bool>,
    /// Rewrites each cell before dynamic typing.
    pub transform: Option<TransformFn>,
    /// Candidates for delimiter auto-detection; empty uses the PapaParse defaults.
    pub delimiters_to_guess: Vec<String>,
}

impl ParseConfig {
    /// Use a fixed delimiter (may be several characters).
    #[must_use]
    pub fn delimiter(mut self, delimiter: impl Into<String>) -> Self {
        self.delimiter = Delimiter::Fixed(delimiter.into());
        self
    }

    /// Compute the delimiter from the input.
    #[must_use]
    pub fn delimiter_fn(mut self, f: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        self.delimiter = Delimiter::Dynamic(Arc::new(f));
        self
    }

    /// Use a fixed line terminator instead of detecting it.
    #[must_use]
    pub fn newline(mut self, newline: Newline) -> Self {
        self.newline = Some(newline);
        self
    }

    /// Character that encloses quoted fields (default `"`).
    #[must_use]
    pub fn quote_char(mut self, quote: char) -> Self {
        self.quote_char = Some(quote);
        self
    }

    /// Character that escapes a quote inside a quoted field (default: the quote itself).
    #[must_use]
    pub fn escape_char(mut self, escape: char) -> Self {
        self.escape_char = Some(escape);
        self
    }

    /// Treat the first row as a header.
    #[must_use]
    pub fn header(mut self, header: bool) -> Self {
        self.header = header;
        self
    }

    /// Rewrite header names: `(name, column index) -> new name`.
    #[must_use]
    pub fn transform_header(
        mut self,
        f: impl Fn(&str, usize) -> String + Send + Sync + 'static,
    ) -> Self {
        self.transform_header = Some(Arc::new(f));
        self
    }

    /// `true` converts every column; use [`ParseConfig::dynamic_typing_with`] for finer control.
    #[must_use]
    pub fn dynamic_typing(mut self, enabled: bool) -> Self {
        self.dynamic_typing = if enabled {
            DynamicTyping::All
        } else {
            DynamicTyping::Off
        };
        self
    }

    /// Set per-column or callback-driven dynamic typing.
    #[must_use]
    pub fn dynamic_typing_with(mut self, typing: DynamicTyping) -> Self {
        self.dynamic_typing = typing;
        self
    }

    /// Stop after this many rows (`0` = no limit).
    #[must_use]
    pub fn preview(mut self, rows: usize) -> Self {
        self.preview = rows;
        self
    }

    /// Skip lines that start with `marker`.
    #[must_use]
    pub fn comments(mut self, marker: impl Into<String>) -> Self {
        self.comments = Some(marker.into());
        self
    }

    /// Choose which empty lines to drop.
    #[must_use]
    pub fn skip_empty_lines(mut self, mode: SkipEmptyLines) -> Self {
        self.skip_empty_lines = mode;
        self
    }

    /// Drop this many leading lines before parsing.
    #[must_use]
    pub fn skip_first_n_lines(mut self, n: usize) -> Self {
        self.skip_first_n_lines = n;
        self
    }

    /// Force (`true`) or forbid (`false`) the quote-free fast path.
    #[must_use]
    pub fn fast_mode(mut self, fast: bool) -> Self {
        self.fast_mode = Some(fast);
        self
    }

    /// Rewrite each cell: `(value, field) -> new value`.
    #[must_use]
    pub fn transform(
        mut self,
        f: impl Fn(&str, Field<'_>) -> String + Send + Sync + 'static,
    ) -> Self {
        self.transform = Some(Arc::new(f));
        self
    }

    /// Replace the delimiter candidates used for detection.
    #[must_use]
    pub fn delimiters_to_guess<I, S>(mut self, delimiters: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.delimiters_to_guess = delimiters.into_iter().map(Into::into).collect();
        self
    }

    pub(crate) fn quote(&self) -> char {
        self.quote_char.unwrap_or('"')
    }
}

impl fmt::Debug for Delimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Delimiter::Auto => f.write_str("Auto"),
            Delimiter::Fixed(d) => f.debug_tuple("Fixed").field(d).finish(),
            Delimiter::Dynamic(_) => f.write_str("Dynamic(<fn>)"),
        }
    }
}

impl fmt::Debug for DynamicTyping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DynamicTyping::Off => f.write_str("Off"),
            DynamicTyping::All => f.write_str("All"),
            DynamicTyping::Fields(m) => f.debug_tuple("Fields").field(m).finish(),
            DynamicTyping::Dynamic(_) => f.write_str("Dynamic(<fn>)"),
        }
    }
}

impl fmt::Debug for ParseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let callback = |set: bool| if set { "Some(<fn>)" } else { "None" };
        f.debug_struct("ParseConfig")
            .field("delimiter", &self.delimiter)
            .field("newline", &self.newline)
            .field("quote_char", &self.quote_char)
            .field("escape_char", &self.escape_char)
            .field("header", &self.header)
            .field(
                "transform_header",
                &callback(self.transform_header.is_some()),
            )
            .field("dynamic_typing", &self.dynamic_typing)
            .field("preview", &self.preview)
            .field("comments", &self.comments)
            .field("skip_empty_lines", &self.skip_empty_lines)
            .field("skip_first_n_lines", &self.skip_first_n_lines)
            .field("fast_mode", &self.fast_mode)
            .field("transform", &callback(self.transform.is_some()))
            .field("delimiters_to_guess", &self.delimiters_to_guess)
            .finish()
    }
}
