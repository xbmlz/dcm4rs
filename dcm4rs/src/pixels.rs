//! Pixel data decoding: descriptors, frame extraction and codecs.
//!
//! The descriptor is read from the group 0028 attributes (rows, columns, samples, bit depth,
//! representation, planar configuration and photometric interpretation). Frames are extracted
//! from native Pixel Data by their fixed size, or from encapsulated Pixel Data by the Basic
//! Offset Table (one fragment per frame when the table is empty).
//!
//! RLE Lossless (PS3.5 Annex G) is decoded here. JPEG, JPEG-LS, JPEG 2000/HTJ2K and JPEG XL are
//! feature-gated additions; MPEG reports [`Error::UnsupportedPixelData`].

use std::fmt;

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::error::{Error, Result};
use crate::file::DicomFile;
use crate::transfer_syntax::TransferSyntax;

#[cfg(feature = "pixel-j2k")]
mod j2k;
#[cfg(feature = "pixel-jpeg")]
mod jpeg;
#[cfg(feature = "pixel-jpegls")]
mod jpegls;
#[cfg(feature = "pixel-jxl")]
mod jxl;
mod rle;

/// How a pixel sample is interpreted: unsigned or two's-complement signed
/// (`(0028,0103) Pixel Representation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelRepresentation {
    /// Unsigned integer samples (value 0).
    Unsigned,
    /// Signed integer samples (value 1).
    Signed,
}

/// How samples are arranged within a frame (`(0028,0006) Planar Configuration`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanarConfiguration {
    /// A single sample per pixel; the attribute is not present.
    None,
    /// Sample values are interleaved per pixel (value 0).
    Interleaved,
    /// Each sample plane is stored contiguously (value 1).
    Planar,
}

/// The photometric interpretation of the pixel data (`(0028,0004)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhotometricInterpretation {
    /// `MONOCHROME1`: minimum value is white.
    Monochrome1,
    /// `MONOCHROME2`: minimum value is black.
    Monochrome2,
    /// `PALETTE COLOR`: samples are indexes into a color lookup table.
    PaletteColor,
    /// `RGB`: red, green and blue samples.
    Rgb,
    /// `YBR_FULL`: full-range luminance and chrominance.
    YbrFull,
    /// `YBR_FULL_422`: horizontally subsampled chrominance.
    YbrFull422,
    /// `YBR_PARTIAL_422`: partial-range, horizontally subsampled chrominance.
    YbrPartial422,
    /// `YBR_PARTIAL_420`: partial-range, horizontally and vertically subsampled chrominance.
    YbrPartial420,
    /// `YBR_ICT`: irreversible color transform (JPEG 2000).
    YbrIct,
    /// `YBR_RCT`: reversible color transform (JPEG 2000).
    YbrRct,
    /// `HSV`: hue, saturation and value.
    Hsv,
    /// `ARGB`: alpha, red, green and blue.
    Argb,
    /// `CMYK`: cyan, magenta, yellow and black.
    Cmyk,
    /// A value this crate does not interpret; the original text is retained.
    Unknown(String),
}

impl PhotometricInterpretation {
    fn from_dicom(text: &str) -> Self {
        match text.trim() {
            "MONOCHROME1" => Self::Monochrome1,
            "MONOCHROME2" => Self::Monochrome2,
            "PALETTE COLOR" => Self::PaletteColor,
            "RGB" => Self::Rgb,
            "YBR_FULL" => Self::YbrFull,
            "YBR_FULL_422" => Self::YbrFull422,
            "YBR_PARTIAL_422" => Self::YbrPartial422,
            "YBR_PARTIAL_420" => Self::YbrPartial420,
            "YBR_ICT" => Self::YbrIct,
            "YBR_RCT" => Self::YbrRct,
            "HSV" => Self::Hsv,
            "ARGB" => Self::Argb,
            "CMYK" => Self::Cmyk,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

impl fmt::Display for PhotometricInterpretation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Monochrome1 => f.write_str("MONOCHROME1"),
            Self::Monochrome2 => f.write_str("MONOCHROME2"),
            Self::PaletteColor => f.write_str("PALETTE COLOR"),
            Self::Rgb => f.write_str("RGB"),
            Self::YbrFull => f.write_str("YBR_FULL"),
            Self::YbrFull422 => f.write_str("YBR_FULL_422"),
            Self::YbrPartial422 => f.write_str("YBR_PARTIAL_422"),
            Self::YbrPartial420 => f.write_str("YBR_PARTIAL_420"),
            Self::YbrIct => f.write_str("YBR_ICT"),
            Self::YbrRct => f.write_str("YBR_RCT"),
            Self::Hsv => f.write_str("HSV"),
            Self::Argb => f.write_str("ARGB"),
            Self::Cmyk => f.write_str("CMYK"),
            Self::Unknown(text) => f.write_str(text),
        }
    }
}

