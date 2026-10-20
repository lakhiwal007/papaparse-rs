//! Runs PapaParse's own test suite (extracted to `conformance/fixtures/*.json`) against
//! this crate.
//!
//! Every runnable case must pass unless it is listed in `conformance/expected-failures.txt`.
//! A listed case that starts passing also fails the run, so the list only ever shrinks.
//! Set `CONFORMANCE_REPORT=1` to print every failure with a diff.

#![cfg(feature = "serde")]
#![allow(clippy::unwrap_used)]

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::PathBuf;

use papaparse::{DynamicTyping, Newline, ParseConfig, Parser, SkipEmptyLines};
use serde_json::{Map, Value as Json, json};

fn conformance_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

#[derive(Clone, Copy, PartialEq)]
enum Suite {
    /// `new Papa.Parser(config).parse(input)`: deepEqual errors/data, meta is a subset.
    Core,
    /// `Papa.parse(input, config)`: deepEqual meta (if given), errors, data.
    Parse,
    /// `Papa.unparse(input, config)`: the CSV string.
    Unparse,
}

/// Suites that are extracted but not run yet, with the reason.
const PENDING_SUITES: &[(&str, &str)] = &[
    (
        "parse_async",
        "file/network/stream inputs are not ported yet",
    ),
    (
        "custom",
        "callback-driven tests need hand-written Rust equivalents",
    ),
];

enum Outcome {
    Pass,
    Fail(String),
    Skip(String),
}

#[test]
fn upstream_conformance() {
    let expected_failures = load_expected_failures();
    let mut report = String::new();
    let mut unexpected_failures = Vec::new();
    let mut unexpected_passes = Vec::new();
    let mut stats: HashMap<&str, [usize; 3]> = HashMap::new(); // pass, fail, skip
    let verbose = std::env::var_os("CONFORMANCE_REPORT").is_some();

    for (name, suite) in [
        ("core_parser", Suite::Core),
        ("parse", Suite::Parse),
        ("unparse", Suite::Unparse),
    ] {
        let fixture: Json = serde_json::from_str(
            &std::fs::read_to_string(conformance_dir().join(format!("fixtures/{name}.json")))
                .unwrap(),
        )
        .unwrap();

        for case in fixture["cases"].as_array().unwrap() {
            let id = format!("{name}::{}", case["description"].as_str().unwrap());
            let counts = stats.entry(name).or_default();
            if case.get("disabled").and_then(Json::as_bool) == Some(true) {
                counts[2] += 1;
                continue;
            }
            match run_case(suite, case) {
                Outcome::Pass => {
                    counts[0] += 1;
                    if expected_failures.contains(&id) {
                        unexpected_passes.push(id);
                    }
                }
                Outcome::Fail(diff) => {
                    counts[1] += 1;
                    if verbose {
                        let _ = writeln!(report, "FAIL {id}\n{diff}\n");
                    }
                    if !expected_failures.contains(&id) {
                        unexpected_failures.push(format!("{id}\n{diff}"));
                    }
                }
                Outcome::Skip(reason) => {
                    counts[2] += 1;
                    if verbose {
                        let _ = writeln!(report, "SKIP {id}: {reason}");
                    }
                }
            }
        }
    }

    for (name, [pass, fail, skip]) in &stats {
        let _ = writeln!(
            report,
            "{name}: {pass} passed, {fail} failed, {skip} skipped"
        );
    }
    for (name, reason) in PENDING_SUITES {
        let _ = writeln!(report, "{name}: pending ({reason})");
    }
    println!("{report}");

    assert!(
        unexpected_failures.is_empty(),
        "{} unexpected failure(s):\n\n{}",
        unexpected_failures.len(),
        unexpected_failures.join("\n\n")
    );
    assert!(
        unexpected_passes.is_empty(),
        "these now pass; remove them from conformance/expected-failures.txt:\n{}",
        unexpected_passes.join("\n")
    );
}

