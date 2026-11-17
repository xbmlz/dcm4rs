//! DICOM unique identifiers: validation and the PS3.6 registry.
//!
//! pydicom models a UID as a `str` subclass that looks its name, type and retirement status up
//! in the registry. [`Uid`] keeps the same answers in one owned value: it validates on
//! construction (VR `UI`, PS3.5 6.2 — at most 64 characters of `0`-`9` and `.`, no leading zero
//! in a component) and serves [`Uid::name`], [`Uid::keyword`], [`Uid::kind`], [`Uid::info`],
//! [`Uid::is_retired`] and the transfer-syntax predicates from the table in `generated.rs`.
//!
//! The predicates that only make sense for a transfer syntax return `Option<bool>`, `None`
//! standing in for the `ValueError` pydicom raises: `Some(false)` and "not a transfer syntax"
//! are different answers, and flattening them would lose the distinction.

mod generated;

use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use crate::error::{Error, Result};

pub use generated::Entry;

/// Transfer syntaxes whose data set is not encapsulated (PS3.5 A.1 to A.4).
const NATIVE: &[&str] = &[
    "1.2.840.10008.1.2",      // Implicit VR Little Endian
    "1.2.840.10008.1.2.1",    // Explicit VR Little Endian
    "1.2.840.10008.1.2.2",    // Explicit VR Big Endian
    "1.2.840.10008.1.2.1.99", // Deflated Explicit VR Little Endian
];

/// Transfer syntaxes that deflate the data set (PS3.5 A.5, A.7 and A.12).
const DEFLATED: &[&str] = &[
    "1.2.840.10008.1.2.1.99",  // Deflated Explicit VR Little Endian
    "1.2.840.10008.1.2.4.95",  // JPIP Referenced Deflate
    "1.2.840.10008.1.2.4.205", // JPIP HTJ2K Referenced Deflate
];

/// Implicit VR Little Endian, the only transfer syntax that omits the VR.
const IMPLICIT_VR_LITTLE_ENDIAN: &str = "1.2.840.10008.1.2";

/// Explicit VR Big Endian, the only standard syntax that reads big-endian values.
const EXPLICIT_VR_BIG_ENDIAN: &str = "1.2.840.10008.1.2.2";

/// The root every DICOM-registered UID starts with.
const DICOM_ROOT: &str = "1.2.840.10008.";

/// Whether `value` is a well-formed UID as PS3.5 6.2 defines it for VR `UI`.
///
/// The value may be at most 64 characters drawn from the digits and the period, every component
/// between periods must be present, and no component may have a leading zero.
pub fn is_valid(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    value.split('.').all(|component| {
        !component.is_empty()
            && component.bytes().all(|byte| byte.is_ascii_digit())
            && (component == "0" || !component.starts_with('0'))
    })
}

/// A DICOM unique identifier.
///
/// A registry UID answers with the name, type and retirement status PS3.6 gives it; an
/// unregistered one answers with itself as the name and empties everywhere else, which is how
/// pydicom reports a private UID.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Uid {
    value: String,
    /// Encoding declared for a privately defined transfer syntax, as
    /// (implicit VR, little endian).
    private_encoding: Option<(bool, bool)>,
}

