//! RLE Lossless decoding (PS3.5 Annex G).

use super::{PixelDescriptor, PlanarConfiguration};
use crate::error::{Error, Result};

/// Length of the RLE frame header: the segment count and fifteen segment offsets.
const HEADER_LEN: usize = 64;

/// Decode one encapsulated RLE frame into native little-endian pixel bytes.
pub(super) fn decode(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
    if frame.len() < HEADER_LEN {
        return Err(Error::InvalidPixelData(format!(
            "RLE frame is {} bytes, header requires {HEADER_LEN}",
            frame.len()
        )));
    }
    let segment_count = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    let bytes_per_sample = match descriptor.bits_allocated {
        8 => 1,
        16 => 2,
        32 => 4,
        bits => {
            return Err(Error::InvalidPixelData(format!(
                "RLE does not support Bits Allocated {bits}"
            )));
        }
    };
    let samples = usize::from(descriptor.samples_per_pixel);
    let expected_segments = samples * bytes_per_sample;
    if segment_count != expected_segments {
        return Err(Error::InvalidPixelData(format!(
            "RLE frame has {segment_count} segments, expected {expected_segments}"
        )));
    }
    if segment_count == 0 || segment_count > 15 {
        return Err(Error::InvalidPixelData(format!(
            "RLE frame has an invalid segment count: {segment_count}"
        )));
    }

    let pixel_count = usize::from(descriptor.rows) * usize::from(descriptor.columns);
    let mut offsets = Vec::with_capacity(segment_count);
    for index in 0..segment_count {
        let start = 4 + index * 4;
        let offset = u32::from_le_bytes([
            frame[start],
            frame[start + 1],
            frame[start + 2],
            frame[start + 3],
        ]) as usize;
        if offset < HEADER_LEN || offset >= frame.len() {
            return Err(Error::InvalidPixelData(format!(
                "RLE segment {index} starts at {offset}, outside the frame"
            )));
        }
        offsets.push(offset);
    }
    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(Error::InvalidPixelData(
            "RLE segment offsets are not ascending".to_owned(),
        ));
    }

    let mut planes = Vec::with_capacity(segment_count);
    for index in 0..segment_count {
        let start = offsets[index];
        let end = offsets.get(index + 1).copied().unwrap_or(frame.len());
        planes.push(decode_segment(&frame[start..end], pixel_count)?);
    }
    Ok(assemble(
        &planes,
        descriptor,
        samples,
        bytes_per_sample,
        pixel_count,
    ))
}

/// Decode one PackBits-style segment to exactly `expected` bytes.
///
/// Control byte `n`: `0..=127` copies `n + 1` literal bytes, `128` is a no-op, and `129..=255`
/// repeats the next byte `257 - n` times. Segments are padded to an even length, so bytes past
/// `expected` are ignored; a run may overshoot by at most one padding byte.
fn decode_segment(bytes: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    let mut cursor = 0;
    while cursor < bytes.len() && out.len() < expected + 2 {
        let control = bytes[cursor];
        cursor += 1;
        match control {
            0..=127 => {
                let count = usize::from(control) + 1;
                let end = cursor + count;
                if end > bytes.len() {
                    return Err(Error::InvalidPixelData(
                        "RLE segment ends inside a literal run".to_owned(),
                    ));
                }
                out.extend_from_slice(&bytes[cursor..end]);
                cursor = end;
            }
            128 => {}
            _ => {
                let count = 257 - usize::from(control);
                let Some(&value) = bytes.get(cursor) else {
                    return Err(Error::InvalidPixelData(
                        "RLE segment ends before a repeat value".to_owned(),
                    ));
                };
                cursor += 1;
                let new_len = out.len() + count;
                out.resize(new_len, value);
            }
        }
    }
    if out.len() < expected {
        return Err(Error::InvalidPixelData(format!(
            "RLE segment decoded {} bytes, expected {expected}",
            out.len()
        )));
    }
    out.truncate(expected);
    Ok(out)
}

/// Interleave or de-interleave the decoded segment planes into native pixel bytes.
///
/// Segment planes are ordered sample-major, most significant byte first; the output uses
/// little-endian samples per the native pixel data layout.
fn assemble(
    planes: &[Vec<u8>],
    descriptor: &PixelDescriptor,
    samples: usize,
    bytes_per_sample: usize,
    pixel_count: usize,
) -> Vec<u8> {
    let mut out = vec![0u8; pixel_count * samples * bytes_per_sample];
    let planar = descriptor.planar_configuration == PlanarConfiguration::Planar;
    for sample in 0..samples {
        for plane in 0..bytes_per_sample {
            let data = &planes[sample * bytes_per_sample + plane];
            // Plane 0 holds the most significant byte; native little-endian stores it last.
            let byte_offset = bytes_per_sample - 1 - plane;
            for (pixel, &value) in data.iter().enumerate() {
                let index = if planar {
                    (sample * pixel_count + pixel) * bytes_per_sample
                } else {
                    (pixel * samples + sample) * bytes_per_sample
                };
                out[index + byte_offset] = value;
            }
        }
    }
    out
}
