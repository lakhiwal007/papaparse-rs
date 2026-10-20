//! Feeding input in pieces must give the same rows as parsing it whole, wherever the
//! pieces are cut (including inside quoted fields, `\r\n` pairs and UTF-8 sequences).
#![no_main]

use libfuzzer_sys::fuzz_target;
use papaparse::{ChunkParser, Flow, Newline, ParseConfig, Row, SkipEmptyLines, parse};

const DELIMS: [&str; 4] = [",", ";", ", ", "é"];
const NEWLINES: [Newline; 3] = [Newline::Lf, Newline::Cr, Newline::CrLf];

fuzz_target!(|data: &[u8]| {
    let [a, b, rest @ ..] = data else { return };
    let Ok(input) = std::str::from_utf8(rest) else { return };

    // Header renaming and `skip_first_n_lines` only see the first piece, as in PapaParse,
    // and detection is per first piece: fix the format and leave those out.
    let mut config = ParseConfig::default()
        .delimiter(DELIMS[usize::from(a & 3)])
        .newline(NEWLINES[usize::from((a >> 2) % 3)])
        .dynamic_typing(a & 0x10 != 0)
        .skip_empty_lines(match (a >> 5) & 3 {
            0 => SkipEmptyLines::No,
            1 => SkipEmptyLines::Yes,
            _ => SkipEmptyLines::Greedy,
        });
    if a & 0x80 != 0 {
        config = config.comments("#");
    }
    let Ok(whole) = parse(input, &config) else { return };

    // Cut at a pseudo-random sequence of byte offsets, snapped to char boundaries.
    let mut parser = ChunkParser::new(&config).unwrap();
    let mut rows: Vec<Row<'static>> = Vec::new();
    let mut collect = |out: papaparse::ParseOutput<'_>| {
        rows.extend(out.data.into_iter().map(Row::into_owned));
        Flow::Continue
    };
    let (mut start, mut step) = (0, usize::from(*b) % 7 + 1);
    while start < input.len() {
        let mut end = (start + step).min(input.len());
        while !input.is_char_boundary(end) {
            end += 1;
        }
        parser.feed(&input[start..end], &mut collect).unwrap();
        start = end;
        step = step * 5 % 11 + 1;
    }
    parser.finish(&mut collect).unwrap();

    assert_eq!(rows, whole.data);
});