fn load_expected_failures() -> BTreeSet<String> {
    std::fs::read_to_string(conformance_dir().join("expected-failures.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn run_case(suite: Suite, case: &Json) -> Outcome {
    if suite == Suite::Unparse {
        return unparse::run_case(case);
    }
    let input = case["input"].as_str().unwrap();
    let empty = Map::new();
    let config_json = case
        .get("config")
        .and_then(Json::as_object)
        .unwrap_or(&empty);
    let config = match build_config(config_json) {
        Ok(c) => c,
        Err(reason) => return Outcome::Skip(reason),
    };
    let expected = &case["expected"];

    let (mut data, mut errors, mut meta) = match suite {
        Suite::Core => match Parser::new(&config) {
            Ok(mut p) => {
                let out = p.parse(input);
                (json!(out.data), json!(out.errors), json!(out.meta))
            }
            Err(e) => return Outcome::Fail(format!("config error: {e}")),
        },
        Suite::Parse => match papaparse::parse(input, &config) {
            Ok(out) => (json!(out.data), json!(out.errors), json!(out.meta)),
            Err(e) => return Outcome::Fail(format!("config error: {e}")),
        },
        Suite::Unparse => unreachable!("handled above"),
    };
    tag_dates(&mut data, config_json.contains_key("dynamicTyping"));

    let mut exp_errors = expected["errors"].clone();
    let mut exp_meta = expected.get("meta").cloned();
    // Offsets are bytes here and UTF-16 units in JS; they only agree on ASCII input.
    if !input.is_ascii() {
        for errs in [&mut errors, &mut exp_errors] {
            for e in errs.as_array_mut().into_iter().flatten() {
                e.as_object_mut().map(|o| o.remove("index"));
            }
        }
        for m in [Some(&mut meta), exp_meta.as_mut()].into_iter().flatten() {
            m.as_object_mut().map(|o| o.remove("cursor"));
        }
    }

    let mut diff = String::new();
    if !js_eq(&errors, &exp_errors) {
        let _ = writeln!(diff, "  errors:   {errors}\n  expected: {exp_errors}");
    }
    if !js_eq(&data, &expected["data"]) {
        let _ = writeln!(diff, "  data:     {data}\n  expected: {}", expected["data"]);
    }
    if let Some(exp_meta) = exp_meta {
        let ok = match suite {
            Suite::Core => is_subset(&meta, &exp_meta),
            Suite::Parse | Suite::Unparse => js_eq(&meta, &exp_meta),
        };
        if !ok {
            let _ = writeln!(diff, "  meta:     {meta}\n  expected: {exp_meta}");
        }
    }
    if diff.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail(format!("  input: {input:?}\n{diff}"))
    }
}

/// Maps a JS config object onto [`ParseConfig`]; `Err` names the reason to skip.
fn build_config(cfg: &Map<String, Json>) -> Result<ParseConfig, String> {
    if let Some((k, _)) = cfg.iter().find(|(_, v)| v.get("$fn").is_some()) {
        return Err(format!("`{k}` is a JS callback"));
    }
    let mut c = ParseConfig::default();
    for (key, v) in cfg {
        c = match (key.as_str(), v) {
            ("delimiter", Json::String(s)) => c.delimiter(s.clone()),
            ("newline", Json::String(s)) => match s.as_str() {
                "\n" => c.newline(Newline::Lf),
                "\r" => c.newline(Newline::Cr),
                "\r\n" => c.newline(Newline::CrLf),
                _ => c,
            },
            ("quoteChar", Json::String(s)) => c.quote_char(single_char(s)?),
            ("escapeChar", Json::String(s)) => c.escape_char(single_char(s)?),
            ("header", Json::Bool(b)) => c.header(*b),
            ("dynamicTyping", Json::Bool(b)) => c.dynamic_typing(*b),
            ("dynamicTyping", Json::Object(m)) => c.dynamic_typing_with(DynamicTyping::Fields(
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_bool() == Some(true)))
                    .collect(),
            )),
            ("preview", Json::Number(n)) => {
                c.preview(usize::try_from(n.as_u64().unwrap_or(0)).unwrap())
            }
            ("comments", Json::Bool(true)) => c.comments("#"),
            ("comments", Json::Bool(false)) => c,
            ("comments", Json::String(s)) => c.comments(s.clone()),
            ("skipEmptyLines", Json::Bool(b)) => c.skip_empty_lines(if *b {
                SkipEmptyLines::Yes
            } else {
                SkipEmptyLines::No
            }),
            ("skipEmptyLines", Json::String(s)) if s == "greedy" => {
                c.skip_empty_lines(SkipEmptyLines::Greedy)
            }
            ("skipFirstNLines", Json::Number(n)) => {
                c.skip_first_n_lines(usize::try_from(n.as_i64().unwrap_or(0).max(0)).unwrap())
            }
            ("fastMode", Json::Bool(b)) => c.fast_mode(*b),
            ("delimitersToGuess", Json::Array(a)) => {
                c.delimiters_to_guess(a.iter().filter_map(Json::as_str).map(str::to_owned))
            }
            _ => return Err(format!("unsupported option `{key}: {v}`")),
        };
    }
    Ok(c)
}

