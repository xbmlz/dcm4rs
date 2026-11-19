//! Pixel data decoding: descriptors, frame extraction and codecs.
//!
//! The descriptor is read from the group 0028 attributes (rows, columns, samples, bit depth,
//! representation, planar configuration and photometric interpretation). Frames are extracted
//! from native Pixel Data by their fixed size, or from encapsulated Pixel Data by the Basic
//! Offset Table (one fragment per frame when the table is empty).
//!
//! RLE Lossless (PS3.5 Annex G) and JPEG Lossless (Process 14) are decoded here. JPEG
//! (eight-bit baseline and progressive, and the 12-bit extended process), JPEG-LS, JPEG
//! 2000/HTJ2K and JPEG XL are feature-gated additions.
//!
//! [`color`] converts YBR to RGB, applies palette colour lookup tables and packs bits, and
//! [`DicomFile::decompress`] rewrites an encapsulated file as native samples.
//!
//! MPEG video is not decoded: [`DataSet::encapsulated_frames`] returns its frames as the
//! compressed bitstreams they are, [`Error::UnsupportedPixelData`] when a pixel is asked for, and
//! [`crate::mpeg`] reads the sequence header for the attributes a viewer needs.

use std::fmt;

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::file::DicomFile;
use crate::transfer_syntax::TransferSyntax;
use crate::value::Value;
use crate::vr::Vr;

/// Transfer syntax UID of RLE Lossless, the one encoding this crate writes.
const RLE_LOSSLESS: &str = "1.2.840.10008.1.2.5";

/// Explicit VR Little Endian, the syntax [`DicomFile::decompress`] writes.
const EXPLICIT_VR_LITTLE_ENDIAN: &str = "1.2.840.10008.1.2.1";

#[cfg(feature = "pixel-j2k")]
mod j2k;
#[cfg(feature = "pixel-jpeg")]
mod jpeg;
mod jpeg_lossless;
#[cfg(feature = "pixel-jpegls")]
mod jpegls;
#[cfg(feature = "pixel-jxl")]
mod jxl;
mod lut;
mod rle;

pub mod color;

