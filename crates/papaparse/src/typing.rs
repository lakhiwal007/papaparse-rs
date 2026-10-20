//! Dynamic typing (papaparse.js `parseDynamic`, `testFloat` and the `FLOAT` / `ISO_DATE` regexes),
//! implemented without a regex engine.

use std::borrow::Cow;

use crate::is_js_whitespace;
use crate::value::Value;

const MAX_FLOAT: f64 = 9_007_199_254_740_992.0; // 2^53

/// Converts a cell the way PapaParse's `dynamicTyping` does.
pub(crate) fn parse_dynamic(value: Cow<'_, str>) -> Value<'_> {
    match value.as_ref() {
        "true" | "TRUE" => Value::Bool(true),
        "false" | "FALSE" => Value::Bool(false),
        "" => Value::Null,
        s => {
            if let Some(n) = parse_float(s) {
                Value::Number(n)
            } else if is_iso_date(s) {
                Value::Date(value)
            } else {
                Value::String(value)
            }
        }
    }
}

/// `FLOAT.test(s) && |parseFloat(s)| < 2^53`, where
/// `FLOAT = /^\s*-?(\d+\.?|\.\d+|\d+\.\d+)([eE][-+]?\d+)?\s*$/`.
fn parse_float(s: &str) -> Option<f64> {
    let t = s.trim_matches(is_js_whitespace);
    let b = t.as_bytes();
    let mut i = usize::from(b.first() == Some(&b'-'));

    let int_digits = count_digits(&b[i..]);
    i += int_digits;
    let mut frac_digits = 0;
    if b.get(i) == Some(&b'.') {
        i += 1;
        frac_digits = count_digits(&b[i..]);
        i += frac_digits;
    }
    if int_digits == 0 && frac_digits == 0 {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp_digits = count_digits(&b[j.min(b.len())..]);
        if exp_digits == 0 {
            return None;
        }
        i = j + exp_digits;
    }
    if i != b.len() {
        return None;
    }

    let n: f64 = t.parse().ok()?;
    (n > -MAX_FLOAT && n < MAX_FLOAT).then_some(n)
}

fn count_digits(b: &[u8]) -> usize {
    b.iter().take_while(|c| c.is_ascii_digit()).count()
}

/// `YYYY-MM-DDTHH:MM[:SS[.fff]](Z|±HH:MM)` with PapaParse's digit-range checks.
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let in_range = |i: usize, max: u8| b.get(i).is_some_and(|c| (b'0'..=max).contains(c));
    let byte = |i: usize, c: u8| b.get(i) == Some(&c);

    // YYYY-MM-DDTHH:MM
    let head = (0..4).all(digit)
        && byte(4, b'-')
        && in_range(5, b'1')
        && digit(6)
        && byte(7, b'-')
        && in_range(8, b'3')
        && digit(9)
        && byte(10, b'T')
        && in_range(11, b'2')
        && digit(12)
        && byte(13, b':')
        && in_range(14, b'5')
        && digit(15);
    if !head {
        return false;
    }

    let mut i = 16;
    if byte(i, b':') {
        if !(in_range(i + 1, b'5') && digit(i + 2)) {
            return false;
        }
        i += 3;
        if byte(i, b'.') {
            let frac = count_digits(&b[i + 1..]);
            if frac == 0 {
                return false;
            }
            i += 1 + frac;
        }
    }

    match b.get(i) {
        Some(b'Z') => i + 1 == b.len(),
        Some(b'+' | b'-') => {
            in_range(i + 1, b'2')
                && digit(i + 2)
                && byte(i + 3, b':')
                && in_range(i + 4, b'5')
                && digit(i + 5)
                && i + 6 == b.len()
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats() {
        for ok in ["1", "-1", "1.", ".5", "1.5", " 2e10 ", "1E-3", "-0"] {
            assert!(parse_float(ok).is_some(), "{ok}");
        }
        for bad in [
            "",
            "-",
            ".",
            "1e",
            "1.2.3",
            "0x10",
            "1,000",
            "9007199254740993",
            "NaN",
            "+1",
        ] {
            assert!(parse_float(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn iso_dates() {
        for ok in [
            "2018-05-04T21:08:03.269Z",
            "2018-05-08T15:20:22.642+11:00",
            "2018-05-04T21:08Z",
            "2018-05-04T21:08:03-05:00",
        ] {
            assert!(is_iso_date(ok), "{ok}");
        }
        for bad in [
            "2018-05-04",
            "2018-05-04T21:08:03",
            "2018-25-04T21:08Z",
            "2018-05-04T21:08:03.Z",
        ] {
            assert!(!is_iso_date(bad), "{bad}");
        }
    }
}
