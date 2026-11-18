//! VR `DS`: a decimal string that keeps the text it was read from.

use std::fmt;

use crate::error::{Error, Result};

/// Longest value PS3.5 6.2 allows for VR `DS`, in characters.
const MAX_LENGTH: usize = 16;

/// A decimal string: the number and the characters it was written as.
///
/// `1.00` and `1` are the same number and different bytes, and both have to survive a read and
/// write. Keeping the text is why pydicom's `DSfloat` does not simply store a float; this does
/// the same and parses the number alongside it.
#[derive(Clone, Debug, PartialEq)]
pub struct DecimalString {
    text: String,
    value: f64,
}

impl DecimalString {
    /// Parse one `DS` value, trimming the spaces PS3.5 allows around it.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.len() > MAX_LENGTH {
            return Err(Error::InvalidValue(format!(
                "a decimal string has at most {MAX_LENGTH} characters: {text}"
            )));
        }
        if !looks_like_decimal(text) {
            return Err(Error::InvalidValue(format!("not a decimal string: {text}")));
        }
        let value: f64 = text
            .parse()
            .map_err(|_| Error::InvalidValue(format!("not a decimal string: {text}")))?;
        if !value.is_finite() {
            return Err(Error::InvalidValue(format!(
                "a decimal string must be finite: {text}"
            )));
        }
        Ok(Self {
            text: text.to_owned(),
            value,
        })
    }

    /// Format a number as a `DS` value of at most sixteen characters.
    ///
    /// Long values switch to scientific notation where pydicom's `format_number_as_ds` does, but
    /// the digits come from Rust's float formatting, so the text can differ from Python's for the
    /// same number: Rust writes an exponent as `2e3`, Python as `2e+03`. Exactly representable
    /// values that fit in sixteen characters are written unchanged.
    pub fn from_f64(value: f64) -> Result<Self> {
        if !value.is_finite() {
            return Err(Error::InvalidValue(format!(
                "a decimal string must be finite: {value}"
            )));
        }
        let text = format_ds(value);
        Self::parse(&text)
    }

    /// The characters the value was written as.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The number the text represents.
    pub fn value(&self) -> f64 {
        self.value
    }
}

impl fmt::Display for DecimalString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl From<DecimalString> for String {
    fn from(value: DecimalString) -> Self {
        value.text
    }
}

/// Format `value` so `DS`'s sixteen-character limit holds.
///
/// The thresholds are pydicom's: scientific notation below `1e-4` and at or above `1e13` (one
/// exponent digit fewer for negative numbers, where the sign costs a character).
fn format_ds(value: f64) -> String {
    let shortest = format!("{value}");
    if shortest.len() <= MAX_LENGTH {
        return shortest;
    }

    let magnitude = if value == 0.0 {
        f64::NEG_INFINITY
    } else {
        value.abs().log10()
    };
    let sign = usize::from(value < 0.0);
    let scientific = magnitude < -4.0 || magnitude >= (14 - sign) as f64;

    if scientific {
        let digits = 10 - sign;
        let text = format!("{value:.digits$e}");
        if text.len() > MAX_LENGTH {
            return format!("{value:.0$e}", digits - 1);
        }
        text
    } else if magnitude >= 1.0 {
        // Leave room for the sign, the digits left of the point and the point itself.
        let decimals = 14 - sign - magnitude.floor() as usize;
        format!("{value:.decimals$}")
    } else {
        format!("{value:.0$}", 14 - sign)
    }
}

/// Whether `text` matches PS3.5's grammar for `DS`: an optional sign, digits with an optional
/// decimal point and an optional exponent.
fn looks_like_decimal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        index += 1;
    }

    let mut digits = 0usize;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
        digits += 1;
    }
    let mut fraction = 0usize;
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
            fraction += 1;
        }
    }
    if digits + fraction == 0 {
        return false;
    }

    if index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
        index += 1;
        if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
            index += 1;
        }
        let mut exponent = 0usize;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
            exponent += 1;
        }
        if exponent == 0 {
            return false;
        }
    }

    index == bytes.len()
}
