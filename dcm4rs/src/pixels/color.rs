//! Colour handling: YBR conversions, palette colour lookup and bit packing (PS3.3 C.7.6.3.1.2,
//! PS3.5 Annex A).
//!
//! These mirror pydicom's `convert_color_space`, `expand_ybr422`, `apply_color_lut`,
//! `pack_bits` and `unpack_bits`. The conversions work on decoded samples and never touch the
//! stored pixel data, so a frame can be displayed as RGB while the file keeps its original
//! encoding.

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::pixels::{PhotometricInterpretation, PixelDescriptor};

/// A colour space a decoded frame can be expressed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    /// Red, green and blue.
    Rgb,
    /// Full-range luminance and chrominance, three samples per pixel.
    YbrFull,
    /// Full-range luminance with horizontally subsampled chrominance, two samples per pixel.
    YbrFull422,
    /// Partial-range luminance with horizontally subsampled chrominance.
    YbrPartial422,
    /// Irreversible colour transform of JPEG 2000.
    YbrIct,
    /// Reversible colour transform of JPEG 2000.
    YbrRct,
}

impl ColorSpace {
    /// The colour space of a photometric interpretation; `None` for the photometric
    /// interpretations that are not a colour space.
    pub fn from_photometric(photometric: &PhotometricInterpretation) -> Option<Self> {
        match photometric {
            PhotometricInterpretation::Rgb => Some(Self::Rgb),
            PhotometricInterpretation::YbrFull => Some(Self::YbrFull),
            PhotometricInterpretation::YbrFull422 => Some(Self::YbrFull422),
            PhotometricInterpretation::YbrPartial422 => Some(Self::YbrPartial422),
            PhotometricInterpretation::YbrIct => Some(Self::YbrIct),
            PhotometricInterpretation::YbrRct => Some(Self::YbrRct),
            _ => None,
        }
    }

    /// The photometric interpretation this colour space is written as.
    pub fn photometric(self) -> PhotometricInterpretation {
        match self {
            Self::Rgb => PhotometricInterpretation::Rgb,
            Self::YbrFull => PhotometricInterpretation::YbrFull,
            Self::YbrFull422 => PhotometricInterpretation::YbrFull422,
            Self::YbrPartial422 => PhotometricInterpretation::YbrPartial422,
            Self::YbrIct => PhotometricInterpretation::YbrIct,
            Self::YbrRct => PhotometricInterpretation::YbrRct,
        }
    }
}

impl std::fmt::Display for ColorSpace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Rgb => "RGB",
            Self::YbrFull => "YBR_FULL",
            Self::YbrFull422 => "YBR_FULL_422",
            Self::YbrPartial422 => "YBR_PARTIAL_422",
            Self::YbrIct => "YBR_ICT",
            Self::YbrRct => "YBR_RCT",
        })
    }
}

/// Convert one decoded frame to `to`, following the coefficients pydicom uses.
///
/// Only eight-bit frames with three samples per pixel are converted. `YBR_FULL`,
/// `YBR_FULL_422` and `YBR_PARTIAL_422` become `RGB`, and `RGB` becomes `YBR_FULL`; the
/// JPEG 2000 transforms (`YBR_ICT`, `YBR_RCT`) are refused because the JPEG 2000 decoder in this
/// crate already applies their inverse and hands back RGB. Converting a frame that is already in
/// the target space returns it unchanged.
pub fn convert_color_space(
    frame: &[u8],
    descriptor: &PixelDescriptor,
    to: ColorSpace,
) -> Result<Vec<u8>> {
    if descriptor.samples_per_pixel != 3 {
        return Err(Error::UnsupportedPixelData(format!(
            "colour conversion needs 3 samples per pixel, this frame has {}",
            descriptor.samples_per_pixel
        )));
    }
    if descriptor.bits_allocated != 8 {
        return Err(Error::UnsupportedPixelData(format!(
            "colour conversion handles 8 Bits Allocated, this frame has {}",
            descriptor.bits_allocated
        )));
    }
    let from = ColorSpace::from_photometric(&descriptor.photometric).ok_or_else(|| {
        Error::UnsupportedPixelData(format!("cannot convert {} to {to}", descriptor.photometric))
    })?;
    if from == to {
        return Ok(frame.to_vec());
    }
    match (from, to) {
        (ColorSpace::YbrFull, ColorSpace::Rgb) => Ok(ybr_full_to_rgb(frame)),
        (ColorSpace::YbrFull422, ColorSpace::Rgb) => Ok(ybr_full_to_rgb(&expand_ybr422(frame, 1)?)),
        (ColorSpace::YbrPartial422, ColorSpace::Rgb) => {
            Ok(ybr_partial_to_rgb(&expand_ybr422(frame, 1)?))
        }
        (ColorSpace::Rgb, ColorSpace::YbrFull) => Ok(rgb_to_ybr_full(frame)),
        (ColorSpace::YbrIct | ColorSpace::YbrRct, _) => Err(Error::UnsupportedPixelData(format!(
            "{from} is already RGB after JPEG 2000 decoding"
        ))),
        (from, to) => Err(Error::UnsupportedPixelData(format!(
            "converting {from} to {to} is not supported"
        ))),
    }
}

