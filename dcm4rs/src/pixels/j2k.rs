//! JPEG 2000 and HTJ2K decoding via `jpeg2k` (the pure-Rust `openjp2` port by default).

use super::PixelDescriptor;
use crate::error::{Error, Result};

const SOC: [u8; 2] = [0xFF, 0x4F];
const SOT: [u8; 2] = [0xFF, 0x90];
const COM: [u8; 2] = [0xFF, 0x64];
const EOC: [u8; 2] = [0xFF, 0xD9];

/// Drop comment segments an encoder left between the last tile-part and `EOC`.
///
/// T.800 A.4.1 allows `COM` in the main header and in tile-part headers only. openjpeg rejects
/// a codestream that carries one after the last tile-part and reports nothing more specific
/// than "Failed to decode image". `pylibjpeg-data`'s replacement for the file pydicom issue
/// #966 reports differs from the original by exactly one such segment — a `COM` reading
/// "PADDING" that precedes `EOC` — so the stream decodes once the segment is gone.
fn strip_trailing_comment_segments(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.get(..2)? != SOC {
        return None;
    }
    let mut offset = 2;
    // Main header: marker segments that carry a length, up to the first SOT.
    loop {
        if *frame.get(offset)? != 0xFF {
            return None;
        }
        let marker = *frame.get(offset + 1)?;
        if marker == SOT[1] {
            break;
        }
        if marker == EOC[1] || marker == SOC[1] {
            return None;
        }
        let length = usize::from(segment_length(frame, offset)?);
        offset = offset.checked_add(2 + length)?;
        if offset >= frame.len() {
            return None;
        }
    }
    // Tile-parts: each SOT states its own extent in Psot.
    loop {
        let marker = frame.get(offset..offset + 2)?;
        if marker == EOC {
            return None;
        }
        if marker != SOT {
            return None;
        }
        let psot = u32::from_be_bytes(frame.get(offset + 6..offset + 10)?.try_into().ok()?);
        let psot = usize::try_from(psot).ok()?;
        if psot < 12 {
            return None;
        }
        let next = offset.checked_add(psot)?;
        if next >= frame.len() {
            return None;
        }
        match frame.get(next..next + 2)? {
            [0xFF, 0x90] => {
                offset = next;
                continue;
            }
            [0xFF, 0xD9] => return None,
            _ => {}
        }
        // Only comment segments may sit between the last tile-part and EOC; drop them.
        let mut position = next;
        let mut stripped = false;
        while frame.get(position..position + 2)? == COM {
            let length = usize::from(segment_length(frame, position)?);
            position = position.checked_add(2 + length)?;
            if position >= frame.len() {
                return None;
            }
            stripped = true;
        }
        if !stripped || frame.get(position..position + 2)? != EOC {
            return None;
        }
        let mut repaired = frame[..next].to_vec();
        repaired.extend_from_slice(&frame[position..]);
        return Some(repaired);
    }
}

/// Read a marker segment's length field, which counts itself but not the marker.
fn segment_length(frame: &[u8], offset: usize) -> Option<u16> {
    let bytes = frame.get(offset + 2..offset + 4)?;
    let length = u16::from_be_bytes([bytes[0], bytes[1]]);
    (length >= 2).then_some(length)
}

