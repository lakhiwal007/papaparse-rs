//! Compact transport of a [`ParseOutput`] to JavaScript.
//!
//! Building JS values cell by cell across the wasm boundary, or via one big `JSON.parse`,
//! costs more than PapaParse's whole parse. Instead each cell becomes a one-byte tag plus,
//! for text, a `(start, end)` pair of UTF-16 offsets into the *original JS string*. The
//! JS side (`js/decode.cjs`) rebuilds cells with `String.prototype.substring`, which V8
//! implements as cheap slices. Only text that does not exist verbatim in the input
//! (unescaped quotes, `transform` output, renamed headers) is copied into a side table.

use std::borrow::Cow;

use papaparse::{ParseOutput, Row, Value};

/// Cell tags; must match `js/decode.cjs`.
/// Cell tags.
pub mod tag {
    /// Text that is a slice of the input.
    pub const INPUT: u8 = 0;
    /// Text from the side table.
    pub const SIDE: u8 = 1;
    /// A number from `numbers`.
    pub const NUMBER: u8 = 2;
    /// `true`
    pub const TRUE: u8 = 3;
    /// `false`
    pub const FALSE: u8 = 4;
    /// `null`
    pub const NULL: u8 = 5;
    /// A date whose text is a slice of the input.
    pub const DATE_INPUT: u8 = 6;
    /// A date whose text is in the side table.
    pub const DATE_SIDE: u8 = 7;
}

/// A parse result in transport form (see the module docs).
#[derive(Debug, Default)]
pub struct Tape {
    /// One tag per cell, rows concatenated.
    pub tags: Vec<u8>,
    /// `(start, end)` UTF-16 offsets into the JS input, for `INPUT`/`DATE_INPUT` cells.
    pub spans: Vec<u32>,
    /// Values of `NUMBER` cells.
    pub numbers: Vec<f64>,
    /// Cells per row.
    pub row_lens: Vec<u32>,
    /// Side table for text that is not a slice of the input.
    pub strings: String,
    /// `(start, end)` UTF-16 offsets into `strings`, for `SIDE`/`DATE_SIDE` cells.
    pub string_spans: Vec<u32>,
}

impl Tape {
    /// Encodes `out`, whose borrowed text must come from `input`.
    pub fn encode(input: &str, out: &ParseOutput<'_>) -> Self {
        Self::encode_with(&mut Utf16Offsets::new(input), input, out)
    }

    /// [`Tape::encode`] with a converter for `input` that the caller reuses across calls
    /// (creating one scans the whole input). Rows must come in input order.
    pub fn encode_with(
        to_utf16: &mut Utf16Offsets<'_>,
        input: &str,
        out: &ParseOutput<'_>,
    ) -> Self {
        let cells = out
            .data
            .iter()
            .map(|r| match r {
                Row::Fields(f) => f.len(),
                Row::Record(r) => r.values().count(),
            })
            .sum();
        let mut enc = Encoder {
            input,
            to_utf16,
            side_len_utf16: 0,
            tape: Tape {
                tags: Vec::with_capacity(cells),
                spans: Vec::with_capacity(cells * 2),
                row_lens: Vec::with_capacity(out.data.len()),
                ..Tape::default()
            },
        };
        for row in &out.data {
            let start = enc.tape.tags.len();
            match row {
                Row::Fields(values) => values.iter().for_each(|v| enc.value(v)),
                Row::Record(r) => r.values().for_each(|v| enc.value(v)),
            }
            enc.tape.row_lens.push(to_u32(enc.tape.tags.len() - start));
        }
        enc.tape
    }
}

struct Encoder<'i, 'c, 'u> {
    input: &'i str,
    to_utf16: &'c mut Utf16Offsets<'u>,
    side_len_utf16: usize,
    tape: Tape,
}

