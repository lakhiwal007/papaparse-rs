//! Chunked and streaming input (papaparse.js `ChunkStreamer`, `ReadableStreamStreamer`).

use std::io::{self, Read};

use crate::config::ParseConfig;
use crate::core::{Flow, Parser};
use crate::error::ConfigError;
use crate::handle::{Hooks, ParseOutput, ParserHandle, Step, parsed_slice};

/// Default chunk size of [`parse_reader`] and [`parse_reader_rows`], in bytes
/// (JS: `Papa.LocalChunkSize`).
pub const DEFAULT_CHUNK_SIZE: usize = 10 * 1024 * 1024;

/// Where a [`ChunkParser`] delivers results.
pub enum Sink<'s> {
    /// The rows completed by each chunk, at once (JS: `config.chunk`).
    Chunks(&'s mut dyn FnMut(ParseOutput<'_>) -> Flow),
    /// One row at a time (JS: `config.step`).
    Rows(&'s mut dyn FnMut(Step<'_>) -> Flow),
}

impl Sink<'_> {
    fn reborrow(&mut self) -> Sink<'_> {
        match self {
            Sink::Chunks(f) => Sink::Chunks(&mut **f),
            Sink::Rows(f) => Sink::Rows(&mut **f),
        }
    }
}

impl std::fmt::Debug for Sink<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Sink::Chunks(_) => "Chunks(<fn>)",
            Sink::Rows(_) => "Rows(<fn>)",
        })
    }
}

/// Parses input that arrives in pieces, such as a file read in blocks or a network stream.
/// JS: `Papa.parse` with `chunk` or `step` callbacks.
///
/// Each piece is parsed as soon as it arrives. A row cut off by the end of a piece is
/// kept and completed by the next one, so memory use is bounded by the piece size
/// (plus the longest row), not the input size.
///
/// ```
/// use papaparse::{ChunkParser, Flow, ParseConfig};
///
/// let mut parser = ChunkParser::new(&ParseConfig::default().delimiter(","))?;
/// let mut rows = Vec::new();
/// for piece in ["a,b\nc,", "d\ne,f"] {
///     parser.feed(piece, |out| { rows.extend(out.data.into_iter().map(|r| r.into_owned())); Flow::Continue })?;
/// }
/// parser.finish(|out| { rows.extend(out.data.into_iter().map(|r| r.into_owned())); Flow::Continue })?;
/// assert_eq!(rows.len(), 3);
/// # Ok::<(), papaparse::ConfigError>(())
/// ```
///
/// As in PapaParse, the delimiter and line ending are detected from the first piece,
/// `skip_first_n_lines` and the byte order mark apply to the first piece, and
/// `meta.cursor` / `error.index` are offsets into the whole stream.
pub struct ChunkParser {
    handle: ParserHandle,
    /// The unfinished last row of the previous piece (JS: `_partialLine`).
    partial: String,
    /// Stream offset of `partial` (JS: `_baseIndex`).
    base_index: usize,
    /// Rows delivered in chunks so far (JS: `_rowCount`).
    row_count: usize,
    first_chunk: bool,
    /// The text parsed so far ends with a row terminator.
    after_newline: bool,
    phase: Phase,
}

/// Where a [`ChunkParser`] is in its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// More pieces may come.
    Open,
    /// The final piece was pushed; a paused parse may still have text to go.
    Ended,
    /// Nothing more will be parsed.
    Done,
}

