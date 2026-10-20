//! Memoizing forward substring search.
//!
//! PapaParse re-runs `indexOf(delimiter|newline|quote, cursor)` after almost every field.
//! When the needle no longer occurs, each of those calls rescans to the end of the input,
//! which makes some inputs quadratic (for example a single-column file of quoted rows).
//! [`Finder`] remembers its last answer, so every byte is scanned at most once per needle
//! and the parser stays linear on all inputs.

use memchr::memmem;

pub(crate) struct Finder<'h> {
    haystack: &'h [u8],
    kind: Kind,
    /// Last search: `(from, result)`. Any later `from'` with `from <= from' <= result`
    /// has the same answer; `None` means no match anywhere at or after `from`.
    memo: Option<(usize, Option<usize>)>,
}

enum Kind {
    Byte(u8),
    Bytes(memmem::Finder<'static>),
}

impl<'h> Finder<'h> {
    pub(crate) fn new(haystack: &'h str, needle: &str) -> Self {
        debug_assert_ne!(needle, "", "empty needles would match everywhere");
        let kind = match needle.as_bytes() {
            [b] => Kind::Byte(*b),
            bytes => Kind::Bytes(memmem::Finder::new(bytes).into_owned()),
        };
        Self {
            haystack: haystack.as_bytes(),
            kind,
            memo: None,
        }
    }

    pub(crate) fn needle_len(&self) -> usize {
        match &self.kind {
            Kind::Byte(_) => 1,
            Kind::Bytes(f) => f.needle().len(),
        }
    }

    /// Byte offset of the first occurrence at or after `from` (JS `indexOf(needle, from)`).
    pub(crate) fn find(&mut self, from: usize) -> Option<usize> {
        if let Some((memo_from, result)) = self.memo {
            if from >= memo_from {
                match result {
                    None => return None,
                    Some(pos) if from <= pos => return Some(pos),
                    Some(_) => {}
                }
            }
        }
        let rest = self.haystack.get(from..)?;
        let result = match &self.kind {
            Kind::Byte(b) => memchr::memchr(*b, rest),
            Kind::Bytes(f) => f.find(rest),
        }
        .map(|i| i + from);
        self.memo = Some((from, result));
        result
    }
}

/// Splits `s` on `sep` like `str::split`, using a SIMD searcher.
pub(crate) fn split<'s>(s: &'s str, sep: &str) -> impl Iterator<Item = &'s str> {
    let mut finder = Finder::new(s, sep);
    let mut start = Some(0);
    std::iter::from_fn(move || {
        let from = start?;
        if let Some(pos) = finder.find(from) {
            start = Some(pos + sep.len());
            Some(&s[from..pos])
        } else {
            start = None;
            Some(&s[from..])
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_str_find() {
        let hay = "a,b,,c\r\nd,\"é\"";
        for needle in [",", "\r\n", "\"", "é", "x"] {
            let mut f = Finder::new(hay, needle);
            for from in 0..=hay.len() {
                if hay.is_char_boundary(from) {
                    assert_eq!(
                        f.find(from),
                        hay[from..].find(needle).map(|i| i + from),
                        "{needle:?} @ {from}"
                    );
                }
            }
        }
    }

    #[test]
    fn memo_is_safe_for_backward_queries() {
        let mut f = Finder::new("a,b,c", ",");
        assert_eq!(f.find(4), None);
        assert_eq!(f.find(0), Some(1));
        assert_eq!(f.find(2), Some(3));
    }

    #[test]
    fn split_matches_std() {
        for (s, sep) in [
            ("a,b,,c", ","),
            ("", ","),
            (",", ","),
            ("a\r\nb\r\n", "\r\n"),
            ("abc", "|"),
        ] {
            assert_eq!(
                split(s, sep).collect::<Vec<_>>(),
                s.split(sep).collect::<Vec<_>>()
            );
        }
    }
}