/// Expand `YBR_FULL_422` samples to three samples per pixel.
///
/// `YBR_FULL_422` stores two pixels in four bytes as `Y1 Y2 Cb Cr`; the expansion repeats the
/// chrominance and yields `Y1 Cb Cr Y2 Cb Cr`. `number_of_frames` only validates the length, as
/// pydicom's `expand_ybr422` does.
pub fn expand_ybr422(frame: &[u8], number_of_frames: usize) -> Result<Vec<u8>> {
    if number_of_frames == 0 {
        return Err(Error::InvalidValue(
            "number of frames must be non-zero".to_owned(),
        ));
    }
    if frame.len() % (4 * number_of_frames) != 0 {
        return Err(Error::InvalidPixelData(format!(
            "{} bytes of YBR_FULL_422 data do not divide into {} frame(s) of four bytes per two \
             pixels",
            frame.len(),
            number_of_frames
        )));
    }
    let mut expanded = Vec::with_capacity(frame.len() / 2 * 3);
    for group in frame.chunks_exact(4) {
        expanded.extend_from_slice(&[group[0], group[2], group[3], group[1], group[2], group[3]]);
    }
    Ok(expanded)
}

/// Expand bit-packed data to one byte per bit, most significant bit first.
///
/// This is pydicom's `unpack_bits`, needed for Overlay Data and other one-bit-per-pixel values.
pub fn unpack_bits(packed: &[u8]) -> Vec<u8> {
    let mut bits = Vec::with_capacity(packed.len() * 8);
    for byte in packed {
        for shift in (0..8).rev() {
            bits.push((byte >> shift) & 1);
        }
    }
    bits
}

/// Pack one byte per bit into the most significant bit first.
///
/// Every value must be `0` or `1`; the padding bits of the last byte are zero.
pub fn pack_bits(bits: &[u8]) -> Result<Vec<u8>> {
    if let Some(value) = bits.iter().find(|value| **value > 1) {
        return Err(Error::InvalidValue(format!(
            "bit-packed values must be 0 or 1, got {value}"
        )));
    }
    let mut packed = vec![0u8; bits.len().div_ceil(8)];
    for (index, bit) in bits.iter().enumerate() {
        if *bit == 1 {
            packed[index / 8] |= 1 << (7 - index % 8);
        }
    }
    Ok(packed)
}

/// The three lookup tables of a `PALETTE COLOR` image (PS3.3 C.7.6.3.1.2).
///
/// The stored samples are indexes into the tables. Entries are eight or sixteen bits wide; the
/// tables of a sixteen-bit image use the Large Palette Color Lookup Table attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteColorLut {
    first_mapped_index: i32,
    entries: usize,
    bits_per_entry: u8,
    red: Vec<u8>,
    green: Vec<u8>,
    blue: Vec<u8>,
}

