//! `Papa.unparse` transport. `js/papa.cjs` resolves the input's shape (arrays, objects,
//! `{ fields, data }`) the way PapaParse does and flattens the cells to write into
//! [`UnparseCells`]: one joined string plus per-cell kinds and lengths, so the whole
//! table crosses into Rust in a handful of values instead of one per cell.

use std::borrow::Cow;

use papaparse::{
    EscapeFormulae, Quotes, UnparseConfig, UnparseHooks, UnparseRow, Value, unparse_with_hooks,
};

use crate::options::{BoolOrString, skip_empty_lines};

/// Cell kinds; must match `js/papa.cjs`.
pub mod kind {
    /// A JS string.
    pub const STRING: u8 = 0;
    /// A number from `numbers`.
    pub const NUMBER: u8 = 1;
    /// `true`
    pub const TRUE: u8 = 2;
    /// `false`
    pub const FALSE: u8 = 3;
    /// `null` or `undefined`: an empty cell.
    pub const EMPTY: u8 = 4;
    /// A date; its text is `toISOString()`, written unquoted (empty for an invalid date).
    pub const DATE: u8 = 5;
    /// Any other value; its text is `String(value)`.
    pub const OTHER: u8 = 6;
}

/// The cells to write, row by row, in column order.
#[derive(Debug)]
pub struct UnparseCells<'c> {
    /// Text of every `STRING`, `DATE` and `OTHER` cell, concatenated.
    pub text: &'c str,
    /// UTF-16 length of each of those cells' text.
    pub lens: &'c [u32],
    /// Kind of each cell.
    pub kinds: &'c [u8],
    /// Values of `NUMBER` cells.
    pub numbers: &'c [f64],
    /// Cells per row.
    pub row_lens: &'c [u32],
    /// Per row, `Object.keys(row).length` (decides whether a row is empty).
    pub row_widths: &'c [u32],
}

/// The unparse options, after `js/papa.cjs` applied PapaParse's type checks.
#[derive(Debug, Clone, Default)]
pub struct UnparseOptions {
    /// `quotes: true | false`
    pub quotes: Option<bool>,
    /// `quotes: [bool, ...]`
    pub quote_columns: Option<Vec<bool>>,
    /// `header`
    pub header: Option<bool>,
    /// `delimiter`
    pub delimiter: Option<String>,
    /// `newline`
    pub newline: Option<String>,
    /// `quoteChar`
    pub quote_char: Option<String>,
    /// `escapeChar`
    pub escape_char: Option<String>,
    /// `skipEmptyLines`
    pub skip_empty_lines: Option<BoolOrString>,
    /// `escapeFormulae: true`
    pub escape_formulae: Option<bool>,
}

impl UnparseOptions {
    fn into_config(self) -> UnparseConfig {
        let mut c = UnparseConfig::default()
            .skip_empty_lines(skip_empty_lines(self.skip_empty_lines.as_ref()));
        if let Some(columns) = self.quote_columns {
            c = c.quotes(Quotes::Columns(columns));
        } else if self.quotes == Some(true) {
            c = c.quotes(Quotes::All);
        }
        if let Some(h) = self.header {
            c = c.header(h);
        }
        if let Some(d) = self.delimiter {
            c = c.delimiter(d);
        }
        if let Some(n) = self.newline {
            c = c.newline(n);
        }
        if let Some(q) = self.quote_char {
            c = c.quote_char(q);
        }
        if let Some(e) = self.escape_char {
            c = c.escape_char(e);
        }
        if self.escape_formulae == Some(true) {
            c = c.escape_formulae(EscapeFormulae::Default);
        }
        c
    }
}

/// One row of [`UnparseCells`].
struct Line<'c> {
    width: usize,
    cells: Vec<Value<'c>>,
}