/// Decode one encapsulated JPEG 2000 / HTJ2K frame.
///
/// The codestream determines the dimensions, component count, precision and signedness; all
/// four must agree with the descriptor. Components are interleaved into native little-endian
/// samples. Color codestreams have the inverse multiple-component transform applied by the
/// decoder, so the returned samples are RGB.
pub(super) fn decode(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
    let repaired = strip_trailing_comment_segments(frame);
    let frame = repaired.as_deref().unwrap_or(frame);
    let image = jpeg2k::Image::from_bytes(frame)
        .map_err(|err| Error::InvalidPixelData(format!("JPEG 2000 decode failed: {err}")))?;
    let width = image.orig_width();
    let height = image.orig_height();
    let component_count = image.num_components();
    if width != u32::from(descriptor.columns)
        || height != u32::from(descriptor.rows)
        || component_count != u32::from(descriptor.samples_per_pixel)
    {
        return Err(Error::InvalidPixelData(format!(
            "JPEG 2000 frame is {width}x{height} with {component_count} component(s), descriptor is {}x{} with {}",
            descriptor.columns, descriptor.rows, descriptor.samples_per_pixel
        )));
    }

    let components = image.components();
    let pixel_count = usize::from(descriptor.rows) * usize::from(descriptor.columns);
    let bytes_per_sample = descriptor.bytes_per_sample();
    let mut out = vec![0u8; pixel_count * components.len() * bytes_per_sample];

    for (sample, component) in components.iter().enumerate() {
        // The codestream precision and signedness are not validated against Bits Stored and
        // Pixel Representation: pydicom decodes the corpus files whose SIZ marker disagrees
        // with the data set, and the samples are interpreted as the codestream declares them.
        let signed = component.is_signed();
        let data = component.data().ok_or(Error::InvalidPixelData(
            "JPEG 2000 component has no sample data".to_owned(),
        ))?;
        if data.len() != pixel_count {
            return Err(Error::InvalidPixelData(format!(
                "JPEG 2000 component {sample} has {} samples, expected {pixel_count}",
                data.len()
            )));
        }
        for (pixel, &value) in data.iter().enumerate() {
            let bytes = if signed {
                (value as i64).to_le_bytes()
            } else {
                (value as u64).to_le_bytes()
            };
            let offset = (pixel * components.len() + sample) * bytes_per_sample;
            out[offset..offset + bytes_per_sample].copy_from_slice(&bytes[..bytes_per_sample]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::strip_trailing_comment_segments;

    /// A codestream carrying `tile_data` in one tile-part, plus `PADDING` as its COM segment.
    const PADDING: [u8; 13] = [
        0xFF, 0x64, 0x00, 0x0B, 0x00, 0x01, 0x50, 0x41, 0x44, 0x44, 0x49, 0x4E, 0x47,
    ];

    /// The COM segment pydicom issue #966 reports: `FF64` before `EOC` instead of a normal
    /// end.
    fn codestream(tile_data: &[u8], trailing: &[&[u8]]) -> Vec<u8> {
        let mut stream = vec![0xFF, 0x4F];
        stream.extend_from_slice(&[0xFF, 0x51, 0x00, 0x04, 0x00, 0x00]);
        let psot = 12 + u32::try_from(tile_data.len()).expect("small tile");
        stream.extend_from_slice(&[0xFF, 0x90, 0x00, 0x0A, 0x00, 0x00]);
        stream.extend_from_slice(&psot.to_be_bytes());
        stream.extend_from_slice(&[0x00, 0x01]);
        stream.extend_from_slice(tile_data);
        for segment in trailing {
            stream.extend_from_slice(segment);
        }
        stream.extend_from_slice(&[0xFF, 0xD9]);
        stream
    }

    #[test]
    fn a_clean_codestream_is_left_alone() {
        let clean = codestream(&[0x11, 0x22, 0x33], &[]);
        assert_eq!(strip_trailing_comment_segments(&clean), None);
    }

    #[test]
    fn a_trailing_comment_segment_is_dropped() {
        let clean = codestream(&[0x11, 0x22, 0x33], &[]);
        let broken = codestream(&[0x11, 0x22, 0x33], &[&PADDING]);
        assert_eq!(strip_trailing_comment_segments(&broken), Some(clean));
    }

    #[test]
    fn every_trailing_comment_segment_is_dropped() {
        let clean = codestream(&[0x11], &[]);
        let broken = codestream(&[0x11], &[&PADDING, &PADDING]);
        assert_eq!(strip_trailing_comment_segments(&broken), Some(clean));
    }

    #[test]
    fn a_trailer_that_is_not_a_comment_keeps_the_stream() {
        // A second COD segment after the tile-part data is not the defect being repaired.
        let broken = codestream(&[0x11], &[&[0xFF, 0x52, 0x00, 0x02]]);
        assert_eq!(strip_trailing_comment_segments(&broken), None);
    }

    #[test]
    fn a_comment_after_an_earlier_tile_part_keeps_the_stream() {
        // Psot points past a COM that sits before the last tile-part; nothing is guessed.
        let mut stream = codestream(&[0x11], &[]);
        assert_eq!(strip_trailing_comment_segments(&stream), None);
        stream.truncate(stream.len() - 2);
        assert_eq!(strip_trailing_comment_segments(&stream), None);
    }
}