pub use color::{
    convert_color_space, expand_ybr422, pack_bits, unpack_bits, ColorSpace, PaletteColorLut,
};

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

    /// Bytes in one frame once decoded, including the packed layout when bits are allocated
    /// below 8.
    ///
    /// This is the size the codecs produce and the size after chroma expansion, so it is what
    /// [`DicomFile::decode_frame`] returns and what a compression codec is checked against.
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

    /// Bytes of one frame as Pixel Data stores them.
    ///
    /// The subsampled colour spaces store fewer samples than [`frame_size`](Self::frame_size)
    /// counts: `YBR_FULL_422` and `YBR_PARTIAL_422` keep two pixels in three samples and
    /// `YBR_PARTIAL_420` keeps four pixels in six, so a native frame is smaller than the decoded
    /// one until the chroma is expanded.
    pub fn stored_frame_size(&self) -> usize {
        let pixels = usize::from(self.rows) * usize::from(self.columns);
        if self.bits_allocated < 8 {
            return (pixels * usize::from(self.samples_per_pixel)).div_ceil(8);
        }
        let samples = match self.photometric {
            PhotometricInterpretation::YbrFull422 | PhotometricInterpretation::YbrPartial422
                if self.samples_per_pixel == 3 =>
            {
                pixels * 2
            }
            PhotometricInterpretation::YbrPartial420 if self.samples_per_pixel == 3 => {
                (pixels * 3).div_ceil(2)
            }
            _ => pixels * usize::from(self.samples_per_pixel),
        };
        samples * self.bytes_per_sample()
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
            return self.frameless_encapsulated_frames(&fragments[1..]);
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
        // The offsets are measured from the first fragment's item tag, so each fragment
        // contributes its value plus the 8-byte item tag and length. Frames may span several
        // fragments; the table only marks the frame boundaries. This mirrors pydicom's
        // `generate_fragmented_frames`.
        let fragments = &fragments[1..];
        let final_index = offsets.len() - 1;
        let mut frames = Vec::new();
        let mut frame = Vec::new();
        let mut current = 0usize;
        let mut stream_offset = 0usize;
        for fragment in fragments {
            if current == final_index {
                frame.extend_from_slice(fragment);
                stream_offset += fragment.len() + 8;
                continue;
            }
            if stream_offset < offsets[current + 1] {
                frame.extend_from_slice(fragment);
            } else {
                frames.push(std::mem::take(&mut frame));
                current += 1;
                frame = fragment.clone();
            }
            stream_offset += fragment.len() + 8;
        }
        if !frame.is_empty() || frames.is_empty() {
            frames.push(frame);
        }
        Ok(frames)
    }

    /// Split fragments into frames when the Basic Offset Table is empty.
    ///
    /// Mirrors pydicom's `generate_fragmented_frames`: a single fragment is one frame,
    /// `Number of Frames` equal to the fragment count means one fragment per frame, one frame
    /// means every fragment belongs to it, and otherwise a JPEG end-of-image marker (`FF D9`)
    /// near the end of a fragment closes a frame.
    fn frameless_encapsulated_frames(&self, fragments: &[Vec<u8>]) -> Result<Vec<Vec<u8>>> {
        if fragments.is_empty() {
            return Ok(Vec::new());
        }
        let number_of_frames = match self.get(tags::NUMBER_OF_FRAMES) {
            Some(element) => {
                let text = element.string()?.trim();
                text.parse::<u32>().map_err(|_| {
                    Error::InvalidPixelData(format!("invalid Number of Frames: {text:?}"))
                })?
            }
            None => 1,
        }
        .max(1) as usize;
        if fragments.len() == 1 || number_of_frames == 1 {
            return Ok(vec![fragments.concat()]);
        }
        if fragments.len() == number_of_frames {
            return Ok(fragments.to_vec());
        }
        if fragments.len() < number_of_frames {
            return Err(Error::InvalidPixelData(format!(
                "encapsulated Pixel Data has {} fragments for {number_of_frames} frames",
                fragments.len()
            )));
        }
        let mut frames = Vec::new();
        let mut frame = Vec::new();
        for fragment in fragments {
            frame.extend_from_slice(fragment);
            let tail = &fragment[fragment.len().saturating_sub(10)..];
            if tail.windows(2).any(|pair| pair == [0xFF, 0xD9]) {
                frames.push(std::mem::take(&mut frame));
            }
        }
        if !frame.is_empty() {
            frames.push(frame);
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
        let frame_size = descriptor.stored_frame_size();
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

    /// Decode frames one at a time.
    ///
    /// Only one decoded frame is held at a time, so a long multi-frame image never needs its
    /// decoded form in memory all at once. When a read deferred Pixel Data, each frame is read
    /// from the source as it is asked for, so the stored samples are never all in memory either;
    /// [`ReadOptions::defer_size`] therefore suits the large multi-frame images best of all.
    /// The encoded frames of an encapsulated file are still collected up front, because the
    /// Basic Offset Table has to be read to find their boundaries; those are the compressed
    /// bytes, not the samples.
    pub fn iter_frames(&self) -> Result<Frames<'_>> {
        let descriptor = self.pixel_descriptor()?;
        let syntax = self.transfer_syntax()?;
        if !syntax.is_encapsulated() {
            let length = self
                .deferred_elements()
                .iter()
                .find(|element| element.tag == tags::PIXEL_DATA)
                .map(|element| (element.offset, element.length));
            if let Some((offset, length)) = length {
                return self.stream_native_frames(descriptor, syntax, offset, length);
            }
        }
        let encoded = if syntax.is_encapsulated() {
            Encoded::Encapsulated(self.dataset.encapsulated_frames()?.into_iter())
        } else {
            let bytes = self
                .dataset
                .element(tags::PIXEL_DATA)?
                .value
                .as_bytes()
                .ok_or_else(|| {
                    Error::InvalidPixelData("Pixel Data is not a byte value".to_owned())
                })?;
            let frame_size = descriptor.stored_frame_size();
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
            Encoded::Native(&bytes[..expected])
        };
        Ok(Frames {
            encoded,
            descriptor,
            syntax,
        })
    }

    /// Frames of a native Pixel Data a read left in the file: each is read from the source.
    fn stream_native_frames<'a>(
        &'a self,
        descriptor: PixelDescriptor,
        syntax: TransferSyntax,
        offset: usize,
        length: usize,
    ) -> Result<Frames<'a>> {
        let frame_size = descriptor.stored_frame_size();
        if frame_size == 0 {
            return Err(Error::InvalidPixelData("frame size is zero".to_owned()));
        }
        let expected = frame_size
            .checked_mul(descriptor.number_of_frames as usize)
            .ok_or_else(|| Error::InvalidPixelData("pixel data size overflows".to_owned()))?;
        if length < expected {
            return Err(Error::InvalidPixelData(format!(
                "Pixel Data has {length} bytes, {expected} required for {} frame(s)",
                descriptor.number_of_frames
            )));
        }
        Ok(Frames {
            encoded: Encoded::NativeSource {
                file: self,
                offset,
                remaining: descriptor.number_of_frames as usize,
            },
            descriptor,
            syntax,
        })
    }

    /// Decode one frame and return it as RGB samples.
    ///
    /// A `PALETTE COLOR` frame is mapped through its colour lookup table and the `YBR_FULL`,
    /// `YBR_FULL_422` and `YBR_PARTIAL_422` frames are converted, so the result is displayable
    /// without further work. Monochrome frames come back as the samples they are, one value per
    /// pixel; the JPEG 2000 colour transforms arrive as RGB from the decoder already.
    pub fn decode_rgb_frame(&self, index: usize) -> Result<Vec<u8>> {
        let descriptor = self.pixel_descriptor()?;
        let frame = self.decode_frame(index)?;
        match descriptor.photometric {
            PhotometricInterpretation::Rgb => Ok(frame),
            PhotometricInterpretation::PaletteColor => {
                color::PaletteColorLut::from_dataset(&self.dataset)?
                    .apply(&frame, descriptor.bits_allocated)
            }
            PhotometricInterpretation::YbrFull
            | PhotometricInterpretation::YbrFull422
            | PhotometricInterpretation::YbrPartial422 => {
                color::convert_color_space(&frame, &descriptor, color::ColorSpace::Rgb)
            }
            _ => Ok(frame),
        }
    }

    /// Map one `PALETTE COLOR` frame through its colour lookup table.
    ///
    /// The result has three samples per pixel, one or two bytes each as
    /// [`PaletteColorLut::bits_per_entry`] says.
    pub fn apply_color_lut(&self, index: usize) -> Result<Vec<u8>> {
        let descriptor = self.pixel_descriptor()?;
        if descriptor.photometric != PhotometricInterpretation::PaletteColor {
            return Err(Error::InvalidPixelData(format!(
                "Photometric Interpretation is {}, only PALETTE COLOR has a colour lookup table",
                descriptor.photometric
            )));
        }
        color::PaletteColorLut::from_dataset(&self.dataset)?
            .apply(&self.decode_frame(index)?, descriptor.bits_allocated)
    }

    /// Replace native Pixel Data with samples laid out as the descriptor says.
    ///
    /// `Number of Frames` follows the length of `data`: it is written when the data holds more
    /// than one frame and removed when it holds one. An encapsulated file is refused, because
    /// its transfer syntax would no longer describe the bytes; use
    /// [`decompress`](Self::decompress) to get from compressed frames to native samples.
    pub fn set_pixel_data(&mut self, data: Vec<u8>) -> Result<()> {
        let syntax = self.transfer_syntax()?;
        if syntax.is_encapsulated() {
            return Err(Error::InvalidPixelData(format!(
                "{syntax} holds encapsulated Pixel Data; decompress it before setting native \
                 samples"
            )));
        }
        let descriptor = self.pixel_descriptor()?;
        self.store_native_pixel_data(&descriptor, data)
    }

    /// Decode every frame into native samples and switch the file to Explicit VR Little Endian.
    ///
    /// The codecs that work in RGB hand back RGB, so a colour image compressed with them becomes
    /// `RGB` with an interleaved planar configuration. RLE Lossless keeps the coded samples, so
    /// it keeps its photometric interpretation. pydicom's `decompress` leaves the photometric
    /// interpretation alone in both cases, which leaves the decoded data mislabelled.
    pub fn decompress(&mut self) -> Result<()> {
        let syntax = self.transfer_syntax()?;
        if !syntax.is_encapsulated() {
            return Err(Error::UnsupportedPixelData(format!(
                "{syntax} is not encapsulated"
            )));
        }
        let descriptor = self.pixel_descriptor()?;
        let mut data = Vec::new();
        for frame in self.decode_pixel_data()? {
            data.extend_from_slice(&frame);
        }
        let keeps_samples = matches!(syntax, TransferSyntax::Encapsulated(RLE_LOSSLESS));
        self.meta.set(
            tags::TRANSFER_SYNTAX_UID,
            Vr::Ui,
            Value::from_str(EXPLICIT_VR_LITTLE_ENDIAN),
        );
        // The photometric interpretation has to move to RGB before the samples are stored: it
        // decides how large a frame is, and the decoders expanded the chroma on the way out.
        if !keeps_samples && descriptor.samples_per_pixel > 1 {
            self.dataset.set(
                tags::PHOTOMETRIC_INTERPRETATION,
                Vr::Cs,
                Value::from_str("RGB"),
            );
            self.dataset
                .set(tags::PLANAR_CONFIGURATION, Vr::Us, Value::from_u16s(&[0]));
        }
        // The descriptor is read again: the frame size follows the photometric interpretation
        // that was just updated.
        let stored = self.pixel_descriptor()?;
        self.store_native_pixel_data(&stored, data)?;
        Ok(())
    }

    /// Store `data` as native Pixel Data, keeping the frame count in step with its length.
    fn store_native_pixel_data(
        &mut self,
        descriptor: &PixelDescriptor,
        data: Vec<u8>,
    ) -> Result<()> {
        let frame_size = descriptor.stored_frame_size();
        if frame_size == 0 || data.len() % frame_size != 0 {
            return Err(Error::InvalidPixelData(format!(
                "{} bytes do not divide into frames of {frame_size} bytes",
                data.len()
            )));
        }
        let frames = data.len() / frame_size;
        let vr = if descriptor.bits_allocated > 8 {
            Vr::Ow
        } else {
            Vr::Ob
        };
        self.dataset
            .set(tags::PIXEL_DATA, vr, Value::from_bytes(data));
        if frames > 1 {
            self.dataset.set(
                tags::NUMBER_OF_FRAMES,
                Vr::Is,
                Value::from_str(&frames.to_string()),
            );
        } else {
            self.dataset.remove(tags::NUMBER_OF_FRAMES);
        }
        Ok(())
    }

    /// Replace native Pixel Data with RLE Lossless encapsulated data.
    ///
    /// Every frame is encoded and the transfer syntax in the file meta information becomes RLE
    /// Lossless. An already encapsulated file is refused, because the frames would have to be
    /// decoded and re-encoded and this crate does not re-encode every codec it can decode.
    pub fn compress_rle(&mut self) -> Result<()> {
        let descriptor = self.dataset.pixel_descriptor()?;
        let syntax = self.transfer_syntax()?;
        if syntax.is_encapsulated() {
            return Err(Error::InvalidPixelData(
                "Pixel Data is already encapsulated".to_owned(),
            ));
        }
        let frames = self.pixel_frames()?;
        let mut encoded = Vec::with_capacity(frames.len());
        for frame in &frames {
            encoded.push(rle::encode(frame, &descriptor)?);
        }
        self.set_encapsulated(encoded, RLE_LOSSLESS)
    }

    /// Store `frames` as encapsulated Pixel Data and switch the transfer syntax to `uid`.
    fn set_encapsulated(&mut self, frames: Vec<Vec<u8>>, uid: &str) -> Result<()> {
        let value = crate::encaps::encapsulate(&frames, 1, true)?;
        // Encapsulated Pixel Data is kept as its fragments, so the encapsulated value is split
        // back into the items it is made of.
        let fragments: Vec<Vec<u8>> = crate::encaps::generate_fragments(&value, Endian::Little)?
            .into_iter()
            .map(|fragment| fragment.to_vec())
            .collect();
        self.dataset
            .set(tags::PIXEL_DATA, Vr::Ob, Value::Fragments(fragments));
        self.meta
            .set(tags::TRANSFER_SYNTAX_UID, Vr::Ui, Value::from_str(uid));
        Ok(())
    }
}

