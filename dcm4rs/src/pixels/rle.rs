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
                // A literal run may be cut short by the segment boundary or by padding;
                // pydicom copies whatever bytes remain instead of reporting an error.
                let end = (cursor + count).min(bytes.len());
                out.extend_from_slice(&bytes[cursor..end]);
                cursor = end;
            }
            128 => {}
            _ => {
                let count = 257 - usize::from(control);
                let Some(&value) = bytes.get(cursor) else {
                    break;
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

/// Encode one native frame as an RLE frame (PS3.5 Annex G).
///
/// The frame holds one segment per byte plane of every sample, in the order the decoder reads
/// them: sample first, then the byte plane from the most significant to the least significant.
pub(super) fn encode(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
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
    let segments = samples * bytes_per_sample;
    if segments == 0 || segments > 15 {
        return Err(Error::InvalidPixelData(format!(
            "RLE holds one to 15 segments, this image needs {segments}"
        )));
    }
    let pixel_count = usize::from(descriptor.rows) * usize::from(descriptor.columns);
    let expected = pixel_count * samples * bytes_per_sample;
    if frame.len() < expected {
        return Err(Error::InvalidPixelData(format!(
            "frame has {} bytes, {expected} required",
            frame.len()
        )));
    }
    let planar = descriptor.planar_configuration == PlanarConfiguration::Planar;

    let mut packed = Vec::with_capacity(segments);
    for sample in 0..samples {
        for plane in 0..bytes_per_sample {
            let byte_offset = bytes_per_sample - 1 - plane;
            let mut segment = Vec::with_capacity(pixel_count);
            for pixel in 0..pixel_count {
                let index = if planar {
                    (sample * pixel_count + pixel) * bytes_per_sample
                } else {
                    (pixel * samples + sample) * bytes_per_sample
                };
                segment.push(frame[index + byte_offset]);
            }
            packed.push(pack(&segment));
        }
    }

    let mut out = vec![0u8; HEADER_LEN];
    out[0..4].copy_from_slice(&(segments as u32).to_le_bytes());
    let mut offset = HEADER_LEN as u32;
    for (index, segment) in packed.iter().enumerate() {
        out[4 + index * 4..8 + index * 4].copy_from_slice(&offset.to_le_bytes());
        offset += segment.len() as u32;
    }
    for segment in &packed {
        out.extend_from_slice(segment);
    }
    Ok(out)
}

/// PackBits: a run of three or more bytes becomes a replicate instruction, everything else a
/// literal. Both forms hold at most 128 bytes.
fn pack(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        let value = bytes[index];
        while index < bytes.len() && bytes[index] == value && index - start < 128 {
            index += 1;
        }
        let run = index - start;
        if run >= 3 {
            out.push((257u16 - run as u16) as u8);
            out.push(value);
            continue;
        }
        // A literal runs until the next place a run of three starts.
        let mut end = start;
        while end < bytes.len() && end - start < 128 {
            if end + 2 < bytes.len()
                && bytes[end] == bytes[end + 1]
                && bytes[end] == bytes[end + 2]
                && end > start
            {
                break;
            }
            end += 1;
        }
        out.push((end - start - 1) as u8);
        out.extend_from_slice(&bytes[start..end]);
        index = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixels::PhotometricInterpretation;

    fn descriptor(rows: u16, columns: u16, samples: u16, bits: u16) -> PixelDescriptor {
        PixelDescriptor {
            rows,
            columns,
            samples_per_pixel: samples,
            bits_allocated: bits,
            bits_stored: bits,
            high_bit: bits - 1,
            pixel_representation: super::super::PixelRepresentation::Unsigned,
            planar_configuration: if samples == 1 {
                PlanarConfiguration::None
            } else {
                PlanarConfiguration::Interleaved
            },
            photometric: PhotometricInterpretation::Monochrome2,
            number_of_frames: 1,
        }
    }

    #[test]
    fn a_frame_round_trips_through_encode_and_decode() {
        let descriptor = descriptor(2, 2, 1, 8);
        let frame = vec![1, 2, 3, 4];
        let encoded = encode(&frame, &descriptor).expect("encode");
        assert_eq!(decode(&encoded, &descriptor).expect("decode"), frame);
    }

    #[test]
    fn a_long_run_is_compressed_to_a_few_bytes() {
        let descriptor = descriptor(4, 4, 1, 8);
        let frame = vec![7; 16];
        let encoded = encode(&frame, &descriptor).expect("encode");
        assert_eq!(
            u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]),
            1
        );
        assert!(encoded.len() < HEADER_LEN + 8);
        assert_eq!(decode(&encoded, &descriptor).expect("decode"), frame);
    }

    #[test]
    fn a_sixteen_bit_frame_has_two_planes() {
        let descriptor = descriptor(2, 1, 1, 16);
        let frame = vec![0x01, 0x02, 0x03, 0x04];
        let encoded = encode(&frame, &descriptor).expect("encode");
        assert_eq!(
            u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]),
            2
        );
        assert_eq!(decode(&encoded, &descriptor).expect("decode"), frame);
    }

    #[test]
    fn an_interleaved_colour_frame_round_trips() {
        let descriptor = descriptor(1, 2, 3, 8);
        let frame = vec![1, 2, 3, 4, 5, 6];
        let encoded = encode(&frame, &descriptor).expect("encode");
        assert_eq!(
            u32::from_le_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]),
            3
        );
        assert_eq!(decode(&encoded, &descriptor).expect("decode"), frame);
    }
}