impl Uid {
    /// Validate and wrap one UID, trimming surrounding whitespace as pydicom does.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value: String = value.into();
        let trimmed = value.trim().to_owned();
        if !is_valid(&trimmed) {
            return Err(Error::InvalidUid(trimmed));
        }
        Ok(Self {
            value: trimmed,
            private_encoding: None,
        })
    }

    /// The UID text.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// The registry row, if PS3.6 registers this UID.
    pub fn entry(&self) -> Option<&'static Entry> {
        registry(&self.value)
    }

    /// The registry name, or the UID itself when it is not registered.
    pub fn name(&self) -> &str {
        self.entry().map_or(&self.value, |entry| entry.name)
    }

    /// The DICOM keyword, empty when the UID is not registered.
    pub fn keyword(&self) -> &str {
        self.entry().map_or("", |entry| entry.keyword)
    }

    /// The registry type, such as `Transfer Syntax` or `SOP Class`, empty when unregistered.
    pub fn kind(&self) -> &str {
        self.entry().map_or("", |entry| entry.kind)
    }

    /// The registry's extra information, empty when there is none or the UID is unregistered.
    pub fn info(&self) -> &str {
        self.entry().map_or("", |entry| entry.info)
    }

    /// Whether PS3.6 retires this UID.
    pub fn is_retired(&self) -> bool {
        self.entry().is_some_and(|entry| entry.retired)
    }

    /// Whether the UID lies outside the DICOM registration root `1.2.840.10008.`.
    pub fn is_private(&self) -> bool {
        !self.value.starts_with(DICOM_ROOT)
    }

    /// Whether the UID is well-formed; always true for a [`Uid`] built by [`Uid::new`].
    pub fn is_valid(&self) -> bool {
        is_valid(&self.value)
    }

    /// Whether this is a transfer syntax: a registered one, or a private UID whose encoding was
    /// declared with [`Uid::with_private_encoding`].
    pub fn is_transfer_syntax(&self) -> bool {
        match self.entry() {
            Some(entry) => entry.kind == "Transfer Syntax",
            None => self.private_encoding.is_some(),
        }
    }

    /// Whether the encoding omits the VR, `None` when this is not a transfer syntax.
    pub fn is_implicit_vr(&self) -> Option<bool> {
        if let Some((implicit, _)) = self.private_encoding {
            return Some(implicit);
        }
        if !self.is_transfer_syntax() {
            return None;
        }
        Some(self.value == IMPLICIT_VR_LITTLE_ENDIAN)
    }

    /// Whether the encoding reads little-endian values, `None` when this is not a transfer
    /// syntax.
    pub fn is_little_endian(&self) -> Option<bool> {
        if let Some((_, little_endian)) = self.private_encoding {
            return Some(little_endian);
        }
        if !self.is_transfer_syntax() {
            return None;
        }
        Some(self.value != EXPLICIT_VR_BIG_ENDIAN)
    }

    /// Whether the data set is deflated, `None` when this is not a transfer syntax.
    pub fn is_deflated(&self) -> Option<bool> {
        if !self.is_transfer_syntax() {
            return None;
        }
        Some(DEFLATED.contains(&self.value.as_str()))
    }

    /// Whether the payload is encapsulated (compressed), `None` when this is not a transfer
    /// syntax.
    pub fn is_compressed(&self) -> Option<bool> {
        if !self.is_transfer_syntax() {
            return None;
        }
        Some(!NATIVE.contains(&self.value.as_str()))
    }

    /// Whether the payload is encapsulated; the same answer as [`Uid::is_compressed`].
    pub fn is_encapsulated(&self) -> Option<bool> {
        self.is_compressed()
    }

    /// Declare the encoding of a privately defined transfer syntax.
    ///
    /// pydicom requires this before a private UID counts as a transfer syntax, because nothing
    /// else says whether it is implicit VR or big endian.
    pub fn with_private_encoding(mut self, implicit_vr: bool, little_endian: bool) -> Self {
        self.private_encoding = Some((implicit_vr, little_endian));
        self
    }
}

impl Deref for Uid {
    type Target = str;

    fn deref(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

impl FromStr for Uid {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::new(value)
    }
}

impl PartialEq<str> for Uid {
    fn eq(&self, other: &str) -> bool {
        self.value == other
    }
}

impl PartialEq<&str> for Uid {
    fn eq(&self, other: &&str) -> bool {
        self.value == *other
    }
}

/// Find the registry row for `value`.
fn registry(value: &str) -> Option<&'static Entry> {
    generated::ENTRIES
        .binary_search_by(|entry| entry.uid.cmp(value))
        .ok()
        .map(|index| &generated::ENTRIES[index])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_for_binary_search() {
        let mut previous = "";
        for entry in generated::ENTRIES {
            assert!(entry.uid > previous, "{} out of order", entry.uid);
            previous = entry.uid;
            // Two retired placeholders carry neither name nor keyword, only an info marker.
            assert!(
                !entry.name.is_empty() || entry.retired,
                "{} has no name but is not retired",
                entry.uid
            );
        }
    }

    #[test]
    fn every_native_syntax_is_registered() {
        for uid in NATIVE.iter().chain(DEFLATED) {
            let entry = registry(uid).unwrap_or_else(|| panic!("{uid} is not registered"));
            assert_eq!(entry.kind, "Transfer Syntax");
        }
    }
}