/// Iterator returned by [`DicomFile::iter_frames`].
pub struct Frames<'a> {
    encoded: Encoded<'a>,
    descriptor: PixelDescriptor,
    syntax: TransferSyntax,
}

enum Encoded<'a> {
    /// Native Pixel Data: slices of the stored bytes, decoded on the way out.
    Native(&'a [u8]),
    /// Native Pixel Data the read left in the file: each frame is read from the source.
    NativeSource {
        file: &'a DicomFile,
        offset: usize,
        remaining: usize,
    },
    /// Encapsulated Pixel Data: one compressed frame at a time.
    Encapsulated(std::vec::IntoIter<Vec<u8>>),
}

impl Iterator for Frames<'_> {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.encoded {
            Encoded::Native(bytes) => {
                if bytes.is_empty() {
                    return None;
                }
                let frame_size = self.descriptor.stored_frame_size();
                let (frame, rest) = bytes.split_at(frame_size.min(bytes.len()));
                *bytes = rest;
                Some(decode_frame_bytes(self.syntax, &self.descriptor, frame))
            }
            Encoded::NativeSource {
                file,
                offset,
                remaining,
            } => {
                if *remaining == 0 {
                    return None;
                }
                *remaining -= 1;
                let frame_size = self.descriptor.stored_frame_size();
                let frame = match file.source_range(*offset, frame_size) {
                    Ok(frame) => frame,
                    Err(err) => {
                        *remaining = 0;
                        return Some(Err(err));
                    }
                };
                *offset += frame_size;
                Some(decode_frame_bytes(self.syntax, &self.descriptor, &frame))
            }
            Encoded::Encapsulated(frames) => {
                let frame = frames.next()?;
                Some(decode_frame_bytes(self.syntax, &self.descriptor, &frame))
            }
        }
    }
}

