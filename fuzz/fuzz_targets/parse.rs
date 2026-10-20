//! Parsing arbitrary input with arbitrary options must never panic, and must uphold
//! basic invariants. The first bytes choose the options; the rest is the input.
#![no_main]

use libfuzzer_sys::fuzz_target;
use papaparse::{Flow, Newline, ParseConfig, Parser, SkipEmptyLines, parse};

const DELIMS: [&str; 8] = [",", ";", "\t", "|", ", ", "é", "::", "\u{1e}"];
const QUOTES: [char; 4] = ['"', '\'', '|', 'é'];
const ESCAPES: [Option<char>; 3] = [None, Some('\\'), Some('"')];
const NEWLINES: [Option<Newline>; 4] = [None, Some(Newline::Lf), Some(Newline::Cr), Some(Newline::CrLf)];

fuzz_target!(|data: &[u8]| {
    let [a, b, c, rest @ ..] = data else { return };
    let Ok(input) = std::str::from_utf8(rest) else { return };

    let mut config = ParseConfig::default()
        .header(a & 1 != 0)
        .dynamic_typing(a & 2 != 0)
        .preview(usize::from(a >> 5))
        .quote_char(QUOTES[usize::from(b & 3)])
        .skip_first_n_lines(usize::from((c >> 6) & 1));
    config.newline = NEWLINES[usize::from((b >> 2) & 3)];
    config.escape_char = ESCAPES[usize::from((b >> 4) % 3)];
    if b & 0x40 != 0 {
        config = config.delimiter(DELIMS[usize::from(c & 7)]);
    }
    if a & 4 != 0 {
        config = config.comments("#");
    }
    config = config.skip_empty_lines(match (a >> 3) & 3 {
        0 => SkipEmptyLines::No,
        1 => SkipEmptyLines::Yes,
        _ => SkipEmptyLines::Greedy,
    });
    if c & 8 != 0 {
        config = config.fast_mode(c & 16 != 0);
    }

    // High-level API: must not panic; cursor stays within the input.
    if let Ok(out) = parse(input, &config) {
        assert!(out.meta.cursor <= input.len());
        for e in &out.errors {
            assert!(e.index.is_none_or(|i| i <= input.len()));
        }
    }

    // Low-level API with a step callback that aborts part-way.
    if let Ok(mut parser) = Parser::new(&config) {
        let stop_after = usize::from(c >> 5);
        let mut rows = 0;
        parser.parse_with_step(input, &mut |r| {
            rows += r.data.len();
            if rows > stop_after { Flow::Abort } else { Flow::Continue }
        });
    }
});
