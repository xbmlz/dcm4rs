//! MPEG-2 sequence headers, and nothing beyond them.
//!
//! DICOM carries video in encapsulated Pixel Data (PS3.5 Annex A.4): the fragments *are* the
//! frames, and a viewer needs Rows, Columns, Cine Rate, Frame Time and Pixel Aspect Ratio to show
//! them. Those five are recorded in the sequence header (ISO/IEC 13818-2, 6.2.2.3) at the front
//! of the codestream, so they can be recovered without decoding anything.
//!
//! This mirrors what the reference implementations do. dcm4che's `MPEG2Parser`/`MPEGHeader` read
//! the same header and fill the same attributes; pydicom and fo-dicom stop one step earlier and
//! only recognize the transfer syntax. None of them decodes the video, and neither does this
//! module: [`DataSet::encapsulated_frames`](crate::DataSet::encapsulated_frames) hands the frames
//! out as the bitstream carries them, and turning them into pixels is the caller's business.

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::vr::Vr;

/// `sequence_header_code`, the start code that introduces a sequence header.
const SEQUENCE_HEADER_CODE: [u8; 4] = [0x00, 0x00, 0x01, 0xB3];

/// The fields of an MPEG-2 sequence header that map onto DICOM attributes.
///
/// The two code fields are kept as codes rather than resolved here: [`Self::frame_rate`] and
/// [`Self::aspect_ratio`] return `None` for the reserved values instead of inventing a rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SequenceHeader {
    /// `horizontal_size_value`: the encoded width in pixels.
    pub columns: u16,
    /// `vertical_size_value`: the encoded height in pixels.
    pub rows: u16,
    /// `aspect_ratio_information` (table 6-3).
    pub aspect_ratio_code: u8,
    /// `frame_rate_code` (table 6-4).
    pub frame_rate_code: u8,
}

/// A frame rate as a rational number of frames per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRate {
    /// Frames per second's numerator; 24000 of `24000/1001`.
    pub numerator: u32,
    /// Frames per second's denominator; 1001 of `24000/1001`.
    pub denominator: u32,
}

/// A display aspect ratio as a rational number, width over height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AspectRatio {
    /// Width of the display aspect ratio; 16 of `16:9`.
    pub numerator: u16,
    /// Height of the display aspect ratio; 9 of `16:9`.
    pub denominator: u16,
}

impl FrameRate {
    /// Frames per second as a float.
    pub fn as_f64(self) -> f64 {
        f64::from(self.numerator) / f64::from(self.denominator)
    }

    /// Frames per second rounded to the nearest whole number, as `Cine Rate` records it.
    pub fn rounded(self) -> u32 {
        self.as_f64().round() as u32
    }

    /// One frame's duration in milliseconds, as `Frame Time` records it.
    pub fn frame_time_ms(self) -> f64 {
        1000.0 / self.as_f64()
    }
}

