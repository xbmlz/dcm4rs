//! Baseline and progressive JPEG decoding via `zune-jpeg`.

use super::PixelDescriptor;
use crate::error::{Error, Result};

/// Decode one encapsulated JPEG frame.
///
/// The decoder emits RGB for three-component images and grayscale for one-component images,
/// matching the pixel layout DICOM applications expect after JPEG decompression. Only 8-bit
/// samples are accepted; 12-bit and JPEG lossless remain unsupported.
pub(super) fn decode(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
    if descriptor.bits_allocated != 8 {
        return Err(Error::UnsupportedPixelData(format!(
            "JPEG decoding supports 8-bit samples, got {} bits",
            descriptor.bits_allocated
        )));
    }
    let mut decoder = zune_jpeg::JpegDecoder::new(frame);
    let pixels = decoder
        .decode()
        .map_err(|err| Error::InvalidPixelData(format!("JPEG decode failed: {err}")))?;
    let expected = descriptor.frame_size();
    if pixels.len() != expected {
        return Err(Error::InvalidPixelData(format!(
            "JPEG decoded {} bytes, descriptor requires {expected}",
            pixels.len()
        )));
    }
    Ok(pixels)
}