/// The attributes that describe one pixel data frame (PS3.3 C.7.6.3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelDescriptor {
    /// Rows in the image (`(0028,0010)`).
    pub rows: u16,
    /// Columns in the image (`(0028,0011)`).
    pub columns: u16,
    /// Number of samples per pixel (`(0028,0002)`).
    pub samples_per_pixel: u16,
    /// Number of bits allocated per sample (`(0028,0100)`).
    pub bits_allocated: u16,
    /// Number of significant bits per sample (`(0028,0101)`).
    pub bits_stored: u16,
    /// Position of the most significant bit (`(0028,0102)`).
    pub high_bit: u16,
    /// Sample signedness (`(0028,0103)`).
    pub pixel_representation: PixelRepresentation,
    /// Sample arrangement (`(0028,0006)`); [`PlanarConfiguration::None`] for one sample.
    pub planar_configuration: PlanarConfiguration,
    /// Photometric interpretation (`(0028,0004)`).
    pub photometric: PhotometricInterpretation,
    /// Number of frames (`(0028,0008)`); 1 when the attribute is absent.
    pub number_of_frames: u32,
}

impl PixelDescriptor {
    /// Read the descriptor from a data set, validating the required attributes.
    pub fn from_dataset(set: &DataSet) -> Result<Self> {
        let rows = set.u16(tags::ROWS)?;
        let columns = set.u16(tags::COLUMNS)?;
        let samples_per_pixel = set.u16(tags::SAMPLES_PER_PIXEL)?;
        let bits_allocated = set.u16(tags::BITS_ALLOCATED)?;
        let bits_stored = set.u16(tags::BITS_STORED)?;
        let high_bit = set.u16(tags::HIGH_BIT)?;
        let pixel_representation = match set.u16(tags::PIXEL_REPRESENTATION)? {
            0 => PixelRepresentation::Unsigned,
            1 => PixelRepresentation::Signed,
            value => {
                return Err(Error::InvalidPixelData(format!(
                    "invalid Pixel Representation: {value}"
                )));
            }
        };
        let planar_configuration = if samples_per_pixel == 1 {
            PlanarConfiguration::None
        } else if set.contains(tags::PLANAR_CONFIGURATION) {
            match set.u16(tags::PLANAR_CONFIGURATION)? {
                0 => PlanarConfiguration::Interleaved,
                1 => PlanarConfiguration::Planar,
                value => {
                    return Err(Error::InvalidPixelData(format!(
                        "invalid Planar Configuration: {value}"
                    )));
                }
            }
        } else {
            PlanarConfiguration::Interleaved
        };
        let photometric_text = set.string(tags::PHOTOMETRIC_INTERPRETATION)?;
        let photometric = PhotometricInterpretation::from_dicom(&photometric_text);
        let number_of_frames = match set.get(tags::NUMBER_OF_FRAMES) {
            Some(element) => {
                let text = element.string()?.trim();
                text.parse::<u32>().map_err(|_| {
                    Error::InvalidPixelData(format!("invalid Number of Frames: {text:?}"))
                })?
            }
            None => 1,
        };

        if rows == 0 || columns == 0 {
            return Err(Error::InvalidPixelData(
                "Rows and Columns must be non-zero".to_owned(),
            ));
        }
        if samples_per_pixel == 0 || samples_per_pixel > 4 {
            return Err(Error::InvalidPixelData(format!(
                "invalid Samples per Pixel: {samples_per_pixel}"
            )));
        }
        if bits_allocated == 0 || bits_allocated > 32 {
            return Err(Error::InvalidPixelData(format!(
                "invalid Bits Allocated: {bits_allocated}"
            )));
        }
        if bits_stored == 0 || bits_stored > bits_allocated {
            return Err(Error::InvalidPixelData(format!(
                "invalid Bits Stored: {bits_stored} for Bits Allocated {bits_allocated}"
            )));
        }
        if high_bit >= bits_allocated {
            return Err(Error::InvalidPixelData(format!(
                "invalid High Bit: {high_bit} for Bits Allocated {bits_allocated}"
            )));
        }
        if number_of_frames == 0 {
            return Err(Error::InvalidPixelData(
                "Number of Frames must be non-zero".to_owned(),
            ));
        }

        Ok(Self {
            rows,
            columns,
            samples_per_pixel,
            bits_allocated,
            bits_stored,
            high_bit,
            pixel_representation,
            planar_configuration,
            photometric,
            number_of_frames,
        })
    }