impl SequenceHeader {
    /// Read the sequence header from a codestream, `None` when it carries none.
    ///
    /// The header is searched for rather than assumed to start at byte zero: a frame may open
    /// with a GOP header or with a sequence end code before the sequence header appears.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let start = bytes
            .windows(SEQUENCE_HEADER_CODE.len())
            .position(|window| window == SEQUENCE_HEADER_CODE)?;
        let data = bytes.get(start + 4..start + 8)?;
        Some(Self {
            columns: (u16::from(data[0]) << 4) | (u16::from(data[1]) >> 4),
            rows: ((u16::from(data[1]) & 0x0F) << 8) | u16::from(data[2]),
            aspect_ratio_code: data[3] >> 4,
            frame_rate_code: data[3] & 0x0F,
        })
    }

    /// The frame rate, `None` for the reserved codes.
    pub fn frame_rate(self) -> Option<FrameRate> {
        let (numerator, denominator) = match self.frame_rate_code {
            1 => (24000, 1001),
            2 => (24, 1),
            3 => (25, 1),
            4 => (30000, 1001),
            5 => (30, 1),
            6 => (50, 1),
            7 => (60000, 1001),
            8 => (60, 1),
            _ => return None,
        };
        Some(FrameRate {
            numerator,
            denominator,
        })
    }

    /// The display aspect ratio, `None` when the code is forbidden or reserved.
    pub fn aspect_ratio(self) -> Option<AspectRatio> {
        let (numerator, denominator) = match self.aspect_ratio_code {
            1 => (1, 1),
            2 => (4, 3),
            3 => (16, 9),
            4 => (221, 100),
            _ => return None,
        };
        Some(AspectRatio {
            numerator,
            denominator,
        })
    }

    /// The pixel aspect ratio the display aspect ratio implies, `None` for square pixels.
    ///
    /// `Pixel Aspect Ratio` (0028,0034) describes a pixel, not the picture, so a display ratio of
    /// 16:9 at 720x576 works out to 64:45 and a 1:1 display ratio is square. The ratio is reduced
    /// by its greatest common divisor; a square one is omitted, as PS3.3 asks.
    pub fn pixel_aspect_ratio(self) -> Option<(u16, u16)> {
        let display = self.aspect_ratio()?;
        let width = u32::from(display.numerator) * u32::from(self.rows);
        let height = u32::from(display.denominator) * u32::from(self.columns);
        let divisor = gcd(width, height);
        if divisor == 0 {
            return None;
        }
        let (width, height) = (width / divisor, height / divisor);
        if width == height || width > u32::from(u16::MAX) || height > u32::from(u16::MAX) {
            return None;
        }
        Some((width as u16, height as u16))
    }

    /// Fill the attributes the header can supply, leaving present values untouched.
    ///
    /// A data set that already records Rows, Columns, Cine Rate, Frame Time or Pixel Aspect Ratio
    /// keeps them: the header describes how the video was encoded, the data set is authoritative
    /// about how it is meant to be shown.
    pub fn apply(self, dataset: &mut DataSet) {
        if !dataset.contains(tags::ROWS) {
            dataset.set_u16(tags::ROWS, self.rows);
        }
        if !dataset.contains(tags::COLUMNS) {
            dataset.set_u16(tags::COLUMNS, self.columns);
        }
        if let Some(frame_rate) = self.frame_rate() {
            if !dataset.contains(tags::CINE_RATE) {
                dataset.set_string(tags::CINE_RATE, Vr::Is, &frame_rate.rounded().to_string());
            }
            if !dataset.contains(tags::FRAME_TIME) {
                dataset.set_string(
                    tags::FRAME_TIME,
                    Vr::Ds,
                    &format!("{:.6}", frame_rate.frame_time_ms()),
                );
            }
        }
        if let Some((width, height)) = self.pixel_aspect_ratio() {
            if !dataset.contains(tags::PIXEL_ASPECT_RATIO) {
                dataset.set_string(
                    tags::PIXEL_ASPECT_RATIO,
                    Vr::Is,
                    &format!("{width}\\{height}"),
                );
            }
        }
    }
}

impl DataSet {
    /// The MPEG-2 sequence header of this data set's encapsulated Pixel Data.
    ///
    /// The header is looked for in every frame, not only the first: a stream that opens with a
    /// thumbnail or a still frame may carry the sequence header later. `None` means no frame
    /// held one, which is also what an H.264 or HEVC stream returns — those are not MPEG-2 and
    /// this module does not read their parameter sets.
    pub fn mpeg2_sequence_header(&self) -> crate::Result<Option<SequenceHeader>> {
        // Native Pixel Data has no fragments and therefore no video: a JPEG or RLE data set
        // asking this question gets an answer, not an error.
        if self
            .element(tags::PIXEL_DATA)?
            .value
            .as_fragments()
            .is_none()
        {
            return Ok(None);
        }
        for frame in self.encapsulated_frames()? {
            if let Some(header) = SequenceHeader::parse(&frame) {
                return Ok(Some(header));
            }
        }
        Ok(None)
    }