impl PaletteColorLut {
    /// Read the three tables and their descriptors from a data set.
    ///
    /// Segmented lookup tables (PS3.3 C.7.9) are not expanded yet and are reported as
    /// unsupported.
    pub fn from_dataset(set: &DataSet) -> Result<Self> {
        if set.contains(tags::SEGMENTED_RED_PALETTE_COLOR_LOOKUP_TABLE_DATA) {
            return Err(Error::UnsupportedPixelData(
                "segmented palette colour lookup tables are not expanded".to_owned(),
            ));
        }
        let descriptor = if set.contains(tags::LARGE_RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR) {
            tags::LARGE_RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR
        } else {
            tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR
        };
        let values = set.u16s(descriptor)?;
        if values.len() < 3 {
            return Err(Error::InvalidPixelData(format!(
                "palette colour lookup table descriptor has {} value(s), 3 required",
                values.len()
            )));
        }
        let first_mapped_index = i32::from(values[0]);
        let entries = usize::from(values[1]).max(1);
        let bits_per_entry = values[2];
        if bits_per_entry != 8 && bits_per_entry != 16 {
            return Err(Error::UnsupportedPixelData(format!(
                "palette colour lookup table uses {bits_per_entry} bits per entry, 8 or 16 expected"
            )));
        }
        let (red_tag, green_tag, blue_tag) =
            if descriptor == tags::LARGE_RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR {
                (
                    tags::LARGE_RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                    tags::LARGE_GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                    tags::LARGE_BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                )
            } else {
                (
                    tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                    tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                    tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
                )
            };
        let endian = set.endian();
        Ok(Self {
            first_mapped_index,
            entries,
            bits_per_entry: bits_per_entry as u8,
            red: table(set, red_tag, entries, bits_per_entry, endian)?,
            green: table(set, green_tag, entries, bits_per_entry, endian)?,
            blue: table(set, blue_tag, entries, bits_per_entry, endian)?,
        })
    }

    /// The pixel value the first entry maps, which may be negative for signed images.
    pub fn first_mapped_index(&self) -> i32 {
        self.first_mapped_index
    }

    /// The number of entries in each table.
    pub fn entries(&self) -> usize {
        self.entries
    }

    /// The width of one entry in bits: eight or sixteen.
    pub fn bits_per_entry(&self) -> u8 {
        self.bits_per_entry
    }

    /// Map one frame of indexes to RGB.
    ///
    /// The result has three samples per pixel, one or two bytes each following
    /// [`bits_per_entry`](Self::bits_per_entry), in little endian. Indexes outside the table are
    /// clamped to its first or last entry, as pydicom does.
    pub fn apply(&self, frame: &[u8], bits_allocated: u16) -> Result<Vec<u8>> {
        let bytes_per_index = match bits_allocated {
            8 => 1,
            16 => 2,
            bits => {
                return Err(Error::UnsupportedPixelData(format!(
                    "palette colour lookup handles 8 or 16 Bits Allocated, this frame has {bits}"
                )));
            }
        };
        if frame.len() % bytes_per_index != 0 {
            return Err(Error::InvalidPixelData(format!(
                "{} bytes do not divide into {bytes_per_index}-byte indexes",
                frame.len()
            )));
        }
        let per_entry = usize::from(self.bits_per_entry / 8);
        let mut rgb = Vec::with_capacity(frame.len() / bytes_per_index * 3 * per_entry);
        for index in frame.chunks_exact(bytes_per_index) {
            let value = if bytes_per_index == 1 {
                i32::from(index[0])
            } else {
                i32::from(u16::from_le_bytes([index[0], index[1]]))
            };
            let entry =
                (value - self.first_mapped_index).clamp(0, self.entries as i32 - 1) as usize;
            let range = entry * per_entry..(entry + 1) * per_entry;
            rgb.extend_from_slice(&self.red[range.clone()]);
            rgb.extend_from_slice(&self.green[range.clone()]);
            rgb.extend_from_slice(&self.blue[range]);
        }
        Ok(rgb)
    }
}