fn single_char(s: &str) -> Result<char, String> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(c),
        _ => Err(format!("multi-character quote/escape {s:?}")),
    }
}

/// Our `Value::Date` serializes as a plain string; the fixtures tag JS `Date`s as
/// `{"$date": iso}`. With dynamic typing on, wrap ISO-looking strings to match.
fn tag_dates(v: &mut Json, typing: bool) {
    if !typing {
        return;
    }
    match v {
        Json::String(s)
            if s.len() >= 17
                && s.as_bytes()[10] == b'T'
                && s[..4].bytes().all(|b| b.is_ascii_digit()) =>
        {
            *v = json!({ "$date": s.clone() });
        }
        Json::Array(a) => a.iter_mut().for_each(|x| tag_dates(x, true)),
        Json::Object(o) => o.values_mut().for_each(|x| tag_dates(x, true)),
        _ => {}
    }
}

/// Structural equality with numbers compared by value (`1` == `1.0`) and dates by presence
/// (JS normalizes to UTC; we keep the original text).
fn js_eq(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => x.as_f64() == y.as_f64(),
        (Json::Array(x), Json::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| js_eq(p, q))
        }
        (Json::Object(x), Json::Object(y))
            if x.contains_key("$date") && y.contains_key("$date") =>
        {
            true
        }
        (Json::Object(x), Json::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| js_eq(v, w)))
        }
        _ => a == b,
    }
}

/// chai `deepNestedInclude`: every key in `expected` is present and equal in `actual`.
fn is_subset(actual: &Json, expected: &Json) -> bool {
    match (actual, expected) {
        (Json::Object(a), Json::Object(e)) => {
            e.iter().all(|(k, v)| a.get(k).is_some_and(|w| js_eq(w, v)))
        }
        _ => js_eq(actual, expected),
    }
}

mod unparse {
    use std::sync::Arc;

    use papaparse::{
        EscapeFormulae, Quotes, SkipEmptyLines, UnparseConfig, UnparseRow, Value, unparse,
        unparse_table,
    };
    use serde_json::{Map, Value as Json};

    use super::Outcome;

    /// A JS array or plain object.
    enum JsRow {
        List(Vec<Value<'static>>),
        Object(Vec<(String, Value<'static>)>),
    }

    impl UnparseRow for JsRow {
        fn keys(&self) -> Option<Vec<&str>> {
            match self {
                JsRow::List(v) => v.keys(),
                JsRow::Object(o) => o.keys(),
            }
        }
        fn width(&self) -> usize {
            match self {
                JsRow::List(v) => v.width(),
                JsRow::Object(o) => o.width(),
            }
        }
        fn at(&self, index: usize) -> Option<&Value<'_>> {
            match self {
                JsRow::List(v) => v.at(index),
                JsRow::Object(o) => o.at(index),
            }
        }
        fn named(&self, name: &str) -> Option<&Value<'_>> {
            match self {
                JsRow::List(v) => v.named(name),
                JsRow::Object(o) => o.named(name),
            }
        }
    }

    fn value(v: &Json) -> Value<'static> {
        match v {
            Json::String(s) => Value::from(s.clone()),
            Json::Number(n) => Value::Number(n.as_f64().unwrap()),
            Json::Bool(b) => Value::Bool(*b),
            Json::Array(a) => Value::List(a.iter().map(value).collect()),
            Json::Object(o) => match o.get("$date").and_then(Json::as_str) {
                Some("Invalid Date") | None => Value::Null, // JS writes both as ''
                Some(iso) => Value::Date(iso.to_owned().into()),
            },
            Json::Null => Value::Null,
        }
    }

