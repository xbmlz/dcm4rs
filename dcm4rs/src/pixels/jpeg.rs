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