    /// Bytes per sample in the stored pixel data.
    pub fn bytes_per_sample(&self) -> usize {
        usize::from(self.bits_allocated).div_ceil(8)
    }

    /// Bytes in one native frame, including the packed layout when bits are allocated below 8.
    pub fn frame_size(&self) -> usize {
        let pixels = usize::from(self.rows)
            * usize::from(self.columns)
            * usize::from(self.samples_per_pixel);
        if self.bits_allocated < 8 {
            pixels.div_ceil(8)
        } else {
            pixels * self.bytes_per_sample()
        }
    }
}

impl DataSet {
    /// Read the pixel descriptor from this data set.
    pub fn pixel_descriptor(&self) -> Result<PixelDescriptor> {
        PixelDescriptor::from_dataset(self)
    }

    /// Split encapsulated Pixel Data into frames.
    ///
    /// The first fragment is the Basic Offset Table. An empty table means one frame per
    /// fragment; otherwise its 32-bit offsets locate each frame within the concatenated
    /// fragment data.
    pub fn encapsulated_frames(&self) -> Result<Vec<Vec<u8>>> {
        let element = self.element(tags::PIXEL_DATA)?;
        let Some(fragments) = element.value.as_fragments() else {
            return Err(Error::InvalidPixelData(
                "Pixel Data is not encapsulated".to_owned(),
            ));
        };
        if fragments.is_empty() {
            return Ok(Vec::new());
        }
        let basic_offset_table = &fragments[0];
        if basic_offset_table.is_empty() {
            return Ok(fragments[1..].to_vec());
        }
        if basic_offset_table.len() % 4 != 0 {
            return Err(Error::InvalidPixelData(format!(
                "Basic Offset Table length {} is not a multiple of 4",
                basic_offset_table.len()
            )));
        }
        let offsets: Vec<usize> = basic_offset_table
            .chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize)
            .collect();
        if offsets.first() != Some(&0) {
            return Err(Error::InvalidPixelData(
                "Basic Offset Table does not start at zero".to_owned(),
            ));
        }
        let data: Vec<u8> = fragments[1..]
            .iter()
            .flat_map(|fragment| fragment.iter().copied())
            .collect();
        let mut frames = Vec::with_capacity(offsets.len());
        for (index, &start) in offsets.iter().enumerate() {
            let end = offsets.get(index + 1).copied().unwrap_or(data.len());
            if start > end || end > data.len() {
                return Err(Error::InvalidPixelData(format!(
                    "Basic Offset Table entry {start}..{end} exceeds {} fragment bytes",
                    data.len()
                )));
            }
            frames.push(data[start..end].to_vec());
        }
        Ok(frames)
    }
}

impl DicomFile {
    /// The pixel descriptor of this file's data set.
    pub fn pixel_descriptor(&self) -> Result<PixelDescriptor> {
        self.dataset.pixel_descriptor()
    }

    /// The encoded frames of this file, without decompression.
    ///
    /// Native Pixel Data is split by [`PixelDescriptor::frame_size`]; encapsulated Pixel Data is
    /// split by the Basic Offset Table. The transfer syntax is taken from the file meta
    /// information.
    pub fn pixel_frames(&self) -> Result<Vec<Vec<u8>>> {
        let syntax = self.transfer_syntax()?;
        let descriptor = self.dataset.pixel_descriptor()?;
        if syntax.is_encapsulated() {
            return self.dataset.encapsulated_frames();
        }
        let element = self.dataset.element(tags::PIXEL_DATA)?;
        let Some(bytes) = element.value.as_bytes() else {
            return Err(Error::InvalidPixelData(
                "Pixel Data is not a byte value".to_owned(),
            ));
        };
        let frame_size = descriptor.frame_size();
        if frame_size == 0 {
            return Err(Error::InvalidPixelData("frame size is zero".to_owned()));
        }
        let expected = frame_size
            .checked_mul(descriptor.number_of_frames as usize)
            .ok_or_else(|| Error::InvalidPixelData("pixel data size overflows".to_owned()))?;
        if bytes.len() < expected {
            return Err(Error::InvalidPixelData(format!(
                "Pixel Data has {} bytes, {expected} required for {} frame(s)",
                bytes.len(),
                descriptor.number_of_frames
            )));
        }
        Ok((0..descriptor.number_of_frames as usize)
            .map(|index| bytes[index * frame_size..(index + 1) * frame_size].to_vec())
            .collect())
    }

