//! Whatever the cells contain, `unparse` must quote them so that parsing the CSV gives
//! the cells back.
#![no_main]

use libfuzzer_sys::fuzz_target;
use papaparse::{Newline, ParseConfig, Row, UnparseConfig, Value, parse, unparse};

// Single characters only: PapaParse (and so this port) quotes a cell only when it contains
// the delimiter, so with `::` the cells `[":", ""]` are written `:::` and can't round-trip.
const DELIMS: [&str; 4] = [",", ";", "\t", "|"];

fuzz_target!(|data: &[u8]| {
    let [a, rest @ ..] = data else { return };
    let Ok(text) = std::str::from_utf8(rest) else { return };

    // Cells are separated by \u{1} and rows by \u{2} in the fuzz input.
    let rows: Vec<Vec<Value<'_>>> = text
        .split('\u{2}')
        .map(|r| r.split('\u{1}').map(Value::from).collect())
        .collect();
    // A lone empty row can't be told apart from no row at all in CSV.
    if rows.len() == 1 && rows[0].len() == 1 && rows[0][0].as_str() == Some("") {
        return;
    }

    let delimiter = DELIMS[usize::from(a & 3)];
    let quote = if a & 4 != 0 { '\'' } else { '"' };
    let mut config = UnparseConfig::default()
        .delimiter(delimiter)
        .quote_char(quote.to_string());
    if a & 8 != 0 {
        // Like PapaParse, `unparse` escapes quotes but not the escape character itself, so
        // a cell ending in `\` would turn its closing quote into an escaped one.
        if text.contains('\\') {
            return;
        }
        config = config.escape_char("\\");
    }
    let csv = unparse(&rows, &config).unwrap();

    let mut back = ParseConfig::default()
        .delimiter(delimiter)
        .newline(Newline::CrLf)
        .quote_char(quote);
    if a & 8 != 0 {
        back = back.escape_char('\\');
    }
    let parsed = parse(&csv, &back).unwrap();
    let expected: Vec<Row<'_>> = rows.into_iter().map(Row::Fields).collect();
    assert_eq!(parsed.data, expected, "csv: {csv:?}");
});
