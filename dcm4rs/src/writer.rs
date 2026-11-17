use std::io::Write;

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::element::Element;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::tag::Tag;
use crate::transfer_syntax::TransferSyntax;
use crate::value::Value;
use crate::vr::Vr;

const ITEM: Tag = Tag::new(0xFFFE, 0xE000);
const SEQUENCE_DELIMITER: Tag = Tag::new(0xFFFE, 0xE0DD);
const UNDEFINED_LENGTH: u32 = 0xFFFF_FFFF;

impl DataSet {
    /// Encode this data set with `syntax`.
    ///
    /// Values are written exactly as stored: the writer never converts byte order, so a data
    /// set read with a big-endian syntax must be written with one too. Sequences are written
    /// with defined lengths, so undefined-length sources come back canonicalized (the decoded
    /// content is identical). Group length elements (`(xxxx,0000)`) are dropped.
    pub fn write(&self, syntax: TransferSyntax) -> Result<Vec<u8>> {
        if syntax == TransferSyntax::DeflatedExplicitVrLittleEndian {
            let raw = self.write(TransferSyntax::ExplicitVrLittleEndian)?;
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&raw).map_err(|_| Error::InvalidData {
                offset: 0,
                reason: "deflate: compression failed",
            })?;
            return encoder.finish().map_err(|_| Error::InvalidData {
                offset: 0,
                reason: "deflate: compression failed",
            });
        }
        let mut corrected = self.clone();
        correct_data_set(&mut corrected, &VrContext::default(), syntax);
        let mut out = Vec::new();
        write_data_set(&corrected, syntax, &mut out)?;
        Ok(out)
    }
}

/// Context used to resolve ambiguous VRs while writing.
#[derive(Clone, Default)]
struct VrContext {
    bits_allocated: Option<u16>,
    pixel_representation: Option<u16>,
    waveform_bits_allocated: Option<u16>,
    lut_descriptor: Option<u16>,
    has_pixel_data: bool,
}

fn context_for(set: &DataSet, parent: &VrContext) -> VrContext {
    let mut context = parent.clone();
    if let Ok(value) = set.u16(tags::BITS_ALLOCATED) {
        context.bits_allocated = Some(value);
    }
    if let Ok(value) = set.u16(tags::PIXEL_REPRESENTATION) {
        context.pixel_representation = Some(value);
    }
    if let Ok(value) = set.u16(Tag::new(0x5400, 0x1004)) {
        context.waveform_bits_allocated = Some(value);
    }
    if let Ok(value) = set.u16(Tag::new(0x0028, 0x3002)) {
        context.lut_descriptor = Some(value);
    }
    context.has_pixel_data |= set.contains(tags::PIXEL_DATA);
    context
}

fn correct_data_set(set: &mut DataSet, parent: &VrContext, syntax: TransferSyntax) {
    let context = context_for(set, parent);
    for element in set.iter_mut() {
        if let Value::Sequence(items) = &mut element.value {
            for item in items {
                correct_data_set(item, &context, syntax);
            }
        } else {
            correct_element(element, &context, syntax);
        }
    }
}

fn correct_element(element: &mut Element, context: &VrContext, syntax: TransferSyntax) {
    if element.tag == tags::PIXEL_DATA {
        element.vr = if element.value.is_fragments() {
            Vr::Ob
        } else if !syntax.is_explicit_vr() {
            // PS3.5 Annex A.1: native Pixel Data under implicit VR is always OW.
            Vr::Ow
        } else if context.bits_allocated.unwrap_or(8) > 8 {
            Vr::Ow
        } else {
            Vr::Ob
        };
        return;
    }
    if is_us_ss_tag(element.tag) {
        let pixel_representation = context
            .pixel_representation
            .unwrap_or(if context.has_pixel_data { 1 } else { 0 });
        element.vr = if pixel_representation == 0 {
            Vr::Us
        } else {
            Vr::Ss
        };
        return;
    }
    if is_waveform_ob_ow_tag(element.tag) {
        element.vr = if context.waveform_bits_allocated.unwrap_or(8) > 8 {
            Vr::Ow
        } else {
            Vr::Ob
        };
        return;
    }
    if element.tag == Tag::new(0x0028, 0x3006) {
        // PS3.3 C.11.1.1.1: a single LUT value must be US, otherwise OW.
        element.vr = if context.lut_descriptor == Some(1) {
            Vr::Us
        } else {
            Vr::Ow
        };
        return;
    }
    if is_overlay_data_tag(element.tag) {
        element.vr = Vr::Ow;
    }
}

