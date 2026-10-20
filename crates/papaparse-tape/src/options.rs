//! PapaParse's JS config object, as plain Rust data, and its mapping onto [`ParseConfig`].
//!
//! Each binding (wasm, Node-API) only reads JS values into [`Options`]; defaults,
//! validation and error messages live here so both bindings behave identically.

use std::collections::HashMap;

use papaparse::{DynamicTyping, Newline, ParseConfig, SkipEmptyLines};

/// `dynamicTyping`: `true`/`false` or `{ column: bool }`.
#[derive(Debug, Clone)]
pub enum Typing {
    /// `dynamicTyping: true | false`
    All(bool),
    /// `dynamicTyping: { name_or_index: bool }`
    Fields(HashMap<String, bool>),
}

/// A value that may be a boolean or a string (`comments`, `skipEmptyLines`).
#[derive(Debug, Clone)]
pub enum BoolOrString {
    /// A boolean.
    Bool(bool),
    /// A string.
    String(String),
}

/// The non-callback options of `Papa.parse`, with JS names in comments.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// `delimiter`
    pub delimiter: Option<String>,
    /// `newline`
    pub newline: Option<String>,
    /// `quoteChar`
    pub quote_char: Option<String>,
    /// `escapeChar`
    pub escape_char: Option<String>,
    /// `header`
    pub header: bool,
    /// `dynamicTyping`
    pub dynamic_typing: Option<Typing>,
    /// `preview` (JS number; non-positive or NaN means no limit)
    pub preview: Option<f64>,
    /// `comments`
    pub comments: Option<BoolOrString>,
    /// `skipEmptyLines`
    pub skip_empty_lines: Option<BoolOrString>,
    /// `skipFirstNLines`
    pub skip_first_n_lines: Option<f64>,
    /// `fastMode`
    pub fast_mode: Option<bool>,
    /// `delimitersToGuess`
    pub delimiters_to_guess: Vec<String>,
}

impl Options {
    /// Maps onto [`ParseConfig`]; `Err` carries a message for a JS `Error`.
    pub fn into_config(self) -> Result<ParseConfig, String> {
        let mut c = ParseConfig::default()
            .header(self.header)
            .preview(js_count(self.preview))
            .skip_first_n_lines(js_count(self.skip_first_n_lines))
            .delimiters_to_guess(self.delimiters_to_guess);
        if let Some(d) = self.delimiter {
            c = c.delimiter(d);
        }
        match self.newline.as_deref() {
            Some("\n") => c = c.newline(Newline::Lf),
            Some("\r") => c = c.newline(Newline::Cr),
            Some("\r\n") => c = c.newline(Newline::CrLf),
            _ => {} // anything else auto-detects, as in JS
        }
        if let Some(q) = single_char("quoteChar", self.quote_char)? {
            c = c.quote_char(q);
        }
        if let Some(e) = single_char("escapeChar", self.escape_char)? {
            c = c.escape_char(e);
        }
        c = match self.dynamic_typing {
            Some(Typing::All(b)) => c.dynamic_typing(b),
            Some(Typing::Fields(fields)) => c.dynamic_typing_with(DynamicTyping::Fields(fields)),
            None => c,
        };
        c = match self.comments {
            Some(BoolOrString::Bool(true)) => c.comments("#"),
            Some(BoolOrString::String(s)) => c.comments(s),
            _ => c,
        };
        c = c.skip_empty_lines(skip_empty_lines(self.skip_empty_lines.as_ref()));
        if let Some(f) = self.fast_mode {
            c = c.fast_mode(f);
        }
        Ok(c)
    }
}

/// JS truthiness: any non-empty string other than `"greedy"` means `true`.
pub(crate) fn skip_empty_lines(v: Option<&BoolOrString>) -> SkipEmptyLines {
    match v {
        Some(BoolOrString::Bool(true)) => SkipEmptyLines::Yes,
        Some(BoolOrString::String(s)) if s == "greedy" => SkipEmptyLines::Greedy,
        Some(BoolOrString::String(s)) if !s.is_empty() => SkipEmptyLines::Yes,
        _ => SkipEmptyLines::No,
    }
}

/// JS count semantics: negative, zero, NaN and infinity mean "off".
fn js_count(n: Option<f64>) -> usize {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    n.filter(|n| n.is_finite() && *n > 0.0)
        .map_or(0, |n| n as usize)
}

fn single_char(name: &str, s: Option<String>) -> Result<Option<char>, String> {
    let Some(s) = s else { return Ok(None) };
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(Some(c)),
        (None, _) => Ok(None), // "" means the default, as in JS
        _ => Err(format!("{name} must be a single character")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_number_semantics() {
        assert_eq!(js_count(Some(3.9)), 3);
        for off in [
            None,
            Some(-2.0),
            Some(0.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(js_count(off), 0);
        }
    }

    #[test]
    fn rejects_multi_char_quote() {
        let o = Options {
            quote_char: Some("ab".into()),
            ..Options::default()
        };
        assert_eq!(
            o.into_config().unwrap_err(),
            "quoteChar must be a single character"
        );
    }
}