impl std::fmt::Debug for ChunkParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChunkParser")
            .field("config", self.handle.config())
            .field("buffered", &self.partial.len())
            .field("base_index", &self.base_index)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl ChunkParser {
    /// Validates `config` and creates a parser.
    pub fn new(config: &ParseConfig) -> Result<Self, ConfigError> {
        // Report a bad fixed configuration now rather than on the first piece.
        Parser::new(config)?;
        Ok(Self {
            handle: ParserHandle::new(config.clone()),
            partial: String::new(),
            base_index: 0,
            row_count: 0,
            first_chunk: true,
            after_newline: false,
            phase: Phase::Open,
        })
    }

    /// Whether parsing has stopped: the input ended, a callback returned
    /// [`Flow::Abort`], or `preview` rows were delivered. Later pieces are ignored.
    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }

    /// Parses the next piece and passes the rows it completes to `on_chunk`.
    ///
    /// `Err` only for an invalid configuration that could not be checked up front
    /// (a `delimiter_fn` returning the comment marker).
    pub fn feed(
        &mut self,
        piece: &str,
        mut on_chunk: impl FnMut(ParseOutput<'_>) -> Flow,
    ) -> Result<(), ConfigError> {
        self.push(piece, false, Hooks::default(), Sink::Chunks(&mut on_chunk))
    }

    /// Ends the input: parses whatever row is still buffered.
    pub fn finish(
        &mut self,
        mut on_chunk: impl FnMut(ParseOutput<'_>) -> Flow,
    ) -> Result<(), ConfigError> {
        self.push("", true, Hooks::default(), Sink::Chunks(&mut on_chunk))
    }

    /// Parses the next piece and passes each row it completes to `on_row`.
    pub fn feed_rows(
        &mut self,
        piece: &str,
        mut on_row: impl FnMut(Step<'_>) -> Flow,
    ) -> Result<(), ConfigError> {
        self.push(piece, false, Hooks::default(), Sink::Rows(&mut on_row))
    }

    /// Ends the input in row mode: parses whatever row is still buffered.
    pub fn finish_rows(
        &mut self,
        mut on_row: impl FnMut(Step<'_>) -> Flow,
    ) -> Result<(), ConfigError> {
        self.push("", true, Hooks::default(), Sink::Rows(&mut on_row))
    }

    /// The general form of [`feed`](Self::feed) and [`finish`](Self::finish): `last`
    /// marks the final piece, and `hooks` replace the matching config callbacks for
    /// this piece. JS: `ChunkStreamer.parseChunk`.
    ///
    /// A row callback returning [`Flow::Pause`] stops parsing after that row; the rest
    /// of the piece is kept and parsed by the next call (pass `""` to just resume).
    /// A chunk callback's `Pause` has no effect: you decide when to push the next piece.
    pub fn push(
        &mut self,
        piece: &str,
        last: bool,
        hooks: Hooks<'_>,
        mut sink: Sink<'_>,
    ) -> Result<(), ConfigError> {
        let last = match self.phase {
            Phase::Done => return Ok(()),
            Phase::Ended => true,
            Phase::Open => last,
        };
        if last {
            self.phase = Phase::Ended;
        }
        let piece = if std::mem::take(&mut self.first_chunk) {
            parsed_slice(piece, self.handle.config())
        } else {
            piece
        };

        // Rejoin the row the previous piece cut in two.
        let joined;
        let input = if self.partial.is_empty() {
            piece
        } else {
            let mut buf = std::mem::take(&mut self.partial);
            buf.push_str(piece);
            joined = buf;
            joined.as_str()
        };
        let base = self.base_index;
        // Input that ends right after a row terminator has a final empty row, as in `parse`.
        let empty_row = self.after_newline;

        let mut flow = Flow::Continue;
        let out = match sink.reborrow() {
            Sink::Chunks(_) => self
                .handle
                .parse_impl(input, base, !last, hooks, None, empty_row)?,
            Sink::Rows(on_row) => {
                let mut step = |s: Step<'_>| {
                    flow = on_row(s);
                    flow
                };
                self.handle
                    .parse_impl(input, base, !last, hooks, Some(&mut step), empty_row)?
            }
        };
        if flow == Flow::Abort || self.handle.preview_reached() {
            self.phase = Phase::Done;
            return Ok(());
        }
        // `meta.cursor` is just past the last row parsed: keep the rest for later.
        if !last || flow == Flow::Pause {
            let consumed = out.meta.cursor - base;
            self.partial.push_str(&input[consumed..]);
            self.base_index = out.meta.cursor;
            if consumed > 0 {
                let newline = self.handle.config().newline.unwrap_or_default();
                self.after_newline = input[..consumed].ends_with(newline.as_str());
            }
        }
        if flow == Flow::Pause {
            return Ok(());
        }

        self.row_count += out.data.len();
        let preview = self.handle.config().preview;
        if last || (preview > 0 && self.row_count >= preview) {
            self.phase = Phase::Done;
        }

        if let Sink::Chunks(on_chunk) = sink {
            if on_chunk(out) == Flow::Abort {
                self.phase = Phase::Done;
            }
        }
        Ok(())
    }

    /// Reads `reader` to the end (or until parsing stops) in pieces of `chunk_size`
    /// bytes. Invalid UTF-8 is replaced with U+FFFD, as a JS text decoder does.
    ///
    /// # Errors
    /// I/O errors from `reader`; a [`ConfigError`] (see [`feed`](Self::feed)) is
    /// returned as an [`io::ErrorKind::InvalidInput`] error wrapping it.
    pub fn read_from<R: Read>(
        &mut self,
        mut reader: R,
        chunk_size: usize,
        mut sink: Sink<'_>,
    ) -> io::Result<()> {
        let chunk_size = chunk_size.max(4);
        let mut carry = Vec::new();
        while !self.is_done() {
            let mut bytes = std::mem::take(&mut carry);
            let wanted = chunk_size.saturating_sub(bytes.len()).max(1);
            let read = reader
                .by_ref()
                .take(wanted as u64)
                .read_to_end(&mut bytes)?;
            let eof = read == 0;
            let text = decode_utf8(bytes, &mut carry, eof);
            self.push(&text, eof, Hooks::default(), sink.reborrow())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        }
        Ok(())
    }
}

/// Parses everything `reader` yields, passing the rows completed by each
/// [`DEFAULT_CHUNK_SIZE`] piece to `on_chunk`. Return [`Flow::Abort`] to stop early.
///
/// ```
/// use papaparse::{Flow, ParseConfig, parse_reader};
///
/// let file = std::io::Cursor::new("a,b\n1,2\n3,4");
/// let mut count = 0;
/// parse_reader(file, &ParseConfig::default().header(true), |out| {
///     count += out.data.len();
///     Flow::Continue
/// })?;
/// assert_eq!(count, 2);
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// # Errors
/// See [`ChunkParser::read_from`]; an invalid `config` is an
/// [`io::ErrorKind::InvalidInput`] error.
pub fn parse_reader<R: Read>(
    reader: R,
    config: &ParseConfig,
    mut on_chunk: impl FnMut(ParseOutput<'_>) -> Flow,
) -> io::Result<()> {
    ChunkParser::new(config)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
        .read_from(reader, DEFAULT_CHUNK_SIZE, Sink::Chunks(&mut on_chunk))
}

/// Parses everything `reader` yields, one row at a time. JS: `Papa.parse(stream, { step })`.
///
/// # Errors
/// As [`parse_reader`].
pub fn parse_reader_rows<R: Read>(
    reader: R,
    config: &ParseConfig,
    mut on_row: impl FnMut(Step<'_>) -> Flow,
) -> io::Result<()> {
    ChunkParser::new(config)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
        .read_from(reader, DEFAULT_CHUNK_SIZE, Sink::Rows(&mut on_row))
}

/// Decodes `bytes` as UTF-8, replacing invalid sequences with U+FFFD. Unless `eof`, an
/// unfinished sequence at the end is moved to `carry` for the next read.
fn decode_utf8(mut bytes: Vec<u8>, carry: &mut Vec<u8>, eof: bool) -> String {
    if !eof {
        let keep = bytes.len() - incomplete_tail(&bytes);
        carry.extend_from_slice(&bytes[keep..]);
        bytes.truncate(keep);
    }
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// Length of a UTF-8 sequence cut off at the end of `bytes` (0 if none).
fn incomplete_tail(bytes: &[u8]) -> usize {
    for back in 1..=bytes.len().min(3) {
        let byte = bytes[bytes.len() - back];
        if byte & 0xC0 != 0x80 {
            let len = match byte {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                _ => 1, // ASCII, or an invalid byte the decoder replaces anyway
            };
            return if len > back { back } else { 0 };
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Row, Value, parse};

    fn chunked(input: &str, config: &ParseConfig, size: usize) -> Vec<Row<'static>> {
        let mut rows = Vec::new();
        let mut parser = ChunkParser::new(config).unwrap();
        parser
            .read_from(
                io::Cursor::new(input),
                size,
                Sink::Chunks(&mut |out| {
                    rows.extend(out.data.into_iter().map(Row::into_owned));
                    Flow::Continue
                }),
            )
            .unwrap();
        rows
    }

    #[test]
    fn chunked_equals_whole() {
        let input = "name,note\n\"Ada\",\"multi\nline, with \"\"quotes\"\"\"\nBob,é😀ü\n\nCy,last";
        for config in [
            ParseConfig::default().delimiter(","),
            ParseConfig::default().header(true).dynamic_typing(true),
            ParseConfig::default().skip_empty_lines(crate::SkipEmptyLines::Yes),
        ] {
            let whole: Vec<_> = parse(input, &config).unwrap().data;
            for size in 4..=input.len() + 1 {
                assert_eq!(chunked(input, &config, size), whole, "size {size}");
            }
        }
    }

    #[test]
    fn rows_mode_matches_and_aborts() {
        let config = ParseConfig::default().header(true);
        let mut seen = Vec::new();
        let mut parser = ChunkParser::new(&config).unwrap();
        for piece in ["a,b\n1,", "2\n3,4\n5,6"] {
            parser
                .feed_rows(piece, |s| {
                    let Row::Record(r) = s.row else { panic!() };
                    seen.push(r["a"].clone().into_owned());
                    if seen.len() == 2 {
                        Flow::Abort
                    } else {
                        Flow::Continue
                    }
                })
                .unwrap();
        }
        assert!(parser.is_done());
        assert_eq!(seen, [Value::from("1"), Value::from("3")]);
    }

    #[test]
    fn pause_keeps_the_rest_for_the_next_push() {
        fn take(parser: &mut ChunkParser, seen: &mut Vec<Row<'static>>, piece: &str, last: bool) {
            let mut step = |s: Step<'_>| {
                seen.push(s.row.into_owned());
                Flow::Pause
            };
            parser
                .push(piece, last, Hooks::default(), Sink::Rows(&mut step))
                .unwrap();
        }
        let mut parser = ChunkParser::new(&ParseConfig::default().delimiter(",")).unwrap();
        let mut seen = Vec::new();
        take(&mut parser, &mut seen, "a\nb\nc", true);
        assert_eq!(seen.len(), 1);
        take(&mut parser, &mut seen, "", false);
        take(&mut parser, &mut seen, "", false);
        assert!(!parser.is_done());
        take(&mut parser, &mut seen, "", false);
        assert!(parser.is_done());
        let expected: Vec<Row<'_>> = ["a", "b", "c"]
            .into_iter()
            .map(|v| Row::Fields(vec![Value::from(v)]))
            .collect();
        assert_eq!(seen, expected);
    }

    #[test]
    fn cursor_is_a_stream_offset() {
        let mut cursors = Vec::new();
        let mut parser = ChunkParser::new(&ParseConfig::default().delimiter(",")).unwrap();
        for piece in ["ab\ncd", "\nef\n", "gh"] {
            parser
                .feed_rows(piece, |s| {
                    cursors.push(s.meta.cursor);
                    Flow::Continue
                })
                .unwrap();
        }
        parser
            .finish_rows(|s| {
                cursors.push(s.meta.cursor);
                Flow::Continue
            })
            .unwrap();
        assert_eq!(cursors, [3, 6, 9, 11]);
    }

    #[test]
    fn utf8_split_across_reads() {
        let mut carry = Vec::new();
        let bytes = "é😀".as_bytes();
        let a = decode_utf8(bytes[..3].to_vec(), &mut carry, false);
        let mut rest = std::mem::take(&mut carry);
        rest.extend_from_slice(&bytes[3..]);
        let b = decode_utf8(rest, &mut carry, true);
        assert_eq!((a.as_str(), b.as_str()), ("é", "😀"));
        assert_eq!(
            decode_utf8(vec![b'a', 0xFF, b'b'], &mut carry, false),
            "a\u{FFFD}b"
        );
        assert_eq!(
            decode_utf8(vec![0xE2, 0x82], &mut Vec::new(), true),
            "\u{FFFD}"
        );
    }
}
