use std::fmt;

/// An invalid configuration. JS throws an `Error` in these cases.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    /// `comments` equals the delimiter.
    CommentSameAsDelimiter,
    /// [`crate::UnparseConfig::columns`] is set but empty.
    EmptyColumns,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::CommentSameAsDelimiter => {
                f.write_str("Comment character same as delimiter")
            }
            ConfigError::EmptyColumns => f.write_str("Option columns is empty"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Category of a recoverable parse problem. JS: `error.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ErrorType {
    /// Malformed or unterminated quoted field.
    Quotes,
    /// Delimiter could not be detected.
    Delimiter,
    /// A row's field count differs from the header's.
    FieldMismatch,
}

/// Specific parse problem. JS: `error.code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ErrorCode {
    /// A quoted field is never closed.
    MissingQuotes,
    /// A closing quote is followed by something other than a delimiter or newline.
    InvalidQuotes,
    /// No candidate delimiter fits; `,` was used.
    UndetectableDelimiter,
    /// More fields than header columns.
    TooManyFields,
    /// Fewer fields than header columns.
    TooFewFields,
}

/// A recoverable problem found while parsing. Parsing continues past these;
/// they are collected in the output's `errors`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ParseError {
    #[cfg_attr(feature = "serde", serde(rename = "type"))]
    /// Category (`type` in JS).
    pub kind: ErrorType,
    /// Specific problem.
    pub code: ErrorCode,
    /// Human-readable description, identical to PapaParse's.
    pub message: String,
    /// Row the error belongs to, when known.
    #[cfg_attr(
        feature = "serde",
        serde(skip_serializing_if = "Option::is_none", default)
    )]
    pub row: Option<usize>,
    /// Byte offset into the input, when known.
    #[cfg_attr(
        feature = "serde",
        serde(skip_serializing_if = "Option::is_none", default)
    )]
    pub index: Option<usize>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(row) = self.row {
            write!(f, " (row {row})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseError {}
