//! VR `IS`: an integer string that keeps the text it was read from.

use std::fmt;

use crate::error::{Error, Result};

/// Longest value PS3.5 6.2 allows for VR `IS`, in characters.
const MAX_LENGTH: usize = 12;

/// The range PS3.5 permits for an `IS` value: a 32-bit signed integer.
const RANGE: std::ops::RangeInclusive<i64> = -(1i64 << 31)..=(1i64 << 31) - 1;

/// An integer string: the number and the characters it was written as.
///
/// `007` and `7` are the same number and different bytes, and both have to survive a read and
/// write. Keeping the text is why pydicom's `IS` keeps `original_string`; this does the same.
///
/// A value that is not an integer — pydicom turns those into `ISfloat` — is rejected here
/// rather than widened: the VR does not permit one, and reading it back as a float would write
/// a value the standard does not allow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegerString {
    text: String,
    value: i32,
}

impl IntegerString {
    /// Parse one `IS` value, trimming the spaces PS3.5 allows around it.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.len() > MAX_LENGTH {
            return Err(Error::InvalidValue(format!(
                "an integer string has at most {MAX_LENGTH} characters: {text}"
            )));
        }
        if !looks_like_integer(text) {
            return Err(Error::InvalidValue(format!(
                "not an integer string: {text}"
            )));
        }
        let value: i64 = text
            .parse()
            .map_err(|_| Error::InvalidValue(format!("not an integer string: {text}")))?;
        if !RANGE.contains(&value) {
            return Err(Error::InvalidValue(format!(
                "an integer string must be between {} and {}: {text}",
                RANGE.start(),
                RANGE.end()
            )));
        }
        Ok(Self {
            text: text.to_owned(),
            value: value as i32,
        })
    }

    /// Write an integer as an `IS` value.
    pub fn from_i32(value: i32) -> Self {
        Self {
            text: value.to_string(),
            value,
        }
    }

    /// The characters the value was written as.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The number the text represents.
    pub fn value(&self) -> i32 {
        self.value
    }
}

impl fmt::Display for IntegerString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl From<IntegerString> for String {
    fn from(value: IntegerString) -> Self {
        value.text
    }
}

/// Whether `text` matches PS3.5's grammar for `IS`: an optional sign and digits.
fn looks_like_integer(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        index += 1;
    }
    if index == bytes.len() {
        return false;
    }
    bytes[index..].iter().all(u8::is_ascii_digit)
}
