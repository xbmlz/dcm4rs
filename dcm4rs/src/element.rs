use std::borrow::Cow;

use crate::charset::Charset;
use crate::dataset::DataSet;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::tag::Tag;
use crate::value::Value;
use crate::vr::Vr;

/// A DICOM data element: tag, VR and value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    /// The tag.
    pub tag: Tag,
    /// The value representation.
    pub vr: Vr,
    /// The value.
    pub value: Value,
}

impl Element {
    /// Construct an element. VR/value consistency is not checked here; checking happens at read
    /// time.
    pub fn new(tag: Tag, vr: Vr, value: Value) -> Self {
        Self { tag, vr, value }
    }

    /// Whether this is a sequence element.
    pub fn is_sequence(&self) -> bool {
        self.vr.is_sequence()
    }

    /// Return a copy whose numeric value bytes are converted from `from` to `to`.
    pub(crate) fn transcode(&self, from: Endian, to: Endian) -> Result<Self> {
        let value = match &self.value {
            Value::Sequence(items) => Value::Sequence(
                items
                    .iter()
                    .map(|item| item.to_endian(to))
                    .collect::<Result<Vec<_>>>()?,
            ),
            Value::Bytes(bytes) if from != to => {
                Value::from_bytes(transcode_bytes(bytes, self.vr)?)
            }
            _ => self.value.clone(),
        };
        Ok(Self {
            tag: self.tag,
            vr: self.vr,
            value,
        })
    }

    /// Byte length of the value; sequences return `0`.
    pub fn byte_len(&self) -> usize {
        self.value.as_bytes().map_or(0, <[u8]>::len)
    }

    /// Whether the value is empty.
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Raw bytes; an empty slice for non-byte values.
    pub fn raw_value(&self) -> &[u8] {
        self.value.as_bytes().unwrap_or(&[])
    }

    /// Sequence items.
    pub fn as_sequence(&self) -> Result<&[DataSet]> {
        if !self.vr.is_sequence() {
            return Err(Error::NotASequence(self.vr));
        }
        Ok(self.value.as_sequence().unwrap_or(&[]))
    }

    /// Mutable sequence items; an empty value is first promoted to an empty sequence.
    pub fn as_sequence_mut(&mut self) -> Result<&mut Vec<DataSet>> {
        if !self.vr.is_sequence() {
            return Err(Error::NotASequence(self.vr));
        }
        if !self.value.is_sequence() {
            self.value = Value::Sequence(Vec::new());
        }
        self.value
            .as_sequence_mut()
            .ok_or(Error::NotASequence(self.vr))
    }

    /// String values split on backslashes, with trailing padding removed; textual VRs only.
    ///
    /// The bytes are decoded as UTF-8. An element does not know its data set, so for values that
    /// may use another character set read through [`DataSet::strings`](crate::DataSet::strings),
    /// which applies the `(0008,0005)` declared encoding, or call [`Element::string_values_with`].
    pub fn string_values(&self) -> Result<Vec<&str>> {
        if !self.vr.is_string() {
            return Err(Error::WrongVr {
                actual: self.vr,
                expected: "textual VR",
            });
        }
        let bytes = self.raw_value();
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        if self.vr.splits_on_backslash() {
            bytes
                .split(|byte| *byte == b'\\')
                .map(|part| decode_text(part, self.vr))
                .collect()
        } else {
            Ok(vec![decode_text(bytes, self.vr)?])
        }
    }