#[cfg(feature = "pixel-jpeg")]
impl DicomFile {
    /// Replace native 8-bit Pixel Data with baseline JPEG.
    ///
    /// `quality` is the encoder's 1-100 scale. A colour frame is written as YCbCr, so its
    /// photometric interpretation becomes `YBR_FULL_422`.
    pub fn compress_jpeg(&mut self, quality: u8) -> Result<()> {
        use jpeg_encoder::{ColorType, Encoder};

        let descriptor = self.dataset.pixel_descriptor()?;
        if self.transfer_syntax()?.is_encapsulated() {
            return Err(Error::InvalidPixelData(
                "Pixel Data is already encapsulated".to_owned(),
            ));
        }
        if descriptor.bits_allocated != 8 {
            return Err(Error::InvalidPixelData(
                "baseline JPEG writes 8-bit samples only".to_owned(),
            ));
        }
        let colour = descriptor.samples_per_pixel == 3;
        let color = match descriptor.samples_per_pixel {
            1 => ColorType::Luma,
            3 => ColorType::Rgb,
            samples => {
                return Err(Error::InvalidPixelData(format!(
                    "baseline JPEG needs one or three samples, this image has {samples}"
                )))
            }
        };
        let frames = self.pixel_frames()?;
        let mut encoded = Vec::with_capacity(frames.len());
        for frame in &frames {
            let mut out = Vec::new();
            Encoder::new(&mut out, quality)
                .encode(frame, descriptor.columns, descriptor.rows, color)
                .map_err(|err| Error::InvalidPixelData(format!("JPEG encoder: {err}")))?;
            encoded.push(out);
        }
        self.set_encapsulated(encoded, "1.2.840.10008.1.2.4.50")?;
        if colour {
            self.dataset
                .set_string(tags::PHOTOMETRIC_INTERPRETATION, Vr::Cs, "YBR_FULL_422");
        }
        Ok(())
    }
}

