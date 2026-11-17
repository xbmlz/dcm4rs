use crate::dataset::DataSet;
use crate::tag::Tag;

/// The raw value of an element.
///
/// Values are always stored as the bytes that appear in the standard; the constructors in this
/// crate encode little endian, while bytes read from a big-endian transfer syntax are stored
/// as-is by the parser and decoded later through [`crate::Endian`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Value {
    /// No value (zero length).
    #[default]
    Empty,
    /// Raw bytes; multiple values are stored consecutively per VR semantics (text separated by
    /// `\`, numbers as fixed-length sequences).
    Bytes(Vec<u8>),
    /// A sequence value (VR = `SQ`).
    Sequence(Vec<DataSet>),
    /// Encapsulated pixel data: one entry per fragment item, the first being the Basic Offset
    /// Table (possibly empty).
    Fragments(Vec<Vec<u8>>),
}

impl Value {
    /// The empty value.
    pub const fn empty() -> Self {
        Self::Empty
    }

    /// Build from raw bytes; an empty byte sequence normalizes to [`Value::Empty`].
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        if bytes.is_empty() {
            Self::Empty
        } else {
            Self::Bytes(bytes)
        }
    }

    /// Build from a single text value (no character set conversion).
    #[allow(clippy::should_implement_trait)] // symmetric with the from_* constructors such as from_u16s
    pub fn from_str(text: &str) -> Self {
        Self::from_bytes(text.as_bytes().to_vec())
    }

    /// Build from several text values joined by backslashes; an empty slice returns
    /// [`Value::Empty`].
    pub fn from_strings(values: &[&str]) -> Self {
        Self::from_str(&values.join("\\"))
    }

    /// Build from a sequence of 16-bit unsigned integers (little endian).
    pub fn from_u16s(values: &[u16]) -> Self {
        Self::from_bytes(encode(values, u16::to_le_bytes))
    }

    /// Build from a sequence of 16-bit signed integers (little endian).
    pub fn from_i16s(values: &[i16]) -> Self {
        Self::from_bytes(encode(values, i16::to_le_bytes))
    }

    /// Build from a sequence of 32-bit unsigned integers (little endian).
    pub fn from_u32s(values: &[u32]) -> Self {
        Self::from_bytes(encode(values, u32::to_le_bytes))
    }

    /// Build from a sequence of 32-bit signed integers (little endian).
    pub fn from_i32s(values: &[i32]) -> Self {
        Self::from_bytes(encode(values, i32::to_le_bytes))
    }

    /// Build from a sequence of 64-bit unsigned integers (little endian).
    pub fn from_u64s(values: &[u64]) -> Self {
        Self::from_bytes(encode(values, u64::to_le_bytes))
    }

    /// Build from a sequence of 64-bit signed integers (little endian).
    pub fn from_i64s(values: &[i64]) -> Self {
        Self::from_bytes(encode(values, i64::to_le_bytes))
    }

    /// Build from a sequence of 32-bit floats (little endian).
    pub fn from_f32s(values: &[f32]) -> Self {
        Self::from_bytes(encode(values, f32::to_le_bytes))
    }

    /// Build from a sequence of 64-bit floats (little endian).
    pub fn from_f64s(values: &[f64]) -> Self {
        Self::from_bytes(encode(values, f64::to_le_bytes))
    }

    /// Build from a sequence of attribute tags (`AT`: each item is a 2-byte group plus a 2-byte
    /// element, little endian).
    pub fn from_tags(tags: &[Tag]) -> Self {
        let mut bytes = Vec::with_capacity(tags.len() * 4);
        for tag in tags {
            bytes.extend_from_slice(&tag.group().to_le_bytes());
            bytes.extend_from_slice(&tag.element().to_le_bytes());
        }
        Self::from_bytes(bytes)
    }

    /// Build from sequence items; an empty sequence normalizes to [`Value::Empty`].
    pub fn sequence(items: Vec<DataSet>) -> Self {
        if items.is_empty() {
            Self::Empty
        } else {
            Self::Sequence(items)
        }
    }

    /// Length: number of bytes for byte values, number of items for sequences, total payload
    /// length for encapsulated fragments.
    pub fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Bytes(bytes) => bytes.len(),
            Self::Sequence(items) => items.len(),
            Self::Fragments(fragments) => fragments.iter().map(Vec::len).sum(),
        }
    }

    /// Whether the value is empty.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Bytes(bytes) => bytes.is_empty(),
            Self::Sequence(items) => items.is_empty(),
            Self::Fragments(fragments) => fragments.is_empty(),
        }
    }

    /// Raw bytes; `None` for non-byte values.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Mutable raw bytes; `None` for non-byte values.
    pub fn as_bytes_mut(&mut self) -> Option<&mut Vec<u8>> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Sequence items; `None` for non-sequence values.
    pub fn as_sequence(&self) -> Option<&[DataSet]> {
        match self {
            Self::Sequence(items) => Some(items),
            _ => None,
        }
    }

    /// Mutable sequence items; `None` for non-sequence values.
    pub fn as_sequence_mut(&mut self) -> Option<&mut Vec<DataSet>> {
        match self {
            Self::Sequence(items) => Some(items),
            _ => None,
        }
    }

    /// Encapsulated fragments; `None` for other values.
    pub fn as_fragments(&self) -> Option<&[Vec<u8>]> {
        match self {
            Self::Fragments(fragments) => Some(fragments),
            _ => None,
        }
    }

    /// Mutable encapsulated fragments; `None` for other values.
    pub fn as_fragments_mut(&mut self) -> Option<&mut Vec<Vec<u8>>> {
        match self {
            Self::Fragments(fragments) => Some(fragments),
            _ => None,
        }
    }

    /// Whether the value is a sequence.
    pub fn is_sequence(&self) -> bool {
        matches!(self, Self::Sequence(_))
    }

    /// Whether the value holds encapsulated pixel data.
    pub fn is_fragments(&self) -> bool {
        matches!(self, Self::Fragments(_))
    }
}

fn encode<T: Copy, const N: usize>(values: &[T], to_bytes: fn(T) -> [u8; N]) -> Vec<u8> {
    values.iter().copied().flat_map(to_bytes).collect()
}