/// Read one lookup table as raw entry bytes.
///
/// An eight-bit table is stored in `OW`, one entry per sixteen-bit word; writers disagree on
/// whether the entry sits in the low or the high byte, so a word whose low byte is zero and
/// whose high byte is not is read as the high byte. A table stored with one byte per entry is
/// accepted too.
fn table(
    set: &DataSet,
    tag: crate::tag::Tag,
    entries: usize,
    bits_per_entry: u16,
    endian: Endian,
) -> Result<Vec<u8>> {
    let bytes = set
        .element(tag)?
        .value
        .as_bytes()
        .ok_or_else(|| Error::InvalidPixelData(format!("{tag} is not a byte value")))?;
    if bits_per_entry == 8 {
        if bytes.len() >= entries {
            if bytes.len() == entries {
                return Ok(bytes[..entries].to_vec());
            }
            let mut table = Vec::with_capacity(entries);
            for word in bytes.chunks_exact(2).take(entries) {
                let value = match endian {
                    Endian::Little => u16::from_le_bytes([word[0], word[1]]),
                    Endian::Big => u16::from_be_bytes([word[0], word[1]]),
                };
                table.push(if value <= 0xFF {
                    value as u8
                } else if value & 0xFF == 0 {
                    (value >> 8) as u8
                } else {
                    return Err(Error::InvalidPixelData(format!(
                        "palette colour lookup table entry {value} does not fit in eight bits"
                    )));
                });
            }
            if table.len() == entries {
                return Ok(table);
            }
        }
        return Err(Error::InvalidPixelData(format!(
            "palette colour lookup table {tag} holds {} bytes, {entries} eight-bit entries \
             require {entries} or {}",
            bytes.len(),
            entries * 2
        )));
    }
    if bytes.len() < entries * 2 {
        return Err(Error::InvalidPixelData(format!(
            "palette colour lookup table {tag} holds {} bytes, {entries} sixteen-bit entries \
             require {}",
            bytes.len(),
            entries * 2
        )));
    }
    // The decoded frame is little endian, so a big-endian table is byte-swapped on the way out.
    let mut table = Vec::with_capacity(entries * 2);
    for word in bytes.chunks_exact(2).take(entries) {
        match endian {
            Endian::Little => table.extend_from_slice(word),
            Endian::Big => table.extend_from_slice(&[word[1], word[0]]),
        }
    }
    Ok(table)
}

fn ybr_full_to_rgb(frame: &[u8]) -> Vec<u8> {
    convert(frame, |y, cb, cr| {
        (
            f64::from(y) + 1.402 * (f64::from(cr) - 128.0),
            f64::from(y)
                - 0.344_136 * (f64::from(cb) - 128.0)
                - 0.714_136 * (f64::from(cr) - 128.0),
            f64::from(y) + 1.772 * (f64::from(cb) - 128.0),
        )
    })
}

fn ybr_partial_to_rgb(frame: &[u8]) -> Vec<u8> {
    convert(frame, |y, cb, cr| {
        let luma = 1.164 * (f64::from(y) - 16.0);
        let cb = f64::from(cb) - 128.0;
        let cr = f64::from(cr) - 128.0;
        (
            luma + 1.596 * cr,
            luma - 0.392 * cb - 0.813 * cr,
            luma + 2.017 * cb,
        )
    })
}

fn rgb_to_ybr_full(frame: &[u8]) -> Vec<u8> {
    let mut ybr = Vec::with_capacity(frame.len());
    for pixel in frame.chunks_exact(3) {
        let (r, g, b) = (
            f64::from(pixel[0]),
            f64::from(pixel[1]),
            f64::from(pixel[2]),
        );
        ybr.push(clamp(0.299 * r + 0.587 * g + 0.114 * b));
        ybr.push(clamp(-0.168_736 * r - 0.331_264 * g + 0.5 * b + 128.0));
        ybr.push(clamp(0.5 * r - 0.418_688 * g - 0.081_312 * b + 128.0));
    }
    ybr
}

fn convert(frame: &[u8], map: impl Fn(u8, u8, u8) -> (f64, f64, f64)) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(frame.len());
    for pixel in frame.chunks_exact(3) {
        let (r, g, b) = map(pixel[0], pixel[1], pixel[2]);
        rgb.push(clamp(r));
        rgb.push(clamp(g));
        rgb.push(clamp(b));
    }
    rgb
}