#[cfg(feature = "pixel-jpegls")]
impl DicomFile {
    /// Replace native Pixel Data with JPEG-LS Lossless.
    pub fn compress_jpegls(&mut self) -> Result<()> {
        use charls::{CharLS, FrameInfo, InterleaveMode};

        let descriptor = self.dataset.pixel_descriptor()?;
        if self.transfer_syntax()?.is_encapsulated() {
            return Err(Error::InvalidPixelData(
                "Pixel Data is already encapsulated".to_owned(),
            ));
        }
        if !matches!(descriptor.bits_allocated, 8 | 16) {
            return Err(Error::InvalidPixelData(
                "JPEG-LS writes 8- and 16-bit samples".to_owned(),
            ));
        }
        if !matches!(descriptor.samples_per_pixel, 1 | 3) {
            return Err(Error::InvalidPixelData(
                "JPEG-LS needs one or three samples".to_owned(),
            ));
        }
        let frames = self.pixel_frames()?;
        let mut encoded = Vec::with_capacity(frames.len());
        for frame in &frames {
            let mut encoder = CharLS::default();
            if descriptor.samples_per_pixel > 1 {
                encoder
                    .set_interleave_mode(InterleaveMode::Sample)
                    .map_err(|err| Error::InvalidPixelData(format!("JPEG-LS encoder: {err}")))?;
            }
            let compressed = encoder
                .encode(
                    FrameInfo {
                        width: u32::from(descriptor.columns),
                        height: u32::from(descriptor.rows),
                        bits_per_sample: i32::from(descriptor.bits_allocated),
                        component_count: i32::from(descriptor.samples_per_pixel),
                    },
                    0,
                    frame,
                )
                .map_err(|err| Error::InvalidPixelData(format!("JPEG-LS encoder: {err}")))?;
            encoded.push(compressed);
        }
        self.set_encapsulated(encoded, "1.2.840.10008.1.2.4.80")
    }
}