    fn row(v: &Json) -> JsRow {
        match v {
            Json::Object(o) => {
                JsRow::Object(o.iter().map(|(k, x)| (k.clone(), value(x))).collect())
            }
            Json::Array(a) => JsRow::List(a.iter().map(value).collect()),
            other => JsRow::List(vec![value(other)]),
        }
    }

    fn config(cfg: &Map<String, Json>) -> Result<UnparseConfig, String> {
        let mut c = UnparseConfig::default();
        for (key, v) in cfg {
            c = match (key.as_str(), v) {
                ("quotes", Json::Bool(b)) => {
                    c.quotes(if *b { Quotes::All } else { Quotes::Needed })
                }
                ("quotes", Json::Array(a)) => c.quotes(Quotes::Columns(
                    a.iter().map(|b| b.as_bool() == Some(true)).collect(),
                )),
                ("quotes", Json::Object(f))
                    if f.get("$fn")
                        .and_then(Json::as_str)
                        .is_some_and(|src| src.contains("typeof value === 'string'")) =>
                {
                    c.quotes(Quotes::Dynamic(Arc::new(|v: &Value<'_>, _| {
                        matches!(v, Value::String(_))
                    })))
                }
                ("delimiter", Json::String(s)) => c.delimiter(s.clone()),
                ("newline", Json::String(s)) => c.newline(s.clone()),
                ("quoteChar", Json::String(s)) => c.quote_char(s.clone()),
                ("escapeChar", Json::String(s)) => c.escape_char(s.clone()),
                ("header", Json::Bool(b)) => c.header(*b),
                ("columns", Json::Array(a)) => {
                    c.columns(a.iter().filter_map(Json::as_str).map(str::to_owned))
                }
                ("skipEmptyLines", Json::Bool(b)) => c.skip_empty_lines(if *b {
                    SkipEmptyLines::Yes
                } else {
                    SkipEmptyLines::No
                }),
                ("skipEmptyLines", Json::String(s)) if s == "greedy" => {
                    c.skip_empty_lines(SkipEmptyLines::Greedy)
                }
                ("escapeFormulae", Json::Bool(b)) => c.escape_formulae(if *b {
                    EscapeFormulae::Default
                } else {
                    EscapeFormulae::Off
                }),
                _ => return Err(format!("unsupported option `{key}: {v}`")),
            };
        }
        Ok(c)
    }

    pub(super) fn run_case(case: &Json) -> Outcome {
        let empty = Map::new();
        let config = match config(
            case.get("config")
                .and_then(Json::as_object)
                .unwrap_or(&empty),
        ) {
            Ok(c) => c,
            Err(reason) => return Outcome::Skip(reason),
        };
        let input = &case["input"];
        let result = match input {
            Json::Array(rows) => unparse(&rows.iter().map(row).collect::<Vec<_>>(), &config),
            Json::Object(o) => {
                let fields: Option<Vec<&str>> = o
                    .get("fields")
                    .and_then(Json::as_array)
                    .map(|f| f.iter().filter_map(Json::as_str).collect());
                let data = o
                    .get("data")
                    .and_then(Json::as_array)
                    .cloned()
                    .unwrap_or_default();
                // JS: a single row given as a flat array (`{ data: ["a", "b"] }`).
                let rows: Vec<JsRow> = if data
                    .first()
                    .is_some_and(|d| !d.is_array() && !d.is_object())
                {
                    vec![row(&Json::Array(data))]
                } else {
                    data.iter().map(row).collect()
                };
                unparse_table(fields.as_deref(), &rows, &config)
            }
            other => return Outcome::Skip(format!("input {other}")),
        };
        match (result, &case["expected"]) {
            (Ok(csv), Json::String(expected)) if csv == *expected => Outcome::Pass,
            (Ok(csv), expected) => {
                Outcome::Fail(format!("  got:      {csv:?}\n  expected: {expected}"))
            }
            (Err(e), expected) => Outcome::Fail(format!("  error: {e}\n  expected: {expected}")),
        }
    }
}