impl Encoder<'_, '_, '_> {
    fn value(&mut self, v: &Value<'_>) {
        match v {
            Value::String(s) => self.text(s, tag::INPUT, tag::SIDE),
            Value::Date(s) => self.text(s, tag::DATE_INPUT, tag::DATE_SIDE),
            Value::Number(n) => {
                self.tape.tags.push(tag::NUMBER);
                self.tape.numbers.push(*n);
            }
            Value::Bool(b) => self.tape.tags.push(if *b { tag::TRUE } else { tag::FALSE }),
            Value::Null => self.tape.tags.push(tag::NULL),
            // Records flatten `__parsed_extra` via `Record::values`; lists don't occur elsewhere.
            Value::List(items) => items.iter().for_each(|i| self.value(i)),
        }
    }

    // `&Cow` on purpose: borrowed text becomes a span, owned text goes to the side table.
    #[allow(clippy::ptr_arg)]
    fn text(&mut self, s: &Cow<'_, str>, input_tag: u8, side_tag: u8) {
        let borrowed = match s {
            Cow::Borrowed(b) => offset_in(self.input, b),
            Cow::Owned(_) => None,
        };
        if let Some(start) = borrowed {
            let (a, b) = (
                self.to_utf16.convert(start),
                self.to_utf16.convert(start + s.len()),
            );
            self.tape.tags.push(input_tag);
            self.tape.spans.extend([to_u32(a), to_u32(b)]);
        } else {
            let start = self.side_len_utf16;
            self.side_len_utf16 += s.encode_utf16().count();
            self.tape.strings.push_str(s);
            self.tape.tags.push(side_tag);
            self.tape
                .string_spans
                .extend([to_u32(start), to_u32(self.side_len_utf16)]);
        }
    }
}

/// Byte offset of `s` within `input`, if `s` is a slice of it. Compares addresses only.
fn offset_in(input: &str, s: &str) -> Option<usize> {
    let base = input.as_ptr() as usize;
    let addr = s.as_ptr() as usize;
    (addr >= base && addr + s.len() <= base + input.len()).then(|| addr - base)
}

/// Offsets fit in `u32` on wasm32 (and JS strings are far shorter than 4 Gi units).
#[allow(clippy::cast_possible_truncation)]
fn to_u32(n: usize) -> u32 {
    debug_assert!(u32::try_from(n).is_ok());
    n as u32
}

/// Converts UTF-8 byte offsets to UTF-16 code-unit offsets (what JS string indices are).
///
/// Identity for ASCII input. Otherwise it walks forward from the previous query, so a
/// sequence of non-decreasing queries costs O(input) in total.
#[derive(Debug)]
pub struct Utf16Offsets<'i> {
    bytes: &'i [u8],
    ascii: bool,
    byte: usize,
    unit: usize,
}

impl<'i> Utf16Offsets<'i> {
    /// A converter for offsets into `input`.
    pub fn new(input: &'i str) -> Self {
        Self {
            bytes: input.as_bytes(),
            ascii: input.is_ascii(),
            byte: 0,
            unit: 0,
        }
    }

    /// The UTF-16 offset of byte offset `byte`.
    pub fn convert(&mut self, byte: usize) -> usize {
        if self.ascii {
            return byte;
        }
        if byte < self.byte {
            (self.byte, self.unit) = (0, 0);
        }
        // Every non-continuation byte starts a char (1 unit); 4-byte chars need 2 units.
        self.unit += self.bytes[self.byte..byte]
            .iter()
            .map(|&b| usize::from(b & 0xc0 != 0x80) + usize::from(b >= 0xf0))
            .sum::<usize>();
        self.byte = byte;
        self.unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use papaparse::{ParseConfig, parse};

    #[test]
    fn utf16_offsets_match_encode_utf16() {
        let s = "aé€😀b";
        let mut conv = Utf16Offsets::new(s);
        for (byte, _) in s.char_indices().chain([(s.len(), ' ')]) {
            assert_eq!(conv.convert(byte), s[..byte].encode_utf16().count());
        }
        assert_eq!(conv.convert(1), 1); // backwards query resets
    }

    #[test]
    fn borrowed_cells_become_spans_and_others_side_strings() {
        let input = "é,\"x\"\"y\"\n1,true";
        let out = parse(
            input,
            &ParseConfig::default().delimiter(",").dynamic_typing(true),
        )
        .unwrap();
        let tape = Tape::encode(input, &out);
        assert_eq!(tape.tags, [tag::INPUT, tag::SIDE, tag::NUMBER, tag::TRUE]);
        assert_eq!(tape.spans, [0, 1]); // "é" is one UTF-16 unit
        assert_eq!(tape.strings, "x\"y");
        assert_eq!(tape.string_spans, [0, 3]);
        assert_eq!(tape.numbers, [1.0]);
        assert_eq!(tape.row_lens, [2, 2]);
    }
}
