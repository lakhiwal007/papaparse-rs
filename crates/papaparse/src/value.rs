use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Index;
use std::sync::Arc;

/// A parsed cell. Without dynamic typing every cell is [`Value::String`].
///
/// Text borrows from the input when it can (`'a`); see [`Value::into_owned`].
#[derive(Debug, Clone, PartialEq)]
pub enum Value<'a> {
    /// Text, as written in the input (after `transform`).
    String(Cow<'a, str>),
    /// A number recognised by dynamic typing.
    Number(f64),
    /// `true`/`TRUE`/`false`/`FALSE` under dynamic typing.
    Bool(bool),
    /// An empty cell under dynamic typing.
    Null,
    /// An ISO-8601 timestamp recognised by dynamic typing, kept as written.
    Date(Cow<'a, str>),
    /// Cells beyond the header width, collected under `__parsed_extra`.
    List(Vec<Value<'a>>),
}

impl Value<'_> {
    /// Copies borrowed text so the value no longer borrows the input.
    pub fn into_owned(self) -> Value<'static> {
        match self {
            Value::String(s) => Value::String(Cow::Owned(s.into_owned())),
            Value::Date(s) => Value::Date(Cow::Owned(s.into_owned())),
            Value::Number(n) => Value::Number(n),
            Value::Bool(b) => Value::Bool(b),
            Value::Null => Value::Null,
            Value::List(items) => Value::List(items.into_iter().map(Value::into_owned).collect()),
        }
    }

    /// The text of a [`Value::String`] or [`Value::Date`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) | Value::Date(s) => Some(s),
            _ => None,
        }
    }

    /// The number of a [`Value::Number`].
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
}

/// Serializes like `JSON.stringify`: integral numbers without a fraction (`36`, not `36.0`).
#[cfg(feature = "serde")]
impl serde::Serialize for Value<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::String(s) | Value::Date(s) => serializer.serialize_str(s),
            Value::Number(n) => match as_safe_integer(*n) {
                Some(i) => serializer.serialize_i64(i),
                None => serializer.serialize_f64(*n),
            },
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Null => serializer.serialize_unit(),
            Value::List(items) => serializer.collect_seq(items),
        }
    }
}

/// `n` as an integer if it has no fractional part and is exactly representable.
pub fn as_safe_integer(n: f64) -> Option<i64> {
    const SAFE: f64 = 9_007_199_254_740_992.0; // 2^53
    #[allow(clippy::cast_possible_truncation)]
    (n.fract() == 0.0 && n.abs() < SAFE).then_some(n as i64)
}

impl<'a> From<&'a str> for Value<'a> {
    fn from(s: &'a str) -> Self {
        Value::String(Cow::Borrowed(s))
    }
}

impl From<String> for Value<'_> {
    fn from(s: String) -> Self {
        Value::String(Cow::Owned(s))
    }
}

/// One output row: a list of cells, or a [`Record`] keyed by header when `header` is set.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Row<'a> {
    /// A row parsed without a header.
    Fields(Vec<Value<'a>>),
    /// A row parsed with a header.
    Record(Record<'a>),
}

impl Row<'_> {
    /// Copies borrowed text so the row no longer borrows the input.
    pub fn into_owned(self) -> Row<'static> {
        match self {
            Row::Fields(values) => Row::Fields(values.into_iter().map(Value::into_owned).collect()),
            Row::Record(r) => Row::Record(r.into_owned()),
        }
    }
}

/// Header shared by every [`Record`] of a parse run.
#[derive(Debug, PartialEq)]
pub(crate) struct Header {
    names: Vec<String>,
    /// Name -> column. On duplicate names the last column wins, like a JS object.
    index: HashMap<String, usize>,
    /// No name occurs twice, so a name at position `i` is column `i`.
    unique: bool,
}

impl Header {
    pub(crate) fn new(names: Vec<String>) -> Arc<Self> {
        let index: HashMap<String, usize> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i))
            .collect();
        let unique = index.len() == names.len();
        Arc::new(Self {
            names,
            index,
            unique,
        })
    }

    pub(crate) fn names(&self) -> &[String] {
        &self.names
    }
}

/// Key for fields beyond the header width.
pub const PARSED_EXTRA: &str = "__parsed_extra";

