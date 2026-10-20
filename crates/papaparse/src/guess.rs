//! Delimiter and line-ending detection (papaparse.js `guessDelimiter`, `guessLineEndings`).

use crate::config::{Newline, ParseConfig, SkipEmptyLines};
use crate::core::Parser;
use crate::search::Finder;
use crate::{RECORD_SEP, UNIT_SEP};

/// Delimiters tried when [`ParseConfig::delimiters_to_guess`] is empty.
pub(crate) const DEFAULT_GUESSES: [&str; 6] = [",", "\t", "|", ";", RECORD_SEP, UNIT_SEP];

/// Detects the line terminator from the first megabyte of `input`, ignoring quoted text.
pub fn guess_line_endings(input: &str, quote: char) -> Newline {
    let mut end = input.len().min(1024 * 1024);
    while !input.is_char_boundary(end) {
        end -= 1;
    }
    let unquoted = strip_quoted(&input[..end], quote);

    let r: Vec<&str> = unquoted.split('\r').collect();
    let n_first_len = unquoted.split('\n').next().map(str::len);
    let n_appears_first = unquoted.contains('\n') && n_first_len < Some(r[0].len());

    if r.len() == 1 || n_appears_first {
        return Newline::Lf;
    }
    let with_n = r.iter().filter(|s| s.starts_with('\n')).count();
    if with_n * 2 >= r.len() {
        Newline::CrLf
    } else {
        Newline::Cr
    }
}

/// Removes every `quote ... quote` span (JS: `replace(/"([^]*?)"/gm, '')`).
fn strip_quoted(s: &str, quote: char) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find(quote) {
        let after_open = open + quote.len_utf8();
        let Some(close) = rest[after_open..].find(quote) else {
            break;
        };
        out.push_str(&rest[..open]);
        rest = &rest[after_open + close + quote.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// Picks the delimiter whose first ten rows have the most consistent field count.
/// Returns `None` when no candidate yields at least two fields per row on average.
pub fn guess_delimiter(input: &str, newline: Newline, config: &ParseConfig) -> Option<String> {
    let custom: Vec<&str> = config
        .delimiters_to_guess
        .iter()
        .map(String::as_str)
        .collect();
    let candidates: &[&str] = if custom.is_empty() {
        &DEFAULT_GUESSES
    } else {
        &custom
    };

    // JS builds each preview parser without `quoteChar`/`fastMode`, so every candidate makes the
    // same "does the input contain `\"`?" fast-path decision. Decide once instead of per candidate.
    let fast_mode = Finder::new(input, "\"").find(0).is_none();

    let mut best: Option<(&str, usize, f64)> = None; // (delimiter, delta, avg field count)
    for &delim in candidates {
        let mut preview_config = ParseConfig::default()
            .delimiter(delim)
            .newline(newline)
            .preview(10)
            .fast_mode(fast_mode);
        preview_config.comments.clone_from(&config.comments);
        // A comment marker equal to this candidate can't be used with it; skip the candidate.
        let Ok(mut parser) = Parser::new(&preview_config) else {
            continue;
        };
        let preview = parser.parse(input);

        let mut delta = 0usize;
        let mut avg = 0f64;
        let mut empty_lines = 0usize;
        let mut prev_count: Option<usize> = None;

        for row in &preview.data {
            if config.skip_empty_lines != SkipEmptyLines::No
                && is_empty_line(row, config.skip_empty_lines)
            {
                empty_lines += 1;
                continue;
            }
            let count = row.len();
            avg += count as f64;
            match prev_count {
                None => prev_count = Some(count),
                Some(prev) if count > 0 => {
                    delta += count.abs_diff(prev);
                    prev_count = Some(count);
                }
                Some(_) => {}
            }
        }
        if !preview.data.is_empty() {
            avg /= (preview.data.len() - empty_lines) as f64;
        }

        // Lowest delta wins; average field count only breaks ties.
        let better = match best {
            None => true,
            Some((_, best_delta, best_avg)) => {
                delta < best_delta || (delta == best_delta && avg > best_avg)
            }
        };
        if avg > 1.99 && better {
            best = Some((delim, delta, avg));
        }
    }
    best.map(|(d, _, _)| d.to_owned())
}

pub(crate) fn is_empty_line<S: AsRef<str>>(row: &[S], mode: SkipEmptyLines) -> bool {
    match mode {
        SkipEmptyLines::Greedy => row
            .iter()
            .all(|f| f.as_ref().trim_matches(crate::is_js_whitespace).is_empty()),
        _ => row.len() == 1 && row[0].as_ref().is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings() {
        assert_eq!(guess_line_endings("a,b\r\nc,d\r\n", '"'), Newline::CrLf);
        assert_eq!(guess_line_endings("a,b\rc,d\r", '"'), Newline::Cr);
        assert_eq!(guess_line_endings("a,\"x\r\ny\"\nc,d", '"'), Newline::Lf);
    }

    #[test]
    fn delimiter() {
        let cfg = ParseConfig::default();
        assert_eq!(
            guess_delimiter("a;b;c\n1;2;3", Newline::Lf, &cfg).as_deref(),
            Some(";")
        );
        assert_eq!(guess_delimiter("abc\ndef", Newline::Lf, &cfg), None);
    }
}
