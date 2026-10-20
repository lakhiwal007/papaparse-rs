//! Shared layer of the JavaScript bindings (`papaparse-wasm`, `papaparse-node`).
//!
//! Bindings read the JS config into [`Options`], call [`run`], and hand the resulting
//! [`Tape`] and info JSON to `js/decode.cjs`, which rebuilds PapaParse's result object.
//! With callbacks or streamed input, `js/papa.cjs` drives a [`Session`] chunk by chunk;
//! `Papa.unparse` goes through [`unparse`]. Keeping this in one crate guarantees both
//! packages produce identical results.

#![warn(missing_docs)]

mod info;
mod options;
mod session;
mod tape;
mod unparse;

pub use papaparse;

pub use crate::info::info;
pub use crate::options::{BoolOrString, Options, Typing};
pub use crate::session::{BatchFlow, ChunkResult, Session, StepBatch};
pub use crate::tape::{Tape, Utf16Offsets, tag};
pub use crate::unparse::{UnparseCells, UnparseOptions, kind, unparse};

/// Parses `input` with `options` and returns the cell tape plus the
/// `{"errors":[...],"meta":{...}}` JSON. `Err` carries a message for a JS `Error`.
pub fn run(input: &str, options: Options) -> Result<(Tape, String), String> {
    let config = options.into_config()?;
    let output = papaparse::parse(input, &config).map_err(|e| e.to_string())?;
    let base = input.len() - papaparse::parsed_slice(input, &config).len();
    Ok((Tape::encode(input, &output), info(input, base, &output)))
}
