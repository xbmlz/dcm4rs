//! `Specific Character Set` (0008,0005) support.
//!
//! The DICOM character set model is described in PS3.3 C.12.1.1. The mapping from character
//! set names to concrete encodings follows pydicom's `python_encoding` table
//! (pydicom/charset.py:25-60). ISO 2022 code extensions are decoded by the state machine below:
//! each escape sequence selects the character set for the fragment that follows it, and a
//! delimiter in a single-byte fragment returns to the default repertoire, matching pydicom's
//! `decode_bytes` (pydicom/charset.py:298-488).

use std::borrow::Cow;
use std::fmt;

use encoding_rs::Encoding;

use crate::element::Element;
use crate::error::{Error, Result};

#[path = "jis0212.rs"]
mod jis0212;

/// The character set a data set declares for its text values.
///
/// ISO_IR 6 (the default repertoire) and ISO_IR 100 are both decoded as ISO-8859-1, matching
/// pydicom's `default_encoding` (pydicom/charset.py:20-31).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Charset {
    /// ISO_IR 6 (when no `(0008,0005)` is present) and ISO_IR 100, decoded as ISO-8859-1.
    #[default]
    Latin1,
    /// ISO_IR 192, UTF-8.
    Utf8,
    /// ISO_IR 101, ISO-8859-2.
    Iso8859_2,
    /// ISO_IR 109, ISO-8859-3.
    Iso8859_3,
    /// ISO_IR 110, ISO-8859-4.
    Iso8859_4,
    /// ISO_IR 144, ISO-8859-5.
    Iso8859_5,
    /// ISO_IR 127, ISO-8859-6.
    Iso8859_6,
    /// ISO_IR 126, ISO-8859-7.
    Iso8859_7,
    /// ISO_IR 138, ISO-8859-8.
    Iso8859_8,
    /// ISO_IR 148, ISO-8859-9; decoded through windows-1254, which the WHATWG encoding
    /// standard uses as its ISO-8859-9 label (the two differ only in the C1 range).
    Iso8859_9,
    /// ISO_IR 166, TIS-620, decoded through windows-874 (a superset for the encoded range).
    Thai,
    /// ISO_IR 13, JIS X 0201 / Shift_JIS.
    ShiftJis,
    /// GBK (CP936).
    Gbk,
    /// GB18030.
    Gb18030,
    /// ISO 2022 code extensions; the declaration order is preserved for the default set.
    Iso2022(Iso2022Config),
}

/// The character sets declared by an ISO 2022 code extension in `(0008,0005)`.
///
/// The first set is the default repertoire used for text without escape sequences; the
/// remaining sets are the G1 designations made available to escape sequences. The list is
/// capped at four values, which covers the combinations DICOM defines in practice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Iso2022Config {
    sets: [CodeSet; 4],
    len: u8,
}

impl Iso2022Config {
    fn new(sets: &[CodeSet]) -> Option<Self> {
        if sets.len() > 4 {
            return None;
        }
        let mut slots = [CodeSet::Ascii; 4];
        slots[..sets.len()].copy_from_slice(sets);
        Some(Self {
            sets: slots,
            len: sets.len() as u8,
        })
    }

    fn names(self) -> String {
        self.sets[..usize::from(self.len)]
            .iter()
            .map(|set| set.name())
            .collect::<Vec<_>>()
            .join("\\")
    }

    /// Whether the escape sequence may select this set: the default repertoire is always
    /// available, the rest must be declared.
    fn declares(self, set: CodeSet) -> bool {
        set == CodeSet::Ascii || self.sets[..usize::from(self.len)].contains(&set)
    }

