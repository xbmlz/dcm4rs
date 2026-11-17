//! JPEG XL decoding via `jxl-oxide` (pure Rust).

use super::{PixelDescriptor, PixelRepresentation};
use crate::error::{Error, Result};
use jxl_oxide::image::BitDepth;

/// Decode one encapsulated JPEG XL frame.
///
/// The decoder is asked for interleaved floating-point samples, then each sample is mapped back
/// to the integer range defined by `Bits Stored`. JPEG XL integer samples are unsigned, so signed
/// `Pixel Representation` is rejected.
pub(super) fn decode(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
    if descriptor.pixel_representation == PixelRepresentation::Signed {
        return Err(Error::UnsupportedPixelData(
            "JPEG XL signed samples are not supported".to_owned(),
        ));
    }
    if descriptor.bits_stored == 0 || descriptor.bits_stored > 16 {
        return Err(Error::UnsupportedPixelData(format!(
            "JPEG XL supports 1-16 Bits Stored, got {}",
            descriptor.bits_stored
        )));
    }

    let image = jxl_oxide::JxlImage::builder()
        .read(frame)
        .map_err(|err| Error::InvalidPixelData(format!("JPEG XL header failed: {err}")))?;
    let format = image.pixel_format();
    let samples = match format {
        jxl_oxide::PixelFormat::Gray => 1,
        jxl_oxide::PixelFormat::Rgb => 3,
        _ => {
            return Err(Error::UnsupportedPixelData(format!(
                "JPEG XL pixel format {format:?}"
            )));
        }
    };
    if samples != descriptor.samples_per_pixel {
        return Err(Error::InvalidPixelData(format!(
            "JPEG XL frame has {samples} component(s), descriptor has {}",
            descriptor.samples_per_pixel
        )));
    }
    let bits = match image.image_header().metadata.bit_depth {
        BitDepth::IntegerSample { bits_per_sample } => bits_per_sample,
        BitDepth::FloatSample { .. } => {
            return Err(Error::UnsupportedPixelData(
                "JPEG XL floating-point samples are not supported".to_owned(),
            ));
        }
    };
    if bits != u32::from(descriptor.bits_stored) {
        return Err(Error::InvalidPixelData(format!(
            "JPEG XL bit depth {bits}, Bits Stored is {}",
            descriptor.bits_stored
        )));
    }

    let render = image
        .render_frame(0)
        .map_err(|err| Error::InvalidPixelData(format!("JPEG XL render failed: {err}")))?;
    let mut stream = render.stream();
    if stream.width() != u32::from(descriptor.columns)
        || stream.height() != u32::from(descriptor.rows)
        || stream.channels() != u32::from(samples)
    {
        return Err(Error::InvalidPixelData(format!(
            "JPEG XL frame is {}x{} with {} component(s), descriptor is {}x{} with {}",
            stream.width(),
            stream.height(),
            stream.channels(),
            descriptor.columns,
            descriptor.rows,
            descriptor.samples_per_pixel
        )));
    }

    let pixel_count = usize::from(descriptor.rows) * usize::from(descriptor.columns);
    let mut floats = vec![0f32; pixel_count * usize::from(samples)];
    let written = stream.write_to_buffer(&mut floats);
    if written != floats.len() {
        return Err(Error::InvalidPixelData(format!(
            "JPEG XL stream wrote {written} of {} samples",
            floats.len()
        )));
    }

    let max = ((1u32 << descriptor.bits_stored) - 1) as f32;
    let bytes_per_sample = descriptor.bytes_per_sample();
    let mut out = vec![0u8; pixel_count * usize::from(samples) * bytes_per_sample];
    for (index, &sample) in floats.iter().enumerate() {
        let value = (sample.clamp(0.0, 1.0) * max).round() as u32;
        let offset = index * bytes_per_sample;
        out[offset..offset + bytes_per_sample]
            .copy_from_slice(&value.to_le_bytes()[..bytes_per_sample]);
    }
    Ok(out)
}
