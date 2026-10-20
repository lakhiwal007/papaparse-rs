//! `papaparse` — convert CSV to JSON and back from the command line.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser as ClapParser, Subcommand, ValueEnum};
use papaparse::{
    ChunkParser, DEFAULT_CHUNK_SIZE, EscapeFormulae, Flow, Newline, ParseConfig, Quotes, Sink,
    SkipEmptyLines, UnparseConfig, UnparseRow, Value, unparse, unparse_table,
};
use serde_json::Value as Json;

#[derive(Debug, ClapParser)]
#[command(
    version,
    about = "Convert CSV to JSON and back using a Rust port of PapaParse",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    parse: ParseArgs,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// CSV to JSON (the default when no subcommand is given).
    Parse(ParseArgs),
    /// JSON to CSV: an array of arrays, an array of objects, or `{ "fields", "data" }`.
    Unparse(UnparseArgs),
}

#[derive(Debug, Args)]
struct ParseArgs {
    /// Input file; reads stdin when omitted or `-`.
    input: Option<PathBuf>,

    /// Field delimiter (auto-detected when omitted).
    #[arg(short, long)]
    delimiter: Option<String>,

    /// Line terminator (auto-detected when omitted).
    #[arg(long, value_enum)]
    newline: Option<NewlineArg>,

    /// Quote character.
    #[arg(long)]
    quote_char: Option<char>,

    /// Escape character for quotes inside quoted fields.
    #[arg(long)]
    escape_char: Option<char>,

    /// Treat the first row as a header and emit objects.
    #[arg(short = 'H', long)]
    header: bool,

    /// Convert numbers, booleans and ISO dates.
    #[arg(short = 't', long)]
    dynamic_typing: bool,

    /// Skip lines starting with this marker.
    #[arg(short, long)]
    comments: Option<String>,

    /// Skip empty lines (`greedy` also skips whitespace-only lines).
    #[arg(short = 's', long, value_enum, num_args = 0..=1, default_missing_value = "yes")]
    skip_empty_lines: Option<SkipArg>,

    /// Only parse this many rows.
    #[arg(short, long, default_value_t = 0)]
    preview: usize,

    /// Output format. `ndjson` streams: memory stays flat however large the input.
    #[arg(short, long, value_enum, default_value_t = Format::Json)]
    format: Format,

    /// Exit with status 2 if any parse errors are reported.
    #[arg(long)]
    strict: bool,
}

#[derive(Debug, Args)]
struct UnparseArgs {
    /// JSON input file; reads stdin when omitted or `-`.
    input: Option<PathBuf>,

    /// Field delimiter.
    #[arg(short, long, default_value = ",")]
    delimiter: String,

    /// Line terminator.
    #[arg(long, value_enum, default_value_t = NewlineArg::Crlf)]
    newline: NewlineArg,

    /// Quote character.
    #[arg(long, default_value = "\"")]
    quote_char: String,

    /// Escape for quotes inside quoted cells (default: double the quote).
    #[arg(long)]
    escape_char: Option<String>,

    /// Quote every cell, not only those that need it.
    #[arg(short, long)]
    quotes: bool,

    /// Leave out the header row.
    #[arg(long)]
    no_header: bool,

    /// Columns to write for objects, in order (default: the first object's keys).
    #[arg(long, value_delimiter = ',')]
    columns: Option<Vec<String>>,

    /// Leave out empty rows (`greedy` also whitespace-only rows).
    #[arg(short = 's', long, value_enum, num_args = 0..=1, default_missing_value = "yes")]
    skip_empty_lines: Option<SkipArg>,