    /// Decode one value. Escape sequences split the value into fragments; each fragment is
    /// decoded with the set its escape sequence designates.
    fn decode(self, bytes: &[u8], delimiters: &[u8], charset: Charset) -> Result<String> {
        let first = self.sets[0];
        if !bytes.contains(&ESC) {
            return first.decode(bytes, charset);
        }

        let mut out = String::new();
        let first_escape = bytes.iter().position(|byte| *byte == ESC).unwrap();
        if first_escape > 0 {
            out.push_str(&first.decode(&bytes[..first_escape], charset)?);
        }

        let mut start = first_escape;
        while start < bytes.len() {
            let end = bytes[start + 1..]
                .iter()
                .position(|byte| *byte == ESC)
                .map(|offset| start + 1 + offset)
                .unwrap_or(bytes.len());
            let fragment = &bytes[start..end];
            let Some((set, sequence_length)) = parse_escape(fragment) else {
                // pydicom falls back to the default encoding for unknown sequences after a
                // warning; we report the value as invalid instead of decoding escape bytes as
                // text (PS3.5 6.1.2.4 requires known escape sequences).
                return Err(Error::InvalidText(charset.name().into_owned()));
            };
            if !self.declares(set) {
                return Err(Error::InvalidText(charset.name().into_owned()));
            }
            let body = &fragment[sequence_length..];
            match set {
                // ISO-2022-JP keeps the escape sequence in the input it decodes.
                CodeSet::JisX0208 => out.push_str(&decode_jis0208(fragment, charset)?),
                CodeSet::JisX0212 => out.push_str(&decode_jis0212(body, charset)?),
                _ => {
                    // pydicom strips the designation escape from single-byte, EUC-KR and GB2312
                    // fragments and resets to the default repertoire at the first delimiter.
                    if let Some(index) = delimiters
                        .iter()
                        .filter_map(|delimiter| body.iter().position(|byte| byte == delimiter))
                        .min()
                    {
                        out.push_str(&set.decode(&body[..index], charset)?);
                        out.push_str(&first.decode(&body[index..], charset)?);
                    } else {
                        out.push_str(&set.decode(body, charset)?);
                    }
                }
            }
            start = end;
        }
        Ok(out)
    }
}

impl Charset {
    /// Resolve the value of `(0008,0005)`. Multiple values are separated by a backslash.
    ///
    /// Returns [`Error::UnsupportedCharacterSet`] for a name that is unknown or for a code
    /// extension that declares more than four sets.
    pub fn from_specific_character_set(value: &str) -> Result<Self> {
        let names: Vec<&str> = value.split('\\').map(str::trim).collect();
        if names
            .iter()
            .any(|name| name.starts_with("ISO 2022 ") || *name == "ISO 2022 58")
        {
            return Self::from_code_extension(&names, value);
        }
        match names.as_slice() {
            [""] | ["ISO_IR 6"] => Ok(Self::Latin1),
            ["ISO_IR 13"] => Ok(Self::ShiftJis),
            ["ISO_IR 100"] => Ok(Self::Latin1),
            ["ISO_IR 101"] => Ok(Self::Iso8859_2),
            ["ISO_IR 109"] => Ok(Self::Iso8859_3),
            ["ISO_IR 110"] => Ok(Self::Iso8859_4),
            ["ISO_IR 126"] => Ok(Self::Iso8859_7),
            ["ISO_IR 127"] => Ok(Self::Iso8859_6),
            ["ISO_IR 138"] => Ok(Self::Iso8859_8),
            ["ISO_IR 144"] => Ok(Self::Iso8859_5),
            ["ISO_IR 148"] => Ok(Self::Iso8859_9),
            ["ISO_IR 166"] => Ok(Self::Thai),
            ["ISO_IR 192"] => Ok(Self::Utf8),
            ["GBK"] => Ok(Self::Gbk),
            ["GB18030"] => Ok(Self::Gb18030),
            _ => Err(Error::UnsupportedCharacterSet(value.to_owned())),
        }
    }

    /// Parse a code-extension declaration, including the stand-alone `ISO 2022 GBK` alias.
    fn from_code_extension(names: &[&str], original: &str) -> Result<Self> {
        let mut sets = Vec::with_capacity(names.len());
        for name in names {
            let set = match *name {
                "" | "ISO 2022 IR 6" => CodeSet::Ascii,
                "ISO 2022 IR 13" => CodeSet::JisX0201,
                "ISO 2022 IR 87" => CodeSet::JisX0208,
                "ISO 2022 IR 100" => CodeSet::Latin1,
                "ISO 2022 IR 101" => CodeSet::Latin2,
                "ISO 2022 IR 109" => CodeSet::Latin3,
                "ISO 2022 IR 110" => CodeSet::Latin4,
                "ISO 2022 IR 126" => CodeSet::Greek,
                "ISO 2022 IR 127" => CodeSet::Arabic,
                "ISO 2022 IR 138" => CodeSet::Hebrew,
                "ISO 2022 IR 144" => CodeSet::Cyrillic,
                "ISO 2022 IR 148" => CodeSet::Turkish,
                "ISO 2022 IR 149" => CodeSet::EucKr,
                "ISO 2022 IR 159" => CodeSet::JisX0212,
                "ISO 2022 IR 166" => CodeSet::Thai,
                "ISO 2022 IR 58" | "ISO 2022 58" => CodeSet::Gb2312,
                "ISO 2022 GBK" if names.len() == 1 => return Ok(Self::Gbk),
                _ => return Err(Error::UnsupportedCharacterSet(original.to_owned())),
            };
            sets.push(set);
        }
        let config = Iso2022Config::new(&sets)
            .ok_or_else(|| Error::UnsupportedCharacterSet(original.to_owned()))?;
        Ok(Self::Iso2022(config))
    }

