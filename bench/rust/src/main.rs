//! Native runner for `bench/run.mjs`, speaking the same protocol as `runners/common.mjs`:
//! `papaparse-bench <file> <config-json> <iterations>` prints
//! `{"times":[ms...],"rows":n,"errors":n,"checksum":"fnv1a"}`. `config.mode` is `parse`,
//! `step` (rows one at a time) or `unparse` (the parsed rows written back to CSV).

use std::time::Instant;

use papaparse::{ChunkParser, Flow, ParseConfig, Row, UnparseConfig, parse, unparse};
use serde_json::{Value as Json, json};

/// What one timed run produced; checksummed after timing, as the JS runners do.
enum Outcome<'a> {
    Parsed(papaparse::ParseOutput<'a>),
    Stepped(Vec<Row<'a>>, usize),
    Csv(String, usize),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [file, config_json, iterations] = args.as_slice() else {
        return Err("usage: papaparse-bench <file> <config-json> <iterations>".into());
    };
    let input = std::fs::read_to_string(file)?;
    let cfg: Json = serde_json::from_str(config_json)?;
    let config = to_config(&cfg);
    let iterations: usize = iterations.parse()?;

    let parsed = parse(&input, &config)?; // also the warm-up, mirroring the JS runners
    let mode = cfg["mode"].as_str().unwrap_or("parse");
    let once = || -> Result<Outcome<'_>, Box<dyn std::error::Error>> {
        Ok(match mode {
            "step" => {
                let mut rows: Vec<Row<'static>> = Vec::new();
                let mut errors = 0;
                let mut parser = ChunkParser::new(&config)?;
                let mut on_row = |s: papaparse::Step<'_>| {
                    errors += s.errors.len();
                    rows.push(s.row.into_owned());
                    Flow::Continue
                };
                parser.feed_rows(&input, &mut on_row)?;
                parser.finish_rows(&mut on_row)?;
                Outcome::Stepped(rows, errors)
            }
            "unparse" => Outcome::Csv(
                unparse(&parsed.data, &UnparseConfig::default())?,
                parsed.data.len(),
            ),
            _ => Outcome::Parsed(parse(&input, &config)?),
        })
    };

    let mut times = Vec::with_capacity(iterations);
    let mut result = None;
    for _ in 0..iterations {
        drop(result.take()); // free the previous result outside the timed region
        let start = Instant::now();
        let outcome = once()?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        result = Some(outcome);
    }
    let (rows, errors, text) = match result.ok_or("iterations must be > 0")? {
        Outcome::Parsed(out) => (
            out.data.len(),
            out.errors.len(),
            serde_json::to_string(&out.data)?,
        ),
        Outcome::Stepped(rows, errors) => (rows.len(), errors, serde_json::to_string(&rows)?),
        Outcome::Csv(csv, rows) => (rows, 0, csv),
    };
    println!(
        "{}",
        json!({ "times": times, "rows": rows, "errors": errors, "checksum": fnv1a(&text) })
    );
    Ok(())
}

fn to_config(cfg: &Json) -> ParseConfig {
    let mut c = ParseConfig::default()
        .header(cfg["header"].as_bool().unwrap_or(false))
        .dynamic_typing(cfg["dynamicTyping"].as_bool().unwrap_or(false));
    if let Some(d) = cfg["delimiter"].as_str() {
        c = c.delimiter(d);
    }
    c
}

fn fnv1a(s: &str) -> String {
    let hash = s.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    format!("{hash:08x}")
}
