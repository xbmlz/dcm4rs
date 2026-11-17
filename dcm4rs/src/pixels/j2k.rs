//! JPEG 2000 and HTJ2K decoding via `jpeg2k` (the pure-Rust `openjp2` port by default).

use super::{PixelDescriptor, PixelRepresentation};
use crate::error::{Error, Result};

/// Decode one encapsulated JPEG 2000 / HTJ2K frame.
///
/// The codestream determines the dimensions, component count, precision and signedness; all
/// four must agree with the descriptor. Components are interleaved into native little-endian
/// samples. Color codestreams have the inverse multiple-component transform applied by the
/// decoder, so the returned samples are RGB.
pub(super) fn decode(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
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
    let signed = descriptor.pixel_representation == PixelRepresentation::Signed;
    let mut out = vec![0u8; pixel_count * components.len() * bytes_per_sample];

    for (sample, component) in components.iter().enumerate() {
        if component.precision() != u32::from(descriptor.bits_stored) {
            return Err(Error::InvalidPixelData(format!(
                "JPEG 2000 component {sample} has precision {}, Bits Stored is {}",
                component.precision(),
                descriptor.bits_stored
            )));
        }
        if component.is_signed() != signed {
            return Err(Error::InvalidPixelData(format!(
                "JPEG 2000 component {sample} signedness disagrees with Pixel Representation"
            )));
        }
        let data = component.data();
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