    /// Resolve `(0008,0005)` from its element.
    ///
    /// The whole value is read rather than `Element::string`, which would split a multi-valued
    /// code-extension declaration on its backslash.
    pub(crate) fn from_element(element: &Element) -> Result<Self> {
        let bytes = element.raw_value();
        let mut end = bytes.len();
        while end > 0 && (bytes[end - 1] == b' ' || bytes[end - 1] == 0) {
            end -= 1;
        }
        let text = std::str::from_utf8(&bytes[..end]).map_err(Error::InvalidUtf8)?;
        Self::from_specific_character_set(text)
    }

    /// The DICOM names of this character set, as declared in `(0008,0005)`.
    ///
    /// `Latin1` serves both ISO_IR 6 (the default repertoire) and ISO_IR 100; it reports
    /// ISO_IR 100, the declaration that names the encoding explicitly. An ISO 2022 declaration
    /// returns the declared values joined by backslashes.
    pub fn name(self) -> Cow<'static, str> {
        match self {
            Self::Latin1 => Cow::Borrowed("ISO_IR 100"),
            Self::Utf8 => Cow::Borrowed("ISO_IR 192"),
            Self::Iso8859_2 => Cow::Borrowed("ISO_IR 101"),
            Self::Iso8859_3 => Cow::Borrowed("ISO_IR 109"),
            Self::Iso8859_4 => Cow::Borrowed("ISO_IR 110"),
            Self::Iso8859_5 => Cow::Borrowed("ISO_IR 144"),
            Self::Iso8859_6 => Cow::Borrowed("ISO_IR 127"),
            Self::Iso8859_7 => Cow::Borrowed("ISO_IR 126"),
            Self::Iso8859_8 => Cow::Borrowed("ISO_IR 138"),
            Self::Iso8859_9 => Cow::Borrowed("ISO_IR 148"),
            Self::Thai => Cow::Borrowed("ISO_IR 166"),
            Self::ShiftJis => Cow::Borrowed("ISO_IR 13"),
            Self::Gbk => Cow::Borrowed("GBK"),
            Self::Gb18030 => Cow::Borrowed("GB18030"),
            Self::Iso2022(config) => Cow::Owned(config.names()),
        }
    }

    /// Decode the bytes of one value with this character set.
    pub fn decode(self, bytes: &[u8]) -> Result<Cow<'_, str>> {
        self.decode_with_delimiters(bytes, &[])
    }

    /// Decode one value, resetting single-byte ISO 2022 fragments to the default repertoire at
    /// the first delimiter (PN uses `^` and `=`; other textual VRs use the control characters
    /// pydicom lists in `TEXT_VR_DELIMS`).
    pub(crate) fn decode_with_delimiters<'a>(
        self,
        bytes: &'a [u8],
        delimiters: &[u8],
    ) -> Result<Cow<'a, str>> {
        match self {
            Self::Iso2022(config) => Ok(Cow::Owned(config.decode(bytes, delimiters, self)?)),
            Self::Latin1 => Ok(Cow::Owned(latin1(bytes))),
            Self::Utf8 => std::str::from_utf8(bytes)
                .map(Cow::Borrowed)
                .map_err(Error::InvalidUtf8),
            Self::Iso8859_2 => decode_bytes(self, encoding_rs::ISO_8859_2, bytes).map(Cow::Owned),
            Self::Iso8859_3 => decode_bytes(self, encoding_rs::ISO_8859_3, bytes).map(Cow::Owned),
            Self::Iso8859_4 => decode_bytes(self, encoding_rs::ISO_8859_4, bytes).map(Cow::Owned),
            Self::Iso8859_5 => decode_bytes(self, encoding_rs::ISO_8859_5, bytes).map(Cow::Owned),
            Self::Iso8859_6 => decode_bytes(self, encoding_rs::ISO_8859_6, bytes).map(Cow::Owned),
            Self::Iso8859_7 => decode_bytes(self, encoding_rs::ISO_8859_7, bytes).map(Cow::Owned),
            Self::Iso8859_8 => decode_bytes(self, encoding_rs::ISO_8859_8, bytes).map(Cow::Owned),
            Self::Iso8859_9 => decode_bytes(self, encoding_rs::WINDOWS_1254, bytes).map(Cow::Owned),
            Self::Thai => decode_bytes(self, encoding_rs::WINDOWS_874, bytes).map(Cow::Owned),
            Self::ShiftJis => decode_bytes(self, encoding_rs::SHIFT_JIS, bytes).map(Cow::Owned),
            Self::Gbk => decode_bytes(self, encoding_rs::GBK, bytes).map(Cow::Owned),
            Self::Gb18030 => decode_bytes(self, encoding_rs::GB18030, bytes).map(Cow::Owned),
        }
    }
}

