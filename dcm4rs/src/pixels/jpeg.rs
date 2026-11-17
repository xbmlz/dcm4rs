//! Baseline, extended and progressive JPEG decoding.

use super::PixelDescriptor;
use crate::error::{Error, Result};

/// Decode one encapsulated JPEG frame.
///
/// Eight-bit frames are decoded by `zune-jpeg`, which implements baseline and progressive DCT.
/// Frames wider than eight bits — DICOM's JPEG Extended, used by 12-bit and 10-bit images — go
/// to `oxiarc-jpeg`, whose extended sequential and progressive decoders run at eight and twelve
/// bits with libjpeg-turbo's inverse DCT. Both paths deliver RGB for three-component images and
/// grayscale for one-component images, matching the layout DICOM applications expect after JPEG
/// decompression.
pub(super) fn decode(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
    let repaired = repair_sequential_spectral_end(frame);
    let frame = repaired.as_deref().unwrap_or(frame);
    let pixels = if descriptor.bits_allocated > 8 {
        decode_wide(frame, descriptor)?
    } else {
        decode_narrow(frame)?
    };
    let expected = descriptor.frame_size();
    if pixels.len() != expected {
        return Err(Error::InvalidPixelData(format!(
            "JPEG decoded {} bytes, descriptor requires {expected}",
            pixels.len()
        )));
    }
    Ok(pixels)
}

/// Repair a sequential scan whose `SOS` declares a spectral end other than 63.
///
/// T.81 B.2.3 requires `Ss == 0` and `Se == 63` for a sequential scan, but encoders in the
/// field write `Se == 0`, and libjpeg answers that with a warning and decodes anyway: a
/// sequential Huffman scan reads all 64 coefficients regardless, so only the conformance
/// check notices. `pylibjpeg-data`'s published replacement for such a file differs from the
/// broken one in this byte alone, and both decode to identical samples.
///
/// Only the first scan of a sequential frame is touched. Progressive scans legitimately use
/// `Se == 0` for their DC scan, so they are left exactly as they are.
fn repair_sequential_spectral_end(frame: &[u8]) -> Option<Vec<u8>> {
    let mut sequential = false;
    let mut offset = 2;
    if frame.starts_with(&[0xFF, 0xD8]) {
        loop {
            if frame.get(offset).copied() != Some(0xFF) {
                return None;
            }
            while frame.get(offset + 1).copied() == Some(0xFF) {
                offset += 1;
            }
            let marker = frame.get(offset + 1).copied()?;
            offset += 2;
            match marker {
                // Markers that carry no length field.
                0x01 | 0xD0..=0xD7 => continue,
                0xC0 | 0xC1 => sequential = true,
                0xDA => {
                    let length = frame.get(offset..offset + 2)?;
                    if u16::from_be_bytes([length[0], length[1]]) < 6 {
                        return None;
                    }
                    let components = usize::from(*frame.get(offset + 2)?);
                    let spectral = offset + 3 + 2 * components;
                    if *frame.get(spectral)? != 0 || *frame.get(spectral + 1)? == 63 {
                        return None;
                    }
                    if !sequential {
                        return None;
                    }
                    let mut repaired = frame.to_vec();
                    repaired[spectral + 1] = 63;
                    return Some(repaired);
                }
                0xD9 => return None,
                _ => {}
            }
            let length = frame.get(offset..offset + 2)?;
            offset += usize::from(u16::from_be_bytes([length[0], length[1]]));
            if offset >= frame.len() {
                return None;
            }
        }
    }
    None
}

fn decode_narrow(frame: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = zune_jpeg::JpegDecoder::new(frame);
    decoder
        .decode()
        .map_err(|err| Error::InvalidPixelData(format!("JPEG decode failed: {err}")))
}

/// Decode a frame of more than eight bits per sample into little-endian 16-bit samples.
fn decode_wide(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
    let mut decoder = oxiarc_jpeg::Decoder::new(frame);
    let info = decoder
        .read_info()
        .map_err(|err| Error::InvalidPixelData(format!("JPEG decode failed: {err}")))?;
    if usize::from(info.num_components) != usize::from(descriptor.samples_per_pixel) {
        return Err(Error::InvalidPixelData(format!(
            "JPEG stream carries {} component(s), descriptor requires {}",
            info.num_components, descriptor.samples_per_pixel
        )));
    }
    let samples = decoder
        .decode_u16()
        .map_err(|err| Error::InvalidPixelData(format!("JPEG decode failed: {err}")))?;
    Ok(samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect())
}