    /// String values decoded with an explicit character set, see [`Self::string_values`].
    pub fn string_values_with(&self, charset: Charset) -> Result<Vec<Cow<'_, str>>> {
        if !self.vr.is_string() {
            return Err(Error::WrongVr {
                actual: self.vr,
                expected: "textual VR",
            });
        }
        let bytes = self.raw_value();
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        if self.vr.splits_on_backslash() {
            bytes
                .split(|byte| *byte == b'\\')
                .map(|part| decode_text_with(part, self.vr, charset))
                .collect()
        } else {
            Ok(vec![decode_text_with(bytes, self.vr, charset)?])
        }
    }

    /// The first string value; an empty value returns an empty string.
    pub fn string(&self) -> Result<&str> {
        Ok(self.string_values()?.first().copied().unwrap_or(""))
    }

    /// The first string value decoded with an explicit character set; an empty value returns an
    /// empty string.
    pub fn string_with(&self, charset: Charset) -> Result<Cow<'_, str>> {
        Ok(self
            .string_values_with(charset)?
            .into_iter()
            .next()
            .unwrap_or(Cow::Borrowed("")))
    }

    /// Decode 16-bit integers for `US`/`SS`/`UN`.
    pub fn i16s(&self, endian: Endian) -> Result<Vec<i16>> {
        self.expect_vr(&[Vr::Us, Vr::Ss, Vr::Un], "US/SS")?;
        let bytes = self.raw_value();
        check_unit(bytes.len(), 2, self.vr)?;
        Ok(bytes
            .chunks_exact(2)
            .map(|chunk| endian.i16([chunk[0], chunk[1]]))
            .collect())
    }

    /// Decode 16-bit unsigned integers for `US`/`SS`/`UN`.
    pub fn u16s(&self, endian: Endian) -> Result<Vec<u16>> {
        self.expect_vr(&[Vr::Us, Vr::Ss, Vr::Un], "US/SS")?;
        let bytes = self.raw_value();
        check_unit(bytes.len(), 2, self.vr)?;
        Ok(bytes
            .chunks_exact(2)
            .map(|chunk| endian.u16([chunk[0], chunk[1]]))
            .collect())
    }

    /// Decode 32-bit integers for `UL`/`SL`/`UN`.
    pub fn i32s(&self, endian: Endian) -> Result<Vec<i32>> {
        self.expect_vr(&[Vr::Ul, Vr::Sl, Vr::Un], "UL/SL")?;
        let bytes = self.raw_value();
        check_unit(bytes.len(), 4, self.vr)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|chunk| endian.i32([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect())
    }

    /// Decode 32-bit unsigned integers for `UL`/`SL`/`UN`.
    pub fn u32s(&self, endian: Endian) -> Result<Vec<u32>> {
        self.expect_vr(&[Vr::Ul, Vr::Sl, Vr::Un], "UL/SL")?;
        let bytes = self.raw_value();
        check_unit(bytes.len(), 4, self.vr)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|chunk| endian.u32([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect())
    }

    /// Decode 64-bit integers for `UV`/`SV`/`UN`.
    pub fn i64s(&self, endian: Endian) -> Result<Vec<i64>> {
        self.expect_vr(&[Vr::Uv, Vr::Sv, Vr::Un], "UV/SV")?;
        let bytes = self.raw_value();
        check_unit(bytes.len(), 8, self.vr)?;
        Ok(bytes
            .chunks_exact(8)
            .map(|chunk| endian.i64(chunk.try_into().expect("8-byte slice")))
            .collect())
    }

    /// Decode 64-bit unsigned integers for `UV`/`SV`/`UN`.
    pub fn u64s(&self, endian: Endian) -> Result<Vec<u64>> {
        self.expect_vr(&[Vr::Uv, Vr::Sv, Vr::Un], "UV/SV")?;
        let bytes = self.raw_value();
        check_unit(bytes.len(), 8, self.vr)?;
        Ok(bytes
            .chunks_exact(8)
            .map(|chunk| endian.u64(chunk.try_into().expect("8-byte slice")))
            .collect())
    }

    /// Decode single-precision floats for `FL`/`UN`.
    pub fn f32s(&self, endian: Endian) -> Result<Vec<f32>> {
        self.expect_vr(&[Vr::Fl, Vr::Un], "FL")?;
        let bytes = self.raw_value();
        check_unit(bytes.len(), 4, self.vr)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|chunk| endian.f32([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect())
    }

    /// Decode double-precision floats for `FD`/`UN`.
    pub fn f64s(&self, endian: Endian) -> Result<Vec<f64>> {
        self.expect_vr(&[Vr::Fd, Vr::Un], "FD")?;
        let bytes = self.raw_value();
        check_unit(bytes.len(), 8, self.vr)?;
        Ok(bytes
            .chunks_exact(8)
            .map(|chunk| endian.f64(chunk.try_into().expect("8-byte slice")))
            .collect())
    }

    /// Decode attribute tags for `AT`/`UN`.
    pub fn tags(&self, endian: Endian) -> Result<Vec<Tag>> {
        self.expect_vr(&[Vr::At, Vr::Un], "AT")?;
        let bytes = self.raw_value();
        check_unit(bytes.len(), 4, self.vr)?;
        Ok(bytes
            .chunks_exact(4)
            .map(|chunk| {
                Tag::new(
                    endian.u16([chunk[0], chunk[1]]),
                    endian.u16([chunk[2], chunk[3]]),
                )
            })
            .collect())
    }

    /// Parse `DS` text into a float sequence; empty components are skipped as "no value".
    pub fn decimal_values(&self) -> Result<Vec<f64>> {
        if self.vr != Vr::Ds {
            return Err(Error::WrongVr {
                actual: self.vr,
                expected: "DS",
            });
        }
        let mut values = Vec::new();
        for part in self.string_values()? {
            let text = part.trim();
            if text.is_empty() {
                continue;
            }
            values.push(
                text.parse::<f64>()
                    .map_err(|_| Error::InvalidNumber(text.to_owned()))?,
            );
        }
        Ok(values)
    }

    /// Parse `IS` text into an integer sequence; empty components are skipped as "no value".
    pub fn integer_values(&self) -> Result<Vec<i64>> {
        if self.vr != Vr::Is {
            return Err(Error::WrongVr {
                actual: self.vr,
                expected: "IS",
            });
        }
        let mut values = Vec::new();
        for part in self.string_values()? {
            let text = part.trim();
            if text.is_empty() {
                continue;
            }
            values.push(
                text.parse::<i64>()
                    .map_err(|_| Error::InvalidNumber(text.to_owned()))?,
            );
        }
        Ok(values)
    }

    fn expect_vr(&self, allowed: &[Vr], expected: &'static str) -> Result<()> {
        if allowed.contains(&self.vr) {
            Ok(())
        } else {
            Err(Error::WrongVr {
                actual: self.vr,
                expected,
            })
        }
    }
}

/// Byte width of one numeric unit, or `None` for values that do not depend on byte order.
fn transcode_unit(vr: Vr) -> Option<usize> {
    match vr {
        Vr::Us | Vr::Ss | Vr::Ow => Some(2),
        Vr::Ul | Vr::Sl | Vr::Fl | Vr::At | Vr::Of | Vr::Ol => Some(4),
        Vr::Fd | Vr::Sv | Vr::Uv | Vr::Ov | Vr::Od => Some(8),
        _ => None,
    }
}

fn transcode_bytes(bytes: &[u8], vr: Vr) -> Result<Vec<u8>> {
    let Some(unit) = transcode_unit(vr) else {
        return Ok(bytes.to_vec());
    };
    if bytes.len() % unit != 0 {
        return Err(Error::WrongLength {
            vr,
            len: bytes.len(),
            unit,
        });
    }
    let mut out = bytes.to_vec();
    for chunk in out.chunks_exact_mut(unit) {
        if vr == Vr::At {
            chunk.swap(0, 1);
            chunk.swap(2, 3);
        } else {
            chunk.reverse();
        }
    }
    Ok(out)
}

fn check_unit(len: usize, unit: usize, vr: Vr) -> Result<()> {
    if len % unit == 0 {
        Ok(())
    } else {
        Err(Error::WrongLength { vr, len, unit })
    }
}

fn decode_text(part: &[u8], vr: Vr) -> Result<&str> {
    std::str::from_utf8(trim_padding(part, vr)).map_err(Error::InvalidUtf8)
}

/// Delimiters that reset an ISO 2022 single-byte fragment to the default repertoire.
/// `PN` uses the component and value separators; other textual VRs use the control characters
/// pydicom lists in `TEXT_VR_DELIMS` (pydicom/valuerep.py:27).
const PN_DELIMITERS: &[u8] = b"^=";
const TEXT_DELIMITERS: &[u8] = &[0x0D, 0x0A, 0x09, 0x0C];

fn decode_text_with(part: &[u8], vr: Vr, charset: Charset) -> Result<Cow<'_, str>> {
    let delimiters = if vr == Vr::Pn {
        PN_DELIMITERS
    } else {
        TEXT_DELIMITERS
    };
    charset.decode_with_delimiters(trim_padding(part, vr), delimiters)
}

/// Drop the trailing padding (spaces and the VR's pad byte) before decoding.
fn trim_padding(part: &[u8], vr: Vr) -> &[u8] {
    let mut end = part.len();
    while end > 0 && (part[end - 1] == b' ' || part[end - 1] == vr.padding()) {
        end -= 1;
    }
    &part[..end]
}
