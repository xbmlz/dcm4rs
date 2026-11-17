//! Transfer syntaxes and the encoding rules they imply (PS3.5 Annex A).

use std::fmt;

use crate::endian::Endian;

/// A DICOM transfer syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TransferSyntax {
    /// Implicit VR Little Endian (`1.2.840.10008.1.2`); the VR of every element comes from the
    /// data dictionary instead of the stream.
    ImplicitVrLittleEndian,
    /// Explicit VR Little Endian (`1.2.840.10008.1.2.1`).
    ExplicitVrLittleEndian,
    /// Explicit VR Big Endian (`1.2.840.10008.1.2.2`); retired, but still encountered.
    ExplicitVrBigEndian,
    /// Deflated Explicit VR Little Endian (`1.2.840.10008.1.2.1.99`). The data set is inflated
    /// with raw DEFLATE (RFC 1951) before being read as explicit VR little endian.
    DeflatedExplicitVrLittleEndian,
    /// An encapsulated (compressed) transfer syntax, identified by its UID. The data set itself is
    /// always explicit VR little endian; only pixel data travels as fragments.
    Encapsulated(&'static str),
}

/// Encapsulated transfer syntaxes from the PS3.6 registry (PS3.5 Annex A.3).
const ENCAPSULATED: &[&str] = &[
    "1.2.840.10008.1.2.4.50",
    "1.2.840.10008.1.2.4.51",
    "1.2.840.10008.1.2.4.52",
    "1.2.840.10008.1.2.4.53",
    "1.2.840.10008.1.2.4.54",
    "1.2.840.10008.1.2.4.55",
    "1.2.840.10008.1.2.4.56",
    "1.2.840.10008.1.2.4.57",
    "1.2.840.10008.1.2.4.70",
    "1.2.840.10008.1.2.4.80",
    "1.2.840.10008.1.2.4.81",
    "1.2.840.10008.1.2.4.90",
    "1.2.840.10008.1.2.4.91",
    "1.2.840.10008.1.2.4.92",
    "1.2.840.10008.1.2.4.93",
    "1.2.840.10008.1.2.4.94",
    "1.2.840.10008.1.2.4.95",
    "1.2.840.10008.1.2.4.100",
    "1.2.840.10008.1.2.4.101",
    "1.2.840.10008.1.2.4.102",
    "1.2.840.10008.1.2.4.103",
    "1.2.840.10008.1.2.4.104",
    "1.2.840.10008.1.2.4.105",
    "1.2.840.10008.1.2.4.106",
    "1.2.840.10008.1.2.4.107",
    "1.2.840.10008.1.2.4.108",
    "1.2.840.10008.1.2.4.110",
    "1.2.840.10008.1.2.4.111",
    "1.2.840.10008.1.2.4.112",
    "1.2.840.10008.1.2.4.201",
    "1.2.840.10008.1.2.4.202",
    "1.2.840.10008.1.2.4.203",
    "1.2.840.10008.1.2.4.204",
    "1.2.840.10008.1.2.5",
];

impl TransferSyntax {
    /// Resolve a transfer syntax UID; `None` when the UID is not one recognized here.
    pub fn from_uid(uid: &str) -> Option<Self> {
        let uid = uid.trim_matches(|ch: char| ch.is_whitespace() || ch == '\0');
        match uid {
            "1.2.840.10008.1.2" => Some(Self::ImplicitVrLittleEndian),
            "1.2.840.10008.1.2.1" => Some(Self::ExplicitVrLittleEndian),
            "1.2.840.10008.1.2.2" => Some(Self::ExplicitVrBigEndian),
            "1.2.840.10008.1.2.1.99" => Some(Self::DeflatedExplicitVrLittleEndian),
            _ => ENCAPSULATED
                .iter()
                .find(|known| **known == uid)
                .map(|known| Self::Encapsulated(known)),
        }
    }

    /// The UID of this transfer syntax.
    pub const fn uid(self) -> &'static str {
        match self {
            Self::ImplicitVrLittleEndian => "1.2.840.10008.1.2",
            Self::ExplicitVrLittleEndian => "1.2.840.10008.1.2.1",
            Self::ExplicitVrBigEndian => "1.2.840.10008.1.2.2",
            Self::DeflatedExplicitVrLittleEndian => "1.2.840.10008.1.2.1.99",
            Self::Encapsulated(uid) => uid,
        }
    }

    /// The byte order of numbers in this transfer syntax.
    pub const fn endian(self) -> Endian {
        match self {
            Self::ExplicitVrBigEndian => Endian::Big,
            _ => Endian::Little,
        }
    }

    /// Whether every element carries its VR in the stream.
    pub const fn is_explicit_vr(self) -> bool {
        !matches!(self, Self::ImplicitVrLittleEndian)
    }

    /// Whether pixel data is carried as encapsulated fragments.
    pub const fn is_encapsulated(self) -> bool {
        matches!(self, Self::Encapsulated(_))
    }
}

impl fmt::Display for TransferSyntax {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.uid())
    }
}