fn is_us_ss_tag(tag: Tag) -> bool {
    matches!(
        tag.as_u32(),
        0x0018_9810
            | 0x0022_1452
            | 0x0028_0104..=0x0028_0109
            | 0x0028_0110
            | 0x0028_0111
            | 0x0028_0120
            | 0x0028_0121
            | 0x0028_1101..=0x0028_1103
            | 0x0028_3002
            | 0x0040_9211
            | 0x0040_9216
            | 0x0060_3004
            | 0x0060_3006
    )
}

fn is_waveform_ob_ow_tag(tag: Tag) -> bool {
    matches!(
        tag.as_u32(),
        0x5400_0110 | 0x5400_0112 | 0x5400_100A | 0x5400_1010
    )
}

fn is_overlay_data_tag(tag: Tag) -> bool {
    let group = tag.group();
    (0x6000..=0x601E).contains(&group) && group % 2 == 0 && tag.element() == 0x3000
}

/// Append the encoded elements of `set` to `out`.
pub(crate) fn write_data_set(
    set: &DataSet,
    syntax: TransferSyntax,
    out: &mut Vec<u8>,
) -> Result<()> {
    for element in set.iter() {
        if element.tag.is_group_length() {
            continue;
        }
        write_element(element, syntax, out)?;
    }
    Ok(())
}

fn write_element(element: &Element, syntax: TransferSyntax, out: &mut Vec<u8>) -> Result<()> {
    let body = body_bytes(element, syntax)?;
    let encapsulated = element.value.is_fragments();
    let length = if encapsulated {
        UNDEFINED_LENGTH
    } else {
        body.len() as u32
    };

    write_tag(out, element.tag, syntax);
    if syntax.is_explicit_vr() {
        out.extend_from_slice(element.vr.as_str().as_bytes());
        if element.vr.uses_long_length() {
            out.extend_from_slice(&[0, 0]);
            write_u32(out, length, syntax);
        } else {
            let short = u16::try_from(body.len()).map_err(|_| Error::InvalidData {
                offset: 0,
                reason: "value too long for a short-length VR",
            })?;
            write_u16(out, short, syntax);
        }
    } else {
        write_u32(out, length, syntax);
    }
    out.extend_from_slice(&body);

    if encapsulated {
        write_tag(out, SEQUENCE_DELIMITER, syntax);
        write_u32(out, 0, syntax);
    }
    Ok(())
}

fn body_bytes(element: &Element, syntax: TransferSyntax) -> Result<Vec<u8>> {
    match &element.value {
        Value::Empty => Ok(Vec::new()),
        Value::Bytes(bytes) => {
            if element.vr.is_sequence() {
                return Err(Error::InvalidData {
                    offset: 0,
                    reason: "sequence element holding a byte value",
                });
            }
            let mut padded = bytes.clone();
            if padded.len() % 2 == 1 {
                padded.push(element.vr.padding());
            }
            Ok(padded)
        }
        Value::Sequence(items) => {
            let mut out = Vec::new();
            for item in items {
                let mut item_body = Vec::new();
                write_data_set(item, syntax, &mut item_body)?;
                write_tag(&mut out, ITEM, syntax);
                write_u32(&mut out, item_body.len() as u32, syntax);
                out.extend_from_slice(&item_body);
            }
            Ok(out)
        }
        Value::Fragments(fragments) => {
            if !matches!(element.vr, Vr::Ob | Vr::Ow) {
                return Err(Error::InvalidData {
                    offset: 0,
                    reason: "encapsulated fragments on a non-pixel-data element",
                });
            }
            let mut out = Vec::new();
            for fragment in fragments {
                // PS3.5 A.4 requires every item to have an even length, so an odd fragment is
                // padded with a zero byte and the length counts the padding, as pydicom does.
                let mut padded: Vec<u8>;
                let body: &[u8] = if fragment.len() % 2 == 1 {
                    padded = fragment.clone();
                    padded.push(0);
                    &padded
                } else {
                    fragment
                };
                write_tag(&mut out, ITEM, syntax);
                write_u32(&mut out, body.len() as u32, syntax);
                out.extend_from_slice(body);
            }
            Ok(out)
        }
    }
}

fn write_tag(out: &mut Vec<u8>, tag: Tag, syntax: TransferSyntax) {
    write_u16(out, tag.group(), syntax);
    write_u16(out, tag.element(), syntax);
}

fn write_u16(out: &mut Vec<u8>, value: u16, syntax: TransferSyntax) {
    match syntax.endian() {
        Endian::Little => out.extend_from_slice(&value.to_le_bytes()),
        Endian::Big => out.extend_from_slice(&value.to_be_bytes()),
    }
}

fn write_u32(out: &mut Vec<u8>, value: u32, syntax: TransferSyntax) {
    match syntax.endian() {
        Endian::Little => out.extend_from_slice(&value.to_le_bytes()),
        Endian::Big => out.extend_from_slice(&value.to_be_bytes()),
    }
}