/// A row keyed by header name. JS: the object produced with `header: true`.
///
/// Records of one parse share their header, so a record costs one `Vec` of values;
/// header names are not copied per row.
#[derive(Debug, Clone)]
pub struct Record<'a> {
    header: Arc<Header>,
    /// Values by column; shorter than the header when the row has too few fields.
    values: Vec<Value<'a>>,
    /// Fields beyond the header width (`__parsed_extra`), as a [`Value::List`].
    extra: Option<Value<'a>>,
}

impl<'a> Record<'a> {
    pub(crate) fn new(header: Arc<Header>, values: Vec<Value<'a>>, extra: Vec<Value<'a>>) -> Self {
        let extra = (!extra.is_empty()).then_some(Value::List(extra));
        Self {
            header,
            values,
            extra,
        }
    }

    /// Copies borrowed text so the record no longer borrows the input.
    pub fn into_owned(self) -> Record<'static> {
        Record {
            header: self.header,
            values: self.values.into_iter().map(Value::into_owned).collect(),
            extra: self.extra.map(Value::into_owned),
        }
    }

    /// Values in column order, without header names (cheaper than [`Record::iter`]).
    /// Fields beyond the header width follow, flattened.
    pub fn values(&self) -> impl Iterator<Item = &Value<'a>> {
        let extra = match &self.extra {
            Some(Value::List(items)) => items.as_slice(),
            _ => &[],
        };
        self.values.iter().chain(extra)
    }

    /// The header names shared by all records of this parse.
    pub fn header(&self) -> &[String] {
        &self.header.names
    }

    /// The value for column `name`, or `None` if the row has no such field.
    pub fn get(&self, name: &str) -> Option<&Value<'a>> {
        if name == PARSED_EXTRA && self.extra.is_some() {
            return self.extra.as_ref();
        }
        self.header
            .index
            .get(name)
            .and_then(|&i| self.values.get(i))
    }

    /// [`Record::get`] for a caller that expects `name` at column `col`: answered by
    /// position, without hashing, when that guess is right.
    pub(crate) fn get_at(&self, name: &str, col: usize) -> Option<&Value<'a>> {
        if self.header.unique && self.header.names.get(col).is_some_and(|n| n == name) {
            return self.values.get(col);
        }
        self.get(name)
    }

    /// `(name, value)` pairs in column order, followed by `__parsed_extra` if present.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value<'a>)> {
        let header = &self.header;
        self.values
            .iter()
            .enumerate()
            // Skip earlier columns shadowed by a later duplicate name, like a JS object.
            .filter(move |&(i, _)| header.index.get(&header.names[i]) == Some(&i))
            .map(move |(i, v)| (header.names[i].as_str(), v))
            .chain(self.extra.as_ref().map(|v| (PARSED_EXTRA, v)))
    }

    /// Number of fields present.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Whether the record has no fields.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty() && self.extra.is_none()
    }
}

impl<'a> Index<&str> for Record<'a> {
    type Output = Value<'a>;

    /// # Panics
    /// If the record has no field `name`; use [`Record::get`] to avoid this.
    fn index(&self, name: &str) -> &Value<'a> {
        self.get(name)
            .unwrap_or_else(|| panic!("record has no field {name:?}"))
    }
}

impl PartialEq for Record<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Record<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.iter())
    }
}

/// Information about a parse run. JS: `results.meta`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Meta {
    /// Delimiter that was used (detected or configured).
    pub delimiter: String,
    /// Line terminator that was used (detected or configured).
    pub linebreak: String,
    /// Whether a step callback aborted parsing.
    pub aborted: bool,
    /// Whether parsing stopped early because of `preview`.
    pub truncated: bool,
    /// Byte offset just past the last fully parsed row.
    pub cursor: usize,
    /// Duplicate headers that were renamed, as `(new name, original)` pairs.
    #[cfg_attr(feature = "serde", serde(serialize_with = "pairs_as_map"))]
    pub renamed_headers: Option<Vec<(String, String)>>,
    /// Header row, when `header` is set.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub fields: Option<Vec<String>>,
}

#[cfg(feature = "serde")]
#[allow(clippy::ref_option)] // signature dictated by serde's `serialize_with`
fn pairs_as_map<S: serde::Serializer>(
    pairs: &Option<Vec<(String, String)>>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match pairs {
        Some(p) => s.collect_map(p.iter().map(|(k, v)| (k, v))),
        None => s.serialize_none(),
    }
}
