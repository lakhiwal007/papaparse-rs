//! Minimal JSON writer for the small, non-tabular part of a result (`errors` and `meta`),
//! producing what `JSON.stringify` would for PapaParse. Hand-written to keep serde out of
//! the binaries. Cells travel separately (see `tape`).

use std::fmt::Write as _;

use papaparse::{Meta, ParseError, ParseOutput};

use crate::tape::Utf16Offsets;

/// `{"errors":[...],"meta":{...}}`. `base` is the byte offset of the parsed slice in the
/// JS input; offsets are converted to UTF-16 units relative to that slice, as in JS.
pub fn info(input: &str, base: usize, out: &ParseOutput<'_>) -> String {
    let mut conv = Utf16Offsets::new(input);
    let base_units = conv.convert(base);
    let mut to_js = |byte: usize| conv.convert(base + byte) - base_units;
    let errors = errors_json(&out.errors, &mut to_js);
    let cursor = to_js(out.meta.cursor);
    format!(
        "{{\"errors\":{errors},\"meta\":{}}}",
        meta_json(&out.meta, Some(cursor), out.meta.fields.as_deref())
    )
}

/// The errors as a JSON array; `to_js` converts byte offsets to JS offsets.
pub(crate) fn errors_json(errors: &[ParseError], to_js: &mut dyn FnMut(usize) -> usize) -> String {
    let mut w = String::from("[");
    for (i, e) in errors.iter().enumerate() {
        if i > 0 {
            w.push(',');
        }
        error(&mut w, e, to_js);
    }
    w.push(']');
    w
}

/// `meta` as a JSON object, with `cursor` (already a JS offset) when given.
pub(crate) fn meta_json(m: &Meta, cursor: Option<usize>, fields: Option<&[String]>) -> String {
    let mut w = String::from("{\"delimiter\":");
    string(&mut w, &m.delimiter);
    w.push_str(",\"linebreak\":");
    string(&mut w, &m.linebreak);
    let _ = write!(
        w,
        ",\"aborted\":{},\"truncated\":{}",
        m.aborted, m.truncated
    );
    if let Some(cursor) = cursor {
        let _ = write!(w, ",\"cursor\":{cursor}");
    }
    w.push_str(",\"renamedHeaders\":");
    match &m.renamed_headers {
        None => w.push_str("null"),
        Some(pairs) => {
            w.push('{');
            for (i, (k, v)) in pairs.iter().enumerate() {
                if i > 0 {
                    w.push(',');
                }
                string(&mut w, k);
                w.push(':');
                string(&mut w, v);
            }
            w.push('}');
        }
    }
    if let Some(fields) = fields {
        w.push_str(",\"fields\":[");
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                w.push(',');
            }
            string(&mut w, f);
        }
        w.push(']');
    }
    w.push('}');
    w
}

fn error(w: &mut String, e: &ParseError, to_js: &mut dyn FnMut(usize) -> usize) {
    w.push_str("{\"type\":");
    string(w, &format!("{:?}", e.kind));
    w.push_str(",\"code\":");
    string(w, &format!("{:?}", e.code));
    w.push_str(",\"message\":");
    string(w, &e.message);
    if let Some(row) = e.row {
        let _ = write!(w, ",\"row\":{row}");
    }
    if let Some(index) = e.index {
        let _ = write!(w, ",\"index\":{}", to_js(index));
    }
    w.push('}');
}

/// Writes a JSON string literal, copying unescaped runs in bulk.
fn string(w: &mut String, s: &str) {
    w.push('"');
    let mut start = 0;
    for (i, b) in s.bytes().enumerate() {
        let escape = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0..=0x1f => "",
            _ => continue,
        };
        w.push_str(&s[start..i]);
        if escape.is_empty() {
            let _ = write!(w, "\\u{b:04x}");
        } else {
            w.push_str(escape);
        }
        start = i + 1;
    }
    w.push_str(&s[start..]);
    w.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use papaparse::{ParseConfig, parse, parsed_slice};

    #[test]
    fn errors_and_meta_with_utf16_offsets() {
        let input = "\u{feff}h\u{1},y\n\"é\"x,1\n7";
        let cfg = ParseConfig::default().delimiter(",").header(true);
        let out = parse(input, &cfg).unwrap();
        let base = input.len() - parsed_slice(input, &cfg).len();
        let s = info(input, base, &out);
        // Ground truth from PapaParse 5.7.0 for the same input and config; note the
        // UTF-16 `cursor` (13) on non-ASCII input.
        let expected = r#"{"errors":[{"type":"Quotes","code":"InvalidQuotes","message":"Trailing quote on quoted field is malformed","row":1,"index":6},{"type":"Quotes","code":"MissingQuotes","message":"Quoted field unterminated","row":1,"index":6},{"type":"FieldMismatch","code":"TooFewFields","message":"Too few fields: expected 2 fields but parsed 1","row":0}],"meta":{"delimiter":",","linebreak":"\n","aborted":false,"truncated":false,"cursor":13,"renamedHeaders":null,"fields":["h\u0001","y"]}}"#;
        assert_eq!(s, expected);
    }
}
