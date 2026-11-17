use std::fmt;
use std::str::Utf8Error;

use crate::tag::Tag;
use crate::vr::Vr;

/// Errors the data model and the encoders can produce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The tag text could not be parsed.
    InvalidTag(String),
    /// Not a known VR.
    UnknownVr(String),
    /// The VR does not match the requested decoding.
    WrongVr {
        /// The element's actual VR.
        actual: Vr,
        /// The VR (or VR family) the decoding requires.
        expected: &'static str,
    },
    /// The value's byte length is not a multiple of its VR's unit.
    WrongLength {
        /// The element's VR.
        vr: Vr,
        /// Actual byte length.
        len: usize,
        /// Expected unit length (2, 4 or 8).
        unit: usize,
    },
    /// The value's bytes are not valid UTF-8.
    InvalidUtf8(Utf8Error),
    /// A numeric string could not be parsed.
    InvalidNumber(String),
    /// The data set does not contain this tag.
    MissingTag(Tag),
    /// The tag is present but has no value.
    NoValue(Tag),
    /// The value is not a sequence, or the VR is not `SQ`.
    NotASequence(Vr),
    /// The encoded data ended in the middle of an item.
    UnexpectedEof {
        /// Byte offset in the input where more data was required.
        offset: usize,
    },
    /// The encoded data is structurally invalid.
    InvalidData {
        /// Byte offset in the input where the problem was detected.
        offset: usize,
        /// What is wrong at that offset.
        reason: &'static str,
    },
    /// The file meta information contains an element outside group 2.
    InvalidMeta(Tag),
    /// The character set is unknown or its code extension is not handled here.
    UnsupportedCharacterSet(String),
    /// The value bytes are not valid text for the declared character set.
    InvalidText(String),
    /// The transfer syntax is recognized but cannot be handled here.
    UnsupportedTransferSyntax(String),
    /// The pixel data uses a compression this build does not decode.
    UnsupportedPixelData(String),
    /// The pixel data attributes or an encoded frame are malformed.
    InvalidPixelData(String),
    /// A file could not be read from disk.
    Io(String),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTag(text) => write!(f, "invalid tag text: {text:?}"),
            Self::UnknownVr(text) => write!(f, "unknown VR: {text:?}"),
            Self::WrongVr { actual, expected } => {
                write!(f, "VR mismatch: expected {expected}, got {actual}")
            }
            Self::WrongLength { vr, len, unit } => {
                write!(
                    f,
                    "value byte length {len} is not a multiple of {unit} (VR={vr})"
                )
            }
            Self::InvalidUtf8(err) => write!(f, "value bytes are not valid UTF-8: {err}"),
            Self::InvalidNumber(text) => write!(f, "could not parse number: {text:?}"),
            Self::MissingTag(tag) => write!(f, "tag {tag} is missing from the data set"),
            Self::NoValue(tag) => write!(f, "tag {tag} has no value"),
            Self::NotASequence(vr) => write!(f, "VR {vr} is not a sequence"),
            Self::UnexpectedEof { offset } => {
                write!(f, "unexpected end of data at offset {offset}")
            }
            Self::InvalidData { offset, reason } => {
                write!(f, "invalid data at offset {offset}: {reason}")
            }
            Self::InvalidMeta(tag) => write!(
                f,
                "file meta information may only contain group 2 elements, found {tag}"
            ),
            Self::UnsupportedCharacterSet(name) => {
                write!(f, "unsupported character set: {name}")
            }
            Self::InvalidText(charset) => {
                write!(
                    f,
                    "value bytes are not valid text for character set {charset}"
                )
            }
            Self::UnsupportedTransferSyntax(uid) => write!(f, "unsupported transfer syntax: {uid}"),
            Self::UnsupportedPixelData(message) => {
                write!(f, "unsupported pixel data: {message}")
            }
            Self::InvalidPixelData(message) => write!(f, "invalid pixel data: {message}"),
            Self::Io(message) => write!(f, "I/O error: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidUtf8(err) => Some(err),
            _ => None,
        }
    }
}