    /// Prefix cells starting with `=`, `+`, `-`, `@`, tab or CR with `'` so spreadsheets
    /// don't run them as formulas.
    #[arg(long)]
    escape_formulae: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum NewlineArg {
    Lf,
    Cr,
    Crlf,
}

impl NewlineArg {
    fn newline(self) -> Newline {
        match self {
            NewlineArg::Lf => Newline::Lf,
            NewlineArg::Cr => Newline::Cr,
            NewlineArg::Crlf => Newline::CrLf,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SkipArg {
    Yes,
    Greedy,
}

impl SkipArg {
    fn mode(self) -> SkipEmptyLines {
        match self {
            SkipArg::Yes => SkipEmptyLines::Yes,
            SkipArg::Greedy => SkipEmptyLines::Greedy,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    /// The full `{ data, errors, meta }` result.
    Json,
    /// One row per line, written as it is parsed.
    Ndjson,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Some(Command::Unparse(args)) => run_unparse(args),
        Some(Command::Parse(args)) => run_parse(&args),
        None => run_parse(&cli.parse),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn open(path: Option<&Path>) -> Result<Box<dyn Read>> {
    Ok(match path {
        Some(p) if p.as_os_str() != "-" => Box::new(BufReader::new(
            File::open(p).with_context(|| format!("opening {}", p.display()))?,
        )),
        _ => Box::new(io::stdin().lock()),
    })
}

fn parse_config(args: &ParseArgs) -> ParseConfig {
    let mut config = ParseConfig::default()
        .header(args.header)
        .dynamic_typing(args.dynamic_typing)
        .preview(args.preview);
    if let Some(d) = &args.delimiter {
        config = config.delimiter(d.clone());
    }
    if let Some(n) = args.newline {
        config = config.newline(n.newline());
    }
    if let Some(q) = args.quote_char {
        config = config.quote_char(q);
    }
    if let Some(e) = args.escape_char {
        config = config.escape_char(e);
    }
    if let Some(c) = &args.comments {
        config = config.comments(c.clone());
    }
    if let Some(s) = args.skip_empty_lines {
        config = config.skip_empty_lines(s.mode());
    }
    config
}

fn run_parse(args: &ParseArgs) -> Result<ExitCode> {
    let config = parse_config(args);
    let reader = open(args.input.as_deref())?;
    let mut out = BufWriter::new(io::stdout().lock());
    let errors = match args.format {
        Format::Json => {
            let mut input = String::new();
            { reader }
                .read_to_string(&mut input)
                .context("reading input")?;
            let output = papaparse::parse(&input, &config)?;
            serde_json::to_writer_pretty(&mut out, &output)?;
            writeln!(out)?;
            for e in &output.errors {
                eprintln!("warning: {e}");
            }
            output.errors.len()
        }
        Format::Ndjson => stream_ndjson(reader, &config, &mut out)?,
    };
    out.flush()?;
    Ok(if args.strict && errors > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    })
}

/// Writes each row as soon as it is parsed; returns the number of parse errors.
fn stream_ndjson(reader: impl Read, config: &ParseConfig, out: &mut impl Write) -> Result<usize> {
    let mut parser = ChunkParser::new(config)?;
    let (mut rows, mut errors) = (0usize, 0usize);
    let mut failure = None;
    let mut on_row = |step: papaparse::Step<'_>| {
        for e in &step.errors {
            // Row numbers in step mode restart per row (as in PapaParse), so count here.
            eprintln!("warning: {} (data row {rows})", e.message);
            errors += 1;
        }
        rows += 1;
        let written = serde_json::to_writer(&mut *out, &step.row)
            .map_err(io::Error::from)
            .and_then(|()| writeln!(out));
        match written {
            Ok(()) => Flow::Continue,
            Err(e) => {
                failure = Some(e);
                Flow::Abort
            }
        }
    };
    parser
        .read_from(reader, DEFAULT_CHUNK_SIZE, Sink::Rows(&mut on_row))
        .context("reading input")?;
    if let Some(e) = failure {
        return Err(e).context("writing output");
    }
    Ok(errors)
}

fn run_unparse(args: UnparseArgs) -> Result<ExitCode> {
    let json: Json =
        serde_json::from_reader(open(args.input.as_deref())?).context("input is not valid JSON")?;

    let mut config = UnparseConfig::default()
        .delimiter(args.delimiter)
        .newline(args.newline.newline().as_str())
        .quote_char(args.quote_char)
        .header(!args.no_header)
        .quotes(if args.quotes {
            Quotes::All
        } else {
            Quotes::Needed
        })
        .escape_formulae(if args.escape_formulae {
            EscapeFormulae::Default
        } else {
            EscapeFormulae::Off
        });
    if let Some(e) = args.escape_char {
        config = config.escape_char(e);
    }
    if let Some(c) = args.columns {
        config = config.columns(c);
    }
    if let Some(s) = args.skip_empty_lines {
        config = config.skip_empty_lines(s.mode());
    }

    // The input shapes `Papa.unparse` accepts.
    let csv = match &json {
        Json::Array(rows) => unparse(&rows.iter().map(JsonRow::new).collect::<Vec<_>>(), &config)?,
        Json::Object(o) if o.contains_key("data") || o.contains_key("fields") => {
            let fields: Option<Vec<&str>> = o
                .get("fields")
                .and_then(Json::as_array)
                .map(|f| f.iter().filter_map(Json::as_str).collect());
            let data = o.get("data").and_then(Json::as_array);
            let rows: Vec<JsonRow<'_>> = match data {
                // A single row given flat: `{ "data": ["a", "b"] }`.
                Some(d) if d.first().is_some_and(|x| !x.is_array() && !x.is_object()) => {
                    vec![JsonRow::List(d.iter().map(cell).collect())]
                }
                Some(d) => d.iter().map(JsonRow::new).collect(),
                None => Vec::new(),
            };
            unparse_table(fields.as_deref(), &rows, &config)?
        }
        _ => bail!("expected an array of rows or an object with `fields` / `data`"),
    };

    let mut out = BufWriter::new(io::stdout().lock());
    out.write_all(csv.as_bytes())?;
    writeln!(out)?;
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

/// A JSON row: an array of cells or an object keyed by column (key order kept).
enum JsonRow<'j> {
    List(Vec<Value<'j>>),
    Object(Vec<(&'j str, Value<'j>)>),
}

impl<'j> JsonRow<'j> {
    fn new(v: &'j Json) -> Self {
        match v {
            Json::Object(o) => {
                JsonRow::Object(o.iter().map(|(k, x)| (k.as_str(), cell(x))).collect())
            }
            Json::Array(a) => JsonRow::List(a.iter().map(cell).collect()),
            other => JsonRow::List(vec![cell(other)]),
        }
    }
}

/// A JSON value as `Papa.unparse` would print it.
fn cell(v: &Json) -> Value<'_> {
    match v {
        Json::String(s) => Value::from(s.as_str()),
        Json::Number(n) => n.as_f64().map_or(Value::Null, Value::Number),
        Json::Bool(b) => Value::Bool(*b),
        Json::Null => Value::Null,
        Json::Array(items) => Value::List(items.iter().map(cell).collect()),
        Json::Object(_) => Value::from("[object Object]"),
    }
}

impl UnparseRow for JsonRow<'_> {
    fn keys(&self) -> Option<Vec<&str>> {
        match self {
            JsonRow::List(_) => None,
            JsonRow::Object(o) => Some(o.iter().map(|(k, _)| *k).collect()),
        }
    }
    fn width(&self) -> usize {
        match self {
            JsonRow::List(v) => v.len(),
            JsonRow::Object(o) => o.len(),
        }
    }
    fn at(&self, index: usize) -> Option<&Value<'_>> {
        match self {
            JsonRow::List(v) => v.get(index),
            JsonRow::Object(_) => None,
        }
    }
    fn named(&self, name: &str) -> Option<&Value<'_>> {
        match self {
            JsonRow::List(_) => None,
            JsonRow::Object(o) => o.iter().find(|(k, _)| *k == name).map(|(_, v)| v),
        }
    }
}
