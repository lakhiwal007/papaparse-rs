//! Heap-accounting leak test. A tracking allocator counts live heap bytes; after each
//! parse result is dropped, the live total must return exactly to its baseline.
//! Runs without the libtest harness so no other test allocates concurrently.

#![allow(clippy::unwrap_used)]

use std::fmt::Write as _;

use papaparse::{
    ChunkParser, Flow, ParseConfig, Parser, Row, SkipEmptyLines, Step, UnparseConfig, parse,
    unparse,
};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn live_bytes() -> usize {
    dhat::HeapStats::get().curr_bytes
}

fn sample() -> String {
    let mut s = String::from("id,name,note,amount\n");
    for i in 0..2_000 {
        let note = if i % 5 == 0 {
            "\"says \"\"hi\"\",\nbye\""
        } else {
            "\"plain, quoted\""
        };
        writeln!(s, "{i},user{i},{note},{}.25", i % 97).unwrap();
    }
    s.push_str("bad,\"unterminated\n");
    s
}

fn main() {
    let _profiler = dhat::Profiler::builder().testing().build();
    let input = sample();
    let configs = [
        ParseConfig::default(),
        ParseConfig::default().header(true).dynamic_typing(true),
        ParseConfig::default()
            .header(true)
            .transform(|v, _| v.to_uppercase()),
        ParseConfig::default()
            .skip_empty_lines(SkipEmptyLines::Greedy)
            .comments("#")
            .preview(500),
    ];

    // Warm up lazily-initialised std state (e.g. hasher keys) before taking the baseline.
    drop(parse(&input, &configs[1]).unwrap());
    let baseline = live_bytes();

    for round in 0..5 {
        for (i, config) in configs.iter().enumerate() {
            let out = parse(&input, config).unwrap();
            assert!(out.data.len() > 100, "parsed {} rows", out.data.len());
            drop(out);
            let owned = parse(&input, config).unwrap().into_owned();
            drop(owned);
            let now = live_bytes();
            assert_eq!(
                now,
                baseline,
                "round {round}, config {i}: {} bytes retained",
                now.abs_diff(baseline)
            );
        }

        let mut parser = Parser::new(&ParseConfig::default().header(true)).unwrap();
        let mut rows = 0;
        parser.parse_with_step(&input, &mut |r| {
            rows += r.data.len();
            if rows > 1_000 {
                Flow::Abort
            } else {
                Flow::Continue
            }
        });
        drop(parser);
        assert_eq!(
            live_bytes(),
            baseline,
            "round {round}: step parser retained memory"
        );

        // Streaming in uneven pieces, by chunk and by row (aborting part-way), then
        // writing the rows back out.
        for config in &configs[..2] {
            let mut chunked = ChunkParser::new(config).unwrap();
            let mut rows: Vec<Row<'static>> = Vec::new();
            for piece in input.as_bytes().chunks(997) {
                let piece = std::str::from_utf8(piece).unwrap(); // ASCII sample
                chunked
                    .feed(piece, |out| {
                        rows.extend(out.data.into_iter().map(Row::into_owned));
                        Flow::Continue
                    })
                    .unwrap();
            }
            chunked
                .finish(|out| {
                    rows.extend(out.data.into_iter().map(Row::into_owned));
                    Flow::Continue
                })
                .unwrap();
            let csv = unparse(&rows, &UnparseConfig::default()).unwrap();
            assert!(csv.len() > input.len() / 2);
            drop((chunked, rows, csv));

            let mut by_row = ChunkParser::new(config).unwrap();
            let mut seen = 0;
            by_row
                .feed_rows(&input, |_: Step<'_>| {
                    seen += 1;
                    if seen > 1_500 {
                        Flow::Abort
                    } else {
                        Flow::Continue
                    }
                })
                .unwrap();
            drop(by_row);
            assert_eq!(
                live_bytes(),
                baseline,
                "round {round}: streaming or unparse retained memory"
            );
        }
    }
    println!(
        "no_leaks: ok (baseline {baseline} live bytes held steady across 65 parses and 10 unparses)"
    );
}
