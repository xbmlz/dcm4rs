use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

/// A DICOM data element tag: a 16-bit group number plus a 16-bit element number.
///
/// Ordering is numeric (group, element) ascending, which matches the order required when writing
/// a data set, so a tag can be used directly as the key of an ordered map.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(u32);

impl Tag {
    /// Build from a group number and an element number.
    pub const fn new(group: u16, element: u16) -> Self {
        Self((group as u32) << 16 | element as u32)
    }

    /// Build from a raw 32-bit value; the high 16 bits are the group number.
    pub const fn from_u32(raw: u32) -> Self {
        Self(raw)
    }

    /// Group number.
    pub const fn group(self) -> u16 {
        (self.0 >> 16) as u16
    }

    /// Element number.
    pub const fn element(self) -> u16 {
        self.0 as u16
    }

    /// Raw 32-bit value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Whether this is a private tag: an odd group number that is not one of the reserved groups
    /// 0001/0003/0005/0007/FFFF.
    pub const fn is_private(self) -> bool {
        let group = self.group();
        group & 1 == 1 && !matches!(group, 0x0001 | 0x0003 | 0x0005 | 0x0007 | 0xFFFF)
    }

    /// Whether this is a group length element (element number 0000).
    pub const fn is_group_length(self) -> bool {
        self.element() == 0x0000
    }

    /// Whether this is a private creator element (element numbers 0010..=00FF inside a private
    /// group).
    pub const fn is_private_creator(self) -> bool {
        let element = self.element();
        self.is_private() && element >= 0x0010 && element <= 0x00FF
    }

    /// The tag that immediately follows this one in the same data set; `None` at the 32-bit limit.
    pub const fn successor(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(raw) => Some(Self(raw)),
            None => None,
        }
    }

    /// Parse tag text, accepting `(0010,0010)`, `0010,0010` and `00100010`.
    pub fn parse(text: &str) -> Result<Self> {
        let trimmed = text.trim();
        let inner = match trimmed.strip_prefix('(') {
            Some(rest) => rest
                .strip_suffix(')')
                .ok_or_else(|| Error::InvalidTag(text.to_owned()))?,
            None => trimmed,
        };
        let (group, element) = match inner.split_once(',') {
            Some((group, element)) => (group.trim(), element.trim()),
            None => {
                let compact = inner.trim();
                if compact.len() != 8 {
                    return Err(Error::InvalidTag(text.to_owned()));
                }
                compact.split_at(4)
            }
        };
        let group = parse_hex(group, text)?;
        let element = parse_hex(element, text)?;
        Ok(Self::new(group, element))
    }
}

fn parse_hex(text: &str, original: &str) -> Result<u16> {
    if text.len() != 4 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::InvalidTag(original.to_owned()));
    }
    u16::from_str_radix(text, 16).map_err(|_| Error::InvalidTag(original.to_owned()))
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:04X},{:04X})", self.group(), self.element())
    }
}

impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tag({:04X},{:04X})", self.group(), self.element())
    }
}

impl FromStr for Tag {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        Self::parse(text)
    }
}