impl UnparseRow for Line<'_> {
    fn keys(&self) -> Option<Vec<&str>> {
        None // positional: the JS side already ordered the cells by `fields`
    }
    fn width(&self) -> usize {
        self.width
    }
    fn at(&self, index: usize) -> Option<&Value<'_>> {
        self.cells.get(index)
    }
    fn named(&self, _: &str) -> Option<&Value<'_>> {
        None
    }
}

/// Writes CSV. `fields` is the header (empty for none); `Err` is a message for a JS `Error`.
pub fn unparse(
    fields: &[String],
    cells: &UnparseCells<'_>,
    options: UnparseOptions,
    hooks: UnparseHooks<'_>,
) -> Result<String, String> {
    let lines = lines(cells).ok_or("papaparse: corrupt unparse input")?;
    unparse_with_hooks(Some(fields), &lines, &options.into_config(), hooks)
        .map_err(|e| e.to_string())
}

fn lines<'c>(c: &UnparseCells<'c>) -> Option<Vec<Line<'c>>> {
    let mut texts = split_utf16(c.text, c.lens);
    let mut numbers = c.numbers.iter();
    let mut kinds = c.kinds.iter();
    let mut out = Vec::with_capacity(c.row_lens.len());
    for (&len, &width) in c.row_lens.iter().zip(c.row_widths) {
        let mut cells = Vec::with_capacity(len as usize);
        for _ in 0..len {
            cells.push(match *kinds.next()? {
                kind::STRING => Value::String(Cow::Borrowed(texts.next()?)),
                kind::NUMBER => Value::Number(*numbers.next()?),
                kind::TRUE => Value::Bool(true),
                kind::FALSE => Value::Bool(false),
                kind::EMPTY => Value::Null,
                kind::DATE => Value::Date(Cow::Borrowed(texts.next()?)),
                // Not a string, so never formula-escaped: the core writes a list as its
                // items joined, which for one item is the item's text.
                kind::OTHER => Value::List(vec![Value::String(Cow::Borrowed(texts.next()?))]),
                _ => return None,
            });
        }
        out.push(Line {
            width: width as usize,
            cells,
        });
    }
    Some(out)
}

/// Splits `text` into pieces of the given UTF-16 lengths.
fn split_utf16<'t>(text: &'t str, lens: &'t [u32]) -> impl Iterator<Item = &'t str> + 't {
    let ascii = text.is_ascii();
    let mut rest = text;
    lens.iter().map_while(move |&units| {
        let units = units as usize;
        let end = if ascii {
            units
        } else {
            let mut seen = 0;
            rest.char_indices()
                .find_map(|(i, ch)| {
                    let at = (seen >= units).then_some(i);
                    seen += ch.len_utf16();
                    at
                })
                .unwrap_or(rest.len())
        };
        let (piece, tail) = rest.split_at_checked(end)?;
        rest = tail;
        Some(piece)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_by_utf16_length() {
        let pieces: Vec<_> = split_utf16("aé😀bc", &[2, 2, 0, 2]).collect();
        assert_eq!(pieces, ["aé", "😀", "", "bc"]);
    }

    #[test]
    fn writes_cells_like_papaparse() {
        let cells = UnparseCells {
            text: "x, y=z2018-05-04T21:08:03.269Z[1]",
            lens: &[4, 2, 24, 3],
            kinds: &[
                kind::STRING,
                kind::NUMBER,
                kind::EMPTY,
                kind::STRING,
                kind::DATE,
                kind::OTHER,
            ],
            numbers: &[1.5e-7],
            row_lens: &[3, 3],
            row_widths: &[3, 3],
        };
        let options = UnparseOptions {
            escape_formulae: Some(true),
            ..UnparseOptions::default()
        };
        let csv = unparse(
            &["a".into(), "b".into(), "c".into()],
            &cells,
            options,
            UnparseHooks::default(),
        );
        assert_eq!(
            csv.unwrap(),
            "a,b,c\r\n\"x, y\",1.5e-7,\r\n\"'=z\",2018-05-04T21:08:03.269Z,[1]"
        );
    }
}