#[cfg(feature = "pixel-j2k")]
impl DicomFile {
    /// Replace native Pixel Data with JPEG 2000 Lossless.
    ///
    /// 8- and 16-bit samples are written as one codestream per frame; the reversible 5-3
    /// transform keeps the samples exact.
    pub fn compress_j2k(&mut self) -> Result<()> {
        use jpeg2k::{ColorSpace, CoreImage, CoreImageComp, EncodeParameters, Image};

        let descriptor = self.dataset.pixel_descriptor()?;
        if self.transfer_syntax()?.is_encapsulated() {
            return Err(Error::InvalidPixelData(
                "Pixel Data is already encapsulated".to_owned(),
            ));
        }
        if !matches!(descriptor.bits_allocated, 8 | 16) {
            return Err(Error::InvalidPixelData(
                "JPEG 2000 writes 8- and 16-bit samples".to_owned(),
            ));
        }
        let samples = usize::from(descriptor.samples_per_pixel);
        let bytes_per_sample = descriptor.bytes_per_sample();
        let pixel_count = usize::from(descriptor.rows) * usize::from(descriptor.columns);
        let signed = descriptor.pixel_representation == PixelRepresentation::Signed;
        let planar = descriptor.planar_configuration == PlanarConfiguration::Planar;
        // A DWT cannot split a small image as many times as the default six resolutions allow.
        let mut resolutions = 1u32;
        while resolutions < 6
            && (1u32 << resolutions) <= u32::from(descriptor.columns.min(descriptor.rows))
        {
            resolutions += 1;
        }
        let frames = self.pixel_frames()?;
        let mut encoded = Vec::with_capacity(frames.len());
        for frame in &frames {
            let comps = (0..samples)
                .map(|sample| {
                    let data = (0..pixel_count)
                        .map(|pixel| {
                            let index = if planar {
                                (sample * pixel_count + pixel) * bytes_per_sample
                            } else {
                                (pixel * samples + sample) * bytes_per_sample
                            };
                            match (bytes_per_sample, signed) {
                                (1, false) => i32::from(frame[index]),
                                (1, true) => i32::from(frame[index] as i8),
                                (_, false) => {
                                    i32::from(u16::from_le_bytes([frame[index], frame[index + 1]]))
                                }
                                (_, true) => {
                                    i32::from(i16::from_le_bytes([frame[index], frame[index + 1]]))
                                }
                            }
                        })
                        .collect();
                    CoreImageComp {
                        dx: 1,
                        dy: 1,
                        w: u32::from(descriptor.columns),
                        h: u32::from(descriptor.rows),
                        x0: 0,
                        y0: 0,
                        prec: u32::from(descriptor.bits_stored),
                        bpp: u32::from(descriptor.bits_allocated),
                        sgnd: u32::from(signed),
                        resno_decoded: 0,
                        factor: 0,
                        alpha: 0,
                        data,
                    }
                })
                .collect();
            let image = Image::from_core_image(CoreImage {
                x0: 0,
                y0: 0,
                x1: u32::from(descriptor.columns),
                y1: u32::from(descriptor.rows),
                numcomps: u32::from(descriptor.samples_per_pixel),
                color_space: if samples >= 3 {
                    ColorSpace::Srgb
                } else {
                    ColorSpace::Gray
                },
                comps,
                icc_profile_buf: Vec::new(),
                icc_profile_is_cielab: false,
            });
            encoded.push(
                image
                    .encode(EncodeParameters::new().num_resolutions(resolutions))
                    .map_err(|err| Error::InvalidPixelData(format!("JPEG 2000 encoder: {err}")))?,
            );
        }
        self.set_encapsulated(encoded, "1.2.840.10008.1.2.4.90")
    }
}

