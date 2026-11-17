use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

/// DICOM Value Representation: the 34 VRs defined by PS3.5.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[repr(u8)]
pub enum Vr {
    /// AE — Application Entity
    Ae,
    /// AS — Age String
    As,
    /// AT — Attribute Tag
    At,
    /// CS — Code String
    Cs,
    /// DA — Date
    Da,
    /// DS — Decimal String
    Ds,
    /// DT — Date Time
    Dt,
    /// FD — Floating Point Double (64-bit float)
    Fd,
    /// FL — Floating Point Single (32-bit float)
    Fl,
    /// IS — Integer String
    Is,
    /// LO — Long String
    Lo,
    /// LT — Long Text
    Lt,
    /// OB — Other Byte
    Ob,
    /// OD — Other Double (64-bit float)
    Od,
    /// OF — Other Float (32-bit float)
    Of,
    /// OL — Other Long (32-bit integer)
    Ol,
    /// OV — Other 64-bit Very Long (64-bit integer)
    Ov,
    /// OW — Other Word (16-bit word)
    Ow,
    /// PN — Person Name
    Pn,
    /// SH — Short String
    Sh,
    /// SL — Signed Long (32-bit signed integer)
    Sl,
    /// SQ — Sequence of Items
    Sq,
    /// SS — Signed Short (16-bit signed integer)
    Ss,
    /// ST — Short Text
    St,
    /// SV — Signed 64-bit Very Long (64-bit signed integer)
    Sv,
    /// TM — Time
    Tm,
    /// UC — Unlimited Characters
    Uc,
    /// UI — Unique Identifier
    Ui,
    /// UL — Unsigned Long (32-bit unsigned integer)
    Ul,
    /// UN — Unknown
    Un,
    /// UR — Universal Resource Identifier
    Ur,
    /// US — Unsigned Short (16-bit unsigned integer)
    Us,
    /// UT — Unlimited Text
    Ut,
    /// UV — Unsigned 64-bit Very Long (64-bit unsigned integer)
    Uv,
}

impl Vr {
    /// All 34 VRs, in standard order.
    pub const ALL: [Self; 34] = [
        Self::Ae,
        Self::As,
        Self::At,
        Self::Cs,
        Self::Da,
        Self::Ds,
        Self::Dt,
        Self::Fd,
        Self::Fl,
        Self::Is,
        Self::Lo,
        Self::Lt,
        Self::Ob,
        Self::Od,
        Self::Of,
        Self::Ol,
        Self::Ov,
        Self::Ow,
        Self::Pn,
        Self::Sh,
        Self::Sl,
        Self::Sq,
        Self::Ss,
        Self::St,
        Self::Sv,
        Self::Tm,
        Self::Uc,
        Self::Ui,
        Self::Ul,
        Self::Un,
        Self::Ur,
        Self::Us,
        Self::Ut,
        Self::Uv,
    ];

    /// Two-character spelling used by the standard, e.g. `"OB"`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ae => "AE",
            Self::As => "AS",
            Self::At => "AT",
            Self::Cs => "CS",
            Self::Da => "DA",
            Self::Ds => "DS",
            Self::Dt => "DT",
            Self::Fd => "FD",
            Self::Fl => "FL",
            Self::Is => "IS",
            Self::Lo => "LO",
            Self::Lt => "LT",
            Self::Ob => "OB",
            Self::Od => "OD",
            Self::Of => "OF",
            Self::Ol => "OL",
            Self::Ov => "OV",
            Self::Ow => "OW",
            Self::Pn => "PN",
            Self::Sh => "SH",
            Self::Sl => "SL",
            Self::Sq => "SQ",
            Self::Ss => "SS",
            Self::St => "ST",
            Self::Sv => "SV",
            Self::Tm => "TM",
            Self::Uc => "UC",
            Self::Ui => "UI",
            Self::Ul => "UL",
            Self::Un => "UN",
            Self::Ur => "UR",
            Self::Us => "US",
            Self::Ut => "UT",
            Self::Uv => "UV",
        }
    }

    /// Whether this VR is textual (value stored as text, possibly multi-valued).
    pub const fn is_string(self) -> bool {
        matches!(
            self,
            Self::Ae
                | Self::As
                | Self::Cs
                | Self::Da
                | Self::Ds
                | Self::Dt
                | Self::Is
                | Self::Lo
                | Self::Lt
                | Self::Pn
                | Self::Sh
                | Self::St
                | Self::Tm
                | Self::Uc
                | Self::Ui
                | Self::Ur
                | Self::Ut
        )
    }

    /// Whether its text is governed by `Specific Character Set` (PS3.3 C.12.1.1.2).
    pub const fn uses_specific_character_set(self) -> bool {
        matches!(
            self,
            Self::Lo | Self::Lt | Self::Pn | Self::Sh | Self::St | Self::Uc | Self::Ut | Self::Ur
        )
    }

    /// Whether explicit-VR encoding uses the "2 reserved bytes + 4-byte length" form (PS3.5 6.2).
    pub const fn uses_long_length(self) -> bool {
        matches!(
            self,
            Self::Ob
                | Self::Od
                | Self::Of
                | Self::Ol
                | Self::Ov
                | Self::Ow
                | Self::Sq
                | Self::Uc
                | Self::Un
                | Self::Ur
                | Self::Ut
        )
    }

    /// Whether multiple values are separated by backslashes. In `LT`, `ST`, `UT` and `UR` a
    /// backslash is an ordinary character (PS3.5 6.2).
    pub const fn splits_on_backslash(self) -> bool {
        self.is_string() && !matches!(self, Self::Lt | Self::St | Self::Ut | Self::Ur)
    }

    /// Pad byte used to reach even length: NUL for `UI`, a space for other textual VRs, NUL for
    /// binary VRs.
    pub const fn padding(self) -> u8 {
        if matches!(self, Self::Ui) {
            0
        } else if self.is_string() {
            b' '
        } else {
            0
        }
    }

    /// Whether this VR is a sequence.
    pub const fn is_sequence(self) -> bool {
        matches!(self, Self::Sq)
    }
}

impl fmt::Display for Vr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Vr {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        let upper = text.trim().to_ascii_uppercase();
        Self::ALL
            .iter()
            .copied()
            .find(|vr| vr.as_str() == upper)
            .ok_or_else(|| Error::UnknownVr(text.to_owned()))
    }
}