    /// Decode one frame (zero-based).
    ///
    /// Native transfer syntaxes return the stored bytes. Encapsulated transfer syntaxes are
    /// decoded by their codec; RLE Lossless is supported, other codecs report
    /// [`Error::UnsupportedPixelData`].
    pub fn decode_frame(&self, index: usize) -> Result<Vec<u8>> {
        let descriptor = self.dataset.pixel_descriptor()?;
        let syntax = self.transfer_syntax()?;
        let frames = self.pixel_frames()?;
        let frame = frames.get(index).ok_or_else(|| {
            Error::InvalidPixelData(format!(
                "frame index {index} out of range (have {})",
                frames.len()
            ))
        })?;
        decode_frame_bytes(syntax, &descriptor, frame)
    }

    /// Decode every frame of this file.
    pub fn decode_pixel_data(&self) -> Result<Vec<Vec<u8>>> {
        let descriptor = self.dataset.pixel_descriptor()?;
        let syntax = self.transfer_syntax()?;
        self.pixel_frames()?
            .iter()
            .map(|frame| decode_frame_bytes(syntax, &descriptor, frame))
            .collect()
    }
}

fn decode_frame_bytes(
    syntax: TransferSyntax,
    descriptor: &PixelDescriptor,
    frame: &[u8],
) -> Result<Vec<u8>> {
    if let TransferSyntax::Encapsulated(uid) = syntax {
        if uid == "1.2.840.10008.1.2.5" {
            return rle::decode(frame, descriptor);
        }
        #[cfg(feature = "pixel-jpeg")]
        if is_jpeg_uid(uid) {
            return jpeg::decode(frame, descriptor);
        }
        #[cfg(feature = "pixel-jpegls")]
        if is_jpegls_uid(uid) {
            return jpegls::decode(frame, descriptor);
        }
        #[cfg(feature = "pixel-j2k")]
        if is_j2k_uid(uid) {
            return j2k::decode(frame, descriptor);
        }
        #[cfg(feature = "pixel-jxl")]
        if is_jxl_uid(uid) {
            return jxl::decode(frame, descriptor);
        }
        return Err(Error::UnsupportedPixelData(format!(
            "transfer syntax {uid}"
        )));
    }
    Ok(frame.to_vec())
}

/// JPEG Baseline (Process 1), Extended (Process 2/4), Spectral Selection and Full Progression.
///
/// JPEG Lossless (Process 14) and JPEG-LS are different codecs and stay unsupported.
#[cfg(feature = "pixel-jpeg")]
fn is_jpeg_uid(uid: &str) -> bool {
    matches!(
        uid,
        "1.2.840.10008.1.2.4.50"
            | "1.2.840.10008.1.2.4.51"
            | "1.2.840.10008.1.2.4.52"
            | "1.2.840.10008.1.2.4.53"
            | "1.2.840.10008.1.2.4.54"
    )
}

/// JPEG-LS Lossless (`.80`) and Near-lossless (`.81`).
#[cfg(feature = "pixel-jpegls")]
fn is_jpegls_uid(uid: &str) -> bool {
    matches!(uid, "1.2.840.10008.1.2.4.80" | "1.2.840.10008.1.2.4.81")
}

/// JPEG XL Lossless (`.110`), JPEG Recompression (`.111`) and Lossy (`.112`).
#[cfg(feature = "pixel-jxl")]
fn is_jxl_uid(uid: &str) -> bool {
    matches!(
        uid,
        "1.2.840.10008.1.2.4.110" | "1.2.840.10008.1.2.4.111" | "1.2.840.10008.1.2.4.112"
    )
}

/// JPEG 2000 (`.90`, `.91`) and HTJ2K (`.201`–`.204`).
#[cfg(feature = "pixel-j2k")]
fn is_j2k_uid(uid: &str) -> bool {
    matches!(
        uid,
        "1.2.840.10008.1.2.4.90"
            | "1.2.840.10008.1.2.4.91"
            | "1.2.840.10008.1.2.4.201"
            | "1.2.840.10008.1.2.4.202"
            | "1.2.840.10008.1.2.4.203"
            | "1.2.840.10008.1.2.4.204"
    )
}