#[cfg(feature = "pixel-jxl")]
impl DicomFile {
    /// Replace native Pixel Data with JPEG XL Lossless.
    ///
    /// `zune-jpegxl` encodes 8- and 16-bit gray or colour frames losslessly and writes
    /// the bare codestream the transfer syntax carries.
    pub fn compress_jxl(&mut self) -> Result<()> {
        use zune_core::bit_depth::BitDepth;
        use zune_core::colorspace::ColorSpace;
        use zune_core::options::EncoderOptions;
        use zune_jpegxl::JxlSimpleEncoder;

        let descriptor = self.dataset.pixel_descriptor()?;
        if self.transfer_syntax()?.is_encapsulated() {
            return Err(Error::InvalidPixelData(
                "Pixel Data is already encapsulated".to_owned(),
            ));
        }
        let (space, depth) = match (descriptor.samples_per_pixel, descriptor.bits_allocated) {
            (1, 8) => (ColorSpace::Luma, BitDepth::Eight),
            (3, 8) => (ColorSpace::RGB, BitDepth::Eight),
            (1, 16) => (ColorSpace::Luma, BitDepth::Sixteen),
            (3, 16) => (ColorSpace::RGB, BitDepth::Sixteen),
            (samples, bits) => {
                return Err(Error::InvalidPixelData(format!(
                    "JPEG XL writes 8- and 16-bit gray or colour, this image has {samples} \
                     sample(s) of {bits} bits"
                )))
            }
        };
        let width = usize::from(descriptor.columns);
        let height = usize::from(descriptor.rows);
        let frame_len = width
            * usize::from(descriptor.samples_per_pixel)
            * height
            * (usize::from(descriptor.bits_allocated) / 8);
        let frames = self.pixel_frames()?;
        let mut encoded = Vec::with_capacity(frames.len());
        for frame in &frames {
            let samples = frame.get(..frame_len).ok_or_else(|| {
                Error::InvalidPixelData(format!(
                    "frame holds {} bytes, JPEG XL needs {frame_len}",
                    frame.len()
                ))
            })?;
            let mut codestream = Vec::new();
            JxlSimpleEncoder::new(samples, EncoderOptions::new(width, height, space, depth))
                .encode(&mut codestream)
                .map_err(|err| Error::InvalidPixelData(format!("JPEG XL encoder: {err}")))?;
            encoded.push(codestream);
        }
        self.set_encapsulated(encoded, "1.2.840.10008.1.2.4.110")
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
        if is_jpeg_lossless_uid(uid) {
            return jpeg_lossless::decode(frame, descriptor);
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

/// JPEG Lossless (Process 14) and Process 14 Selection Value 1 (`.57`, `.70`).
fn is_jpeg_lossless_uid(uid: &str) -> bool {
    matches!(uid, "1.2.840.10008.1.2.4.57" | "1.2.840.10008.1.2.4.70")
}

/// JPEG Baseline (Process 1), Extended (Process 2/4), Spectral Selection and Full Progression.
///
/// JPEG Lossless is decoded by [`jpeg_lossless`] and JPEG-LS is a different codec.
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