fn clamp(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(photometric: PhotometricInterpretation, samples: u16) -> PixelDescriptor {
        PixelDescriptor {
            rows: 1,
            columns: 2,
            samples_per_pixel: samples,
            bits_allocated: 8,
            bits_stored: 8,
            high_bit: 7,
            pixel_representation: crate::pixels::PixelRepresentation::Unsigned,
            planar_configuration: crate::pixels::PlanarConfiguration::Interleaved,
            photometric,
            number_of_frames: 1,
        }
    }

    #[test]
    fn neutral_ybr_full_is_grey() {
        let frame = [128u8, 128, 128];
        let rgb = convert_color_space(
            &frame,
            &descriptor(PhotometricInterpretation::YbrFull, 3),
            ColorSpace::Rgb,
        )
        .unwrap();
        assert_eq!(rgb, [128, 128, 128]);
    }

    #[test]
    fn ybr_full_422_is_expanded_before_conversion() {
        let expanded = expand_ybr422(&[128, 128, 128, 128], 1).unwrap();
        assert_eq!(expanded, [128, 128, 128, 128, 128, 128]);
        let frame = [76u8, 85, 255, 255];
        let rgb = convert_color_space(
            &frame,
            &descriptor(PhotometricInterpretation::YbrFull422, 3),
            ColorSpace::Rgb,
        )
        .unwrap();
        assert_eq!(rgb.len(), 6);
        assert!(rgb[0] > 200 && rgb[1] < 60 && rgb[2] > 200);
    }

    #[test]
    fn expanding_needs_whole_pixel_groups() {
        assert!(expand_ybr422(&[1, 2, 3], 1).is_err());
        assert!(expand_ybr422(&[1, 2, 3, 4], 0).is_err());
    }

    #[test]
    fn rgb_to_ybr_full_and_back_is_close() {
        let rgb = [10u8, 200, 30, 250, 0, 120];
        let descriptor = descriptor(PhotometricInterpretation::Rgb, 3);
        let ybr = convert_color_space(&rgb, &descriptor, ColorSpace::YbrFull).unwrap();
        let mut round_trip = descriptor.clone();
        round_trip.photometric = PhotometricInterpretation::YbrFull;
        let back = convert_color_space(&ybr, &round_trip, ColorSpace::Rgb).unwrap();
        for (a, b) in rgb.iter().zip(back.iter()) {
            assert!(a.abs_diff(*b) <= 2, "expected {rgb:?}, got {back:?}");
        }
    }

    #[test]
    fn bits_survive_a_pack_unpack_round_trip() {
        let bits = [1u8, 0, 1, 1, 0, 0, 1, 0, 1];
        let packed = pack_bits(&bits).unwrap();
        assert_eq!(packed, [0b1011_0010, 0b1000_0000]);
        assert_eq!(unpack_bits(&packed)[..bits.len()], bits);
        assert!(pack_bits(&[2]).is_err());
    }

    #[test]
    fn eight_bit_entries_in_the_high_byte_are_read_too() {
        let mut set = DataSet::new();
        set.set(
            tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            crate::vr::Vr::Us,
            crate::value::Value::from_u16s(&[0, 2, 8]),
        );
        let words: Vec<u16> = vec![0x0000, 0xFF00];
        for (descriptor, data) in [
            (
                tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
                tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
            ),
            (
                tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
                tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
            ),
        ] {
            set.set(
                descriptor,
                crate::vr::Vr::Us,
                crate::value::Value::from_u16s(&[0, 2, 8]),
            );
            set.set(
                data,
                crate::vr::Vr::Ow,
                crate::value::Value::from_u16s(&words),
            );
        }
        set.set(
            tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
            crate::vr::Vr::Ow,
            crate::value::Value::from_u16s(&words),
        );
        let lut = PaletteColorLut::from_dataset(&set).unwrap();
        assert_eq!(lut.entries(), 2);
        assert_eq!(lut.red, vec![0, 255]);
    }
}