impl fmt::Display for Charset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name().as_ref())
    }
}

/// One character set of an ISO 2022 code extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodeSet {
    Ascii,
    Latin1,
    Latin2,
    Latin3,
    Latin4,
    Greek,
    Arabic,
    Hebrew,
    Cyrillic,
    Turkish,
    Thai,
    JisX0201,
    JisX0208,
    JisX0212,
    EucKr,
    Gb2312,
}

impl CodeSet {
    fn name(self) -> &'static str {
        match self {
            Self::Ascii => "ISO 2022 IR 6",
            Self::Latin1 => "ISO 2022 IR 100",
            Self::Latin2 => "ISO 2022 IR 101",
            Self::Latin3 => "ISO 2022 IR 109",
            Self::Latin4 => "ISO 2022 IR 110",
            Self::Greek => "ISO 2022 IR 126",
            Self::Arabic => "ISO 2022 IR 127",
            Self::Hebrew => "ISO 2022 IR 138",
            Self::Cyrillic => "ISO 2022 IR 144",
            Self::Turkish => "ISO 2022 IR 148",
            Self::Thai => "ISO 2022 IR 166",
            Self::JisX0201 => "ISO 2022 IR 13",
            Self::JisX0208 => "ISO 2022 IR 87",
            Self::JisX0212 => "ISO 2022 IR 159",
            Self::EucKr => "ISO 2022 IR 149",
            Self::Gb2312 => "ISO 2022 IR 58",
        }
    }

    fn decode(self, bytes: &[u8], charset: Charset) -> Result<String> {
        match self {
            // ISO_IR 6 is the 94-character default repertoire; pydicom decodes it as ISO-8859-1
            // (`default_encoding = "iso8859"`), so bytes beyond ASCII read as Latin-1.
            Self::Ascii | Self::Latin1 => Ok(latin1(bytes)),
            Self::Latin2 => decode_bytes(charset, encoding_rs::ISO_8859_2, bytes),
            Self::Latin3 => decode_bytes(charset, encoding_rs::ISO_8859_3, bytes),
            Self::Latin4 => decode_bytes(charset, encoding_rs::ISO_8859_4, bytes),
            Self::Greek => decode_bytes(charset, encoding_rs::ISO_8859_7, bytes),
            Self::Arabic => decode_bytes(charset, encoding_rs::ISO_8859_6, bytes),
            Self::Hebrew => decode_bytes(charset, encoding_rs::ISO_8859_8, bytes),
            Self::Cyrillic => decode_bytes(charset, encoding_rs::ISO_8859_5, bytes),
            Self::Turkish => decode_bytes(charset, encoding_rs::WINDOWS_1254, bytes),
            Self::Thai => decode_bytes(charset, encoding_rs::WINDOWS_874, bytes),
            Self::JisX0201 => decode_bytes(charset, encoding_rs::SHIFT_JIS, bytes),
            Self::JisX0208 => decode_jis0208(bytes, charset),
            Self::JisX0212 => decode_jis0212(bytes, charset),
            // GB2312 bytes are a subset of GBK, which encoding_rs decodes.
            Self::Gb2312 => decode_bytes(charset, encoding_rs::GBK, bytes),
            Self::EucKr => decode_bytes(charset, encoding_rs::EUC_KR, bytes),
        }
    }
}

