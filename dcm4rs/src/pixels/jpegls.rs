//! JPEG-LS Lossless and Near-lossless decoding via `charls`.

use super::PixelDescriptor;
use crate::error::{Error, Result};

/// Decode one encapsulated JPEG-LS frame.
///
/// CharLS emits the native sample layout (interleaved for the sample/line modes used by DICOM,
/// planar for the none mode). Multi-component frames are assumed interleaved, matching the
/// `PlanarConfiguration` of encapsulated data sets.
pub(super) fn decode(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
    let mut header = charls::CharLS::default();
    let info = header
        .get_frame_info(frame)
        .map_err(|err| Error::InvalidPixelData(format!("JPEG-LS header failed: {err}")))?;
    if info.width != u32::from(descriptor.columns)
        || info.height != u32::from(descriptor.rows)
        || info.component_count != i32::from(descriptor.samples_per_pixel)
        || info.bits_per_sample != i32::from(descriptor.bits_allocated)
    {
        return Err(Error::InvalidPixelData(format!(
            "JPEG-LS frame is {}x{} with {} component(s) at {} bit(s), descriptor is {}x{} with {} component(s) at {} bit(s)",
            info.width,
            info.height,
            info.component_count,
            info.bits_per_sample,
            descriptor.columns,
            descriptor.rows,
            descriptor.samples_per_pixel,
            descriptor.bits_allocated
        )));
    }

    let mut decoder = charls::CharLS::default();
    let mut pixels = decoder
        .decode(frame)
        .map_err(|err| Error::InvalidPixelData(format!("JPEG-LS decode failed: {err}")))?;
    let expected = descriptor.frame_size();
    if pixels.len() != expected {
        return Err(Error::InvalidPixelData(format!(
            "JPEG-LS decoded {} bytes, descriptor requires {expected}",
            pixels.len()
        )));
    }
    swap_on_big_endian(&mut pixels, descriptor.bytes_per_sample());
    Ok(pixels)
}

/// CharLS returns 16-bit samples in host byte order; the native pixel layout is little endian.
fn swap_on_big_endian(pixels: &mut [u8], bytes_per_sample: usize) {
    #[cfg(target_endian = "big")]
    if bytes_per_sample == 2 {
        for sample in pixels.chunks_exact_mut(2) {
            sample.swap(0, 1);
        }
    }
    let _ = (pixels, bytes_per_sample);
}