    /// Parse the MPEG-2 sequence header and record what it knows in this data set.
    ///
    /// Returns the header that was found, `None` when there was none to apply. Values already
    /// present are kept; see [`SequenceHeader::apply`].
    pub fn apply_mpeg2_sequence_header(&mut self) -> crate::Result<Option<SequenceHeader>> {
        let header = self.mpeg2_sequence_header()?;
        if let Some(header) = header {
            header.apply(self);
        }
        Ok(header)
    }
}

/// Greatest common divisor, zero when both inputs are zero.
fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sequence header for 512x512, display ratio 1:1, 25 frames per second.
    const SQUARE_512: &[u8] = b"\x00\x00\x01\xb3\x20\x02\x00\x13";

    #[test]
    fn reads_the_size_and_codes() {
        let header = SequenceHeader::parse(SQUARE_512).expect("header");
        assert_eq!(header.columns, 512);
        assert_eq!(header.rows, 512);
        assert_eq!(header.aspect_ratio_code, 1);
        assert_eq!(header.frame_rate_code, 3);
    }

    #[test]
    fn resolves_the_frame_rate() {
        let header = SequenceHeader::parse(SQUARE_512).expect("header");
        let rate = header.frame_rate().expect("frame rate");
        assert_eq!(rate.numerator, 25);
        assert_eq!(rate.denominator, 1);
        assert_eq!(rate.rounded(), 25);
        assert!((rate.frame_time_ms() - 40.0).abs() < 1e-9);
    }

    #[test]
    fn thirty_frames_is_really_29_97() {
        let header = SequenceHeader {
            aspect_ratio_code: 1,
            frame_rate_code: 5,
            ..SequenceHeader::parse(SQUARE_512).expect("header")
        };
        let rate = header.frame_rate().expect("frame rate");
        assert_eq!(rate.numerator, 30);
        assert_eq!(rate.denominator, 1);
        assert_eq!(header.frame_rate_code, 5);
        let ntsc = SequenceHeader {
            frame_rate_code: 4,
            ..header
        };
        let rate = ntsc.frame_rate().expect("frame rate");
        assert_eq!(rate.numerator, 30000);
        assert_eq!(rate.denominator, 1001);
        assert_eq!(rate.rounded(), 30);
    }

    #[test]
    fn reserved_codes_resolve_to_nothing() {
        let header = SequenceHeader {
            aspect_ratio_code: 0,
            frame_rate_code: 15,
            ..SequenceHeader::parse(SQUARE_512).expect("header")
        };
        assert!(header.frame_rate().is_none());
        assert!(header.aspect_ratio().is_none());
        assert!(header.pixel_aspect_ratio().is_none());
    }

    #[test]
    fn sixteen_by_nine_at_720x576_is_64_by_45() {
        // 720x576, display ratio 16:9, 25 frames per second.
        let bytes = b"\x00\x00\x01\xb3\x2d\x02\x40\x33";
        let header = SequenceHeader::parse(bytes).expect("header");
        assert_eq!(header.columns, 720);
        assert_eq!(header.rows, 576);
        assert_eq!(header.pixel_aspect_ratio(), Some((64, 45)));
    }

    #[test]
    fn square_pixels_are_omitted() {
        let header = SequenceHeader::parse(SQUARE_512).expect("header");
        assert_eq!(header.pixel_aspect_ratio(), None);
    }

    #[test]
    fn a_codestream_without_a_sequence_header_is_none() {
        assert!(SequenceHeader::parse(b"\x00\x00\x01\xb1").is_none());
        assert!(SequenceHeader::parse(b"").is_none());
        // A start code with fewer than four bytes behind it cannot be measured.
        assert!(SequenceHeader::parse(b"\x00\x00\x01\xb3\x20\x02").is_none());
    }
}