const ESC: u8 = 0x1B;

/// Map one escape sequence at the start of `fragment` to the set it designates and the
/// sequence length (three or four bytes). See PS3.3 C.12.1.1.3, tables C.12-3 and C.12-4.
fn parse_escape(fragment: &[u8]) -> Option<(CodeSet, usize)> {
    match fragment {
        [ESC, b'(', b'B', ..] => Some((CodeSet::Ascii, 3)),
        [ESC, b'(', b'J', ..] => Some((CodeSet::JisX0201, 3)),
        [ESC, b')', b'I', ..] => Some((CodeSet::JisX0201, 3)),
        [ESC, b'-', b'A', ..] => Some((CodeSet::Latin1, 3)),
        [ESC, b'-', b'B', ..] => Some((CodeSet::Latin2, 3)),
        [ESC, b'-', b'C', ..] => Some((CodeSet::Latin3, 3)),
        [ESC, b'-', b'D', ..] => Some((CodeSet::Latin4, 3)),
        [ESC, b'-', b'F', ..] => Some((CodeSet::Greek, 3)),
        [ESC, b'-', b'G', ..] => Some((CodeSet::Arabic, 3)),
        [ESC, b'-', b'H', ..] => Some((CodeSet::Hebrew, 3)),
        [ESC, b'-', b'L', ..] => Some((CodeSet::Cyrillic, 3)),
        [ESC, b'-', b'M', ..] => Some((CodeSet::Turkish, 3)),
        [ESC, b'-', b'T', ..] => Some((CodeSet::Thai, 3)),
        [ESC, b'$', b'B', ..] | [ESC, b'$', b'@', ..] => Some((CodeSet::JisX0208, 3)),
        [ESC, b'$', b'(', b'D', ..] => Some((CodeSet::JisX0212, 4)),
        [ESC, b'$', b')', b'A', ..] => Some((CodeSet::Gb2312, 4)),
        [ESC, b'$', b')', b'C', ..] => Some((CodeSet::EucKr, 4)),
        _ => None,
    }
}

/// Decode a JIS X 0208 fragment. `encoding_rs`'s ISO-2022-JP decoder expects the escape
/// sequence in the input, so it is passed through and the return to ASCII is appended.
fn decode_jis0208(fragment: &[u8], charset: Charset) -> Result<String> {
    let mut data = Vec::with_capacity(fragment.len() + 3);
    if fragment.starts_with(&[ESC, b'$']) {
        data.extend_from_slice(fragment);
    } else {
        data.extend_from_slice(&[ESC, b'$', b'B']);
        data.extend_from_slice(fragment);
    }
    data.extend_from_slice(&[ESC, b'(', b'B']);
    let (text, had_errors) = encoding_rs::ISO_2022_JP.decode_without_bom_handling(&data);
    if had_errors {
        return Err(Error::InvalidText(charset.name().into_owned()));
    }
    Ok(text.into_owned())
}

/// Decode JIS X 0212 bytes with the generated table (`tools/gen_jis0212.py`).
fn decode_jis0212(bytes: &[u8], charset: Charset) -> Result<String> {
    let mut out = String::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let code = u16::from(pair[0]) << 8 | u16::from(pair[1]);
        match jis0212::JIS_X_0212.binary_search_by_key(&code, |(value, _)| *value) {
            Ok(index) => out.push(jis0212::JIS_X_0212[index].1),
            Err(_) => return Err(Error::InvalidText(charset.name().into_owned())),
        }
    }
    if bytes.len() % 2 != 0 {
        return Err(Error::InvalidText(charset.name().into_owned()));
    }
    Ok(out)
}

/// ISO-8859-1: every byte maps to the code point of the same number.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

fn decode_bytes(charset: Charset, encoding: &'static Encoding, bytes: &[u8]) -> Result<String> {
    let (text, had_errors) = encoding.decode_without_bom_handling(bytes);
    if had_errors {
        // Report the DICOM name rather than the backend encoding's name, so the message reads
        // like the declaration in the file.
        return Err(Error::InvalidText(charset.name().into_owned()));
    }
    Ok(text.into_owned())
}
