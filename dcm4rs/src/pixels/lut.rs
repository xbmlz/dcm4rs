//! Pixel value transforms: modality LUT and VOI LUT (PS3.3 C.11.2, C.11.6).
//!
//! These mirror pydicom's `apply_modality_lut`, `apply_voi_lut` and `apply_windowing`: a stored
//! sample is first mapped through Rescale Slope and Intercept (or the Modality LUT Sequence),
//! then through the VOI LUT Sequence or Window Center and Width. The transforms are applied one
//! frame at a time and never change the stored pixel data.

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::error::{Error, Result};
use crate::file::DicomFile;
use crate::pixels::{PixelDescriptor, PixelRepresentation};
use crate::tag::Tag;

/// The output range of a linear window, as PS3.3 C.11.2.1.2 fixes it for 8-bit display.
const WINDOW_MIN: f64 = 0.0;
const WINDOW_MAX: f64 = 255.0;

impl DicomFile {
    /// One frame's samples with the modality transform applied.
    ///
    /// A Modality LUT Sequence wins over Rescale Slope and Intercept; without either, the stored
    /// values come back unchanged.
    pub fn apply_modality_lut(&self, frame: usize) -> Result<Vec<f64>> {
        let descriptor = self.pixel_descriptor()?;
        let samples = samples(&descriptor, &self.decode_frame(frame)?)?;
        if let Some(lut) = modality_lut(&self.dataset)? {
            return Ok(samples.iter().map(|x| lut.lookup(*x as f64)).collect());
        }
        let (slope, intercept) = rescale(&self.dataset)?;
        Ok(samples
            .into_iter()
            .map(|x| x as f64 * slope + intercept)
            .collect())
    }

    /// One frame's samples mapped to `0..=255` by Window Center and Width.
    pub fn apply_windowing(&self, frame: usize) -> Result<Vec<u8>> {
        let descriptor = self.pixel_descriptor()?;
        let samples = samples(&descriptor, &self.decode_frame(frame)?)?;
        let (slope, intercept) = rescale(&self.dataset)?;
        let (center, width) = window(&self.dataset)?;
        Ok(samples
            .into_iter()
            .map(|x| window_value(x as f64 * slope + intercept, center, width))
            .collect())
    }

    /// One frame's samples mapped by the VOI transform.
    ///
    /// A VOI LUT Sequence is used when present, Window Center and Width otherwise, as pydicom's
    /// `apply_voi_lut` does. The result is not rounded, so a LUT with more than 256 entries keeps
    /// its resolution.
    pub fn apply_voi_lut(&self, frame: usize) -> Result<Vec<f64>> {
        let descriptor = self.pixel_descriptor()?;
        let samples = samples(&descriptor, &self.decode_frame(frame)?)?;
        let (slope, intercept) = rescale(&self.dataset)?;
        let mapped: Vec<f64> = samples
            .iter()
            .map(|x| *x as f64 * slope + intercept)
            .collect();

        if let Some(lut) = voi_lut(&self.dataset)? {
            return Ok(mapped.iter().map(|x| lut.lookup(*x)).collect());
        }
        let (center, width) = window(&self.dataset)?;
        Ok(mapped
            .into_iter()
            .map(|x| window_value(x, center, width) as f64)
            .collect())
    }
}

/// Stored samples of one frame as signed integers; the descriptor decides the width.
fn samples(descriptor: &PixelDescriptor, frame: &[u8]) -> Result<Vec<i64>> {
    let signed = descriptor.pixel_representation == PixelRepresentation::Signed;
    match descriptor.bits_allocated {
        8 => Ok(if signed {
            frame.iter().map(|b| i64::from(*b as i8)).collect()
        } else {
            frame.iter().map(|b| i64::from(*b)).collect()
        }),
        16 => Ok(if signed {
            frame
                .chunks_exact(2)
                .map(|pair| i64::from(i16::from_le_bytes([pair[0], pair[1]])))
                .collect()
        } else {
            frame
                .chunks_exact(2)
                .map(|pair| i64::from(u16::from_le_bytes([pair[0], pair[1]])))
                .collect()
        }),
        32 => Ok(if signed {
            frame
                .chunks_exact(4)
                .map(|quad| i64::from(i32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]])))
                .collect()
        } else {
            frame
                .chunks_exact(4)
                .map(|quad| i64::from(u32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]])))
                .collect()
        }),
        bits => Err(Error::InvalidPixelData(format!(
            "Bits Allocated {bits} has no supported sample width"
        ))),
    }
}

/// Rescale Slope and Intercept, defaulting to the identity transform.
fn rescale(set: &DataSet) -> Result<(f64, f64)> {
    let slope = match set.get(tags::RESCALE_SLOPE) {
        Some(element) => element
            .string()?
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::InvalidPixelData("invalid Rescale Slope".to_owned()))?,
        None => 1.0,
    };
    let intercept = match set.get(tags::RESCALE_INTERCEPT) {
        Some(element) => element
            .string()?
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::InvalidPixelData("invalid Rescale Intercept".to_owned()))?,
        None => 0.0,
    };
    Ok((slope, intercept))
}

/// Window Center and Width, the first value of each when the element is multi-valued.
///
/// Both are `DS`, so the values are read as decimal text rather than as binary floats.
fn window(set: &DataSet) -> Result<(f64, f64)> {
    let center = first_decimal(set, tags::WINDOW_CENTER, "Window Center")?;
    let width = first_decimal(set, tags::WINDOW_WIDTH, "Window Width")?;
    Ok((center, width))
}

fn first_decimal(set: &DataSet, tag: Tag, name: &str) -> Result<f64> {
    let missing = || Error::InvalidPixelData(format!("no valid {name} for windowing"));
    let element = set.element(tag).map_err(|_| missing())?;
    element
        .decimal_values()?
        .into_iter()
        .next()
        .ok_or_else(missing)
}

/// The linear window of PS3.3 C.11.2.1.2 for an 8-bit output range.
fn window_value(value: f64, center: f64, width: f64) -> u8 {
    // PS3.3 treats a width below 1 as 1; anything narrower would divide by zero.
    let width = width.max(1.0);
    let low = center - (width - 1.0) / 2.0 - 0.5;
    let high = center + (width - 1.0) / 2.0 + 0.5;
    if value <= low {
        return WINDOW_MIN as u8;
    }
    if value > high {
        return WINDOW_MAX as u8;
    }
    let scaled = (value - (center - 0.5)) / (width - 1.0);
    let mapped = (scaled + 0.5) * (WINDOW_MAX - WINDOW_MIN) + WINDOW_MIN;
    mapped.round().clamp(WINDOW_MIN, WINDOW_MAX) as u8
}

/// A resolved lookup table: the descriptor's three numbers and the entries themselves.
struct LookupTable {
    first: i64,
    entries: Vec<f64>,
}

impl LookupTable {
    /// Map one value; anything outside the table's range takes the nearest entry, which is what
    /// a viewer does with a truncated LUT.
    fn lookup(&self, value: f64) -> f64 {
        let index = value.round() as i64 - self.first;
        if index < 0 {
            return self.entries.first().copied().unwrap_or(0.0);
        }
        let index = index as usize;
        self.entries
            .get(index)
            .copied()
            .unwrap_or_else(|| self.entries.last().copied().unwrap_or(0.0))
    }
}

/// The Modality LUT Sequence (`(0028,3000)`), one item taken.
fn modality_lut(set: &DataSet) -> Result<Option<LookupTable>> {
    read_lut(
        set,
        Tag::new(0x0028, 0x3000),
        Tag::new(0x0028, 0x3002),
        Tag::new(0x0028, 0x3006),
    )
}

/// The VOI LUT Sequence (`(0028,3010)`), one item taken.
fn voi_lut(set: &DataSet) -> Result<Option<LookupTable>> {
    read_lut(
        set,
        Tag::new(0x0028, 0x3010),
        Tag::new(0x0028, 0x3002),
        Tag::new(0x0028, 0x3006),
    )
}

/// Read a LUT Sequence's first item: descriptor `(0028,3002)` and data `(0028,3006)`.
fn read_lut(
    set: &DataSet,
    sequence: Tag,
    descriptor: Tag,
    data: Tag,
) -> Result<Option<LookupTable>> {
    let Some(element) = set.get(sequence) else {
        return Ok(None);
    };
    let items = element.as_sequence()?;
    let Some(item) = items.first() else {
        return Ok(None);
    };
    let numbers = item.u16s(descriptor)?;
    let [count, first, bits] = numbers.as_slice() else {
        return Err(Error::InvalidPixelData(format!(
            "{descriptor} needs three values, found {}",
            numbers.len()
        )));
    };
    let count = usize::from(*count);
    let first = i64::from(*first as i16);
    let entries: Vec<f64> = match *bits {
        8 => item
            .element(data)?
            .raw_value()
            .iter()
            .map(|byte| f64::from(*byte))
            .collect(),
        16 => item
            .element(data)?
            .raw_value()
            .chunks_exact(2)
            .map(|pair| f64::from(u16::from_le_bytes([pair[0], pair[1]])))
            .collect(),
        other => {
            return Err(Error::InvalidPixelData(format!(
                "LUT entries of {other} bits are not supported"
            )))
        }
    };
    let entries: Vec<f64> = entries.into_iter().take(count).collect();
    Ok(Some(LookupTable { first, entries }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::tags;
    use crate::pixels::PhotometricInterpretation;
    use crate::transfer_syntax::TransferSyntax;
    use crate::value::Value;
    use crate::vr::Vr;

    /// A data set that carries the two UIDs `DicomFile::new` derives its meta information from.
    fn base_set() -> DataSet {
        let mut set = DataSet::new();
        set.set_string(tags::SOP_CLASS_UID, Vr::Ui, "1.2.840.10008.5.1.4.1.1.2");
        set.set_string(tags::SOP_INSTANCE_UID, Vr::Ui, "1.2.3");
        set
    }

    /// A one-frame, 16-bit unsigned image holding `values`.
    fn image(values: &[u16]) -> DicomFile {
        let mut set = base_set();
        set.set(Tag::new(0x0028, 0x0002), Vr::Us, Value::from_u16s(&[1]));
        set.set(
            Tag::new(0x0028, 0x0004),
            Vr::Cs,
            Value::from_str("MONOCHROME2"),
        );
        set.set(Tag::new(0x0028, 0x0010), Vr::Us, Value::from_u16s(&[1]));
        set.set(
            Tag::new(0x0028, 0x0011),
            Vr::Us,
            Value::from_u16s(&[values.len() as u16]),
        );
        set.set(Tag::new(0x0028, 0x0100), Vr::Us, Value::from_u16s(&[16]));
        set.set(Tag::new(0x0028, 0x0101), Vr::Us, Value::from_u16s(&[16]));
        set.set(Tag::new(0x0028, 0x0102), Vr::Us, Value::from_u16s(&[15]));
        set.set(Tag::new(0x0028, 0x0103), Vr::Us, Value::from_u16s(&[0]));
        set.set(tags::PIXEL_DATA, Vr::Ow, Value::from_u16s(values));
        DicomFile::new(set, TransferSyntax::ExplicitVrLittleEndian).expect("file")
    }

    #[test]
    fn rescale_slope_and_intercept_are_applied() {
        let mut file = image(&[0, 100, 400]);
        file.dataset.set_string(tags::RESCALE_SLOPE, Vr::Ds, "0.5");
        file.dataset
            .set_string(tags::RESCALE_INTERCEPT, Vr::Ds, "-100");
        let values = file.apply_modality_lut(0).expect("lut");
        assert_eq!(values, vec![-100.0, -50.0, 100.0]);
    }

    #[test]
    fn without_rescale_the_samples_come_back_unchanged() {
        let file = image(&[1, 2, 3]);
        assert_eq!(
            file.apply_modality_lut(0).expect("lut"),
            vec![1.0, 2.0, 3.0]
        );
    }

    #[test]
    fn a_window_maps_to_the_eight_bit_range() {
        let mut file = image(&[0, 128, 255, 511]);
        file.dataset.set_string(tags::WINDOW_CENTER, Vr::Ds, "256");
        file.dataset.set_string(tags::WINDOW_WIDTH, Vr::Ds, "512");
        let values = file.apply_windowing(0).expect("window");
        // 0 is at the window's lower edge, 128 a quarter of the way in, 255 just under the
        // centre, 511 past the upper edge.
        assert_eq!(values, vec![0, 64, 127, 255]);
    }

    #[test]
    fn windowing_refuses_a_file_without_a_window() {
        let file = image(&[0]);
        assert!(file.apply_windowing(0).is_err());
    }

    #[test]
    fn a_voi_lut_sequence_is_preferred_over_the_window() {
        let mut file = image(&[0, 1, 2, 3]);
        file.dataset.set_string(tags::WINDOW_CENTER, Vr::Ds, "1");
        file.dataset.set_string(tags::WINDOW_WIDTH, Vr::Ds, "1");

        let mut item = DataSet::new();
        item.set(
            Tag::new(0x0028, 0x3002),
            Vr::Us,
            Value::from_u16s(&[4, 0, 16]),
        );
        item.set(
            Tag::new(0x0028, 0x3006),
            Vr::Ow,
            Value::from_u16s(&[10, 20, 30, 40]),
        );
        file.dataset.set(
            Tag::new(0x0028, 0x3010),
            Vr::Sq,
            Value::sequence(vec![item]),
        );

        assert_eq!(
            file.apply_voi_lut(0).expect("voi"),
            vec![10.0, 20.0, 30.0, 40.0]
        );
    }

    #[test]
    fn a_voi_lut_clamps_outside_its_range() {
        let lut = LookupTable {
            first: 0,
            entries: vec![5.0, 6.0, 7.0],
        };
        assert_eq!(lut.lookup(-3.0), 5.0);
        assert_eq!(lut.lookup(99.0), 7.0);
        assert_eq!(lut.lookup(1.0), 6.0);
    }

    #[test]
    fn a_modality_lut_maps_the_samples() {
        let mut file = image(&[0, 1, 2]);
        let mut item = DataSet::new();
        item.set(
            Tag::new(0x0028, 0x3002),
            Vr::Us,
            Value::from_u16s(&[3, 0, 16]),
        );
        item.set(
            Tag::new(0x0028, 0x3006),
            Vr::Ow,
            Value::from_u16s(&[7, 8, 9]),
        );
        file.dataset.set(
            Tag::new(0x0028, 0x3000),
            Vr::Sq,
            Value::sequence(vec![item]),
        );
        assert_eq!(
            file.apply_modality_lut(0).expect("lut"),
            vec![7.0, 8.0, 9.0]
        );
    }

    #[test]
    fn a_signed_sample_keeps_its_sign() {
        let mut set = base_set();
        set.set(Tag::new(0x0028, 0x0002), Vr::Us, Value::from_u16s(&[1]));
        set.set(
            Tag::new(0x0028, 0x0004),
            Vr::Cs,
            Value::from_str("MONOCHROME2"),
        );
        set.set(Tag::new(0x0028, 0x0010), Vr::Us, Value::from_u16s(&[1]));
        set.set(Tag::new(0x0028, 0x0011), Vr::Us, Value::from_u16s(&[2]));
        set.set(Tag::new(0x0028, 0x0100), Vr::Us, Value::from_u16s(&[16]));
        set.set(Tag::new(0x0028, 0x0101), Vr::Us, Value::from_u16s(&[16]));
        set.set(Tag::new(0x0028, 0x0102), Vr::Us, Value::from_u16s(&[15]));
        set.set(Tag::new(0x0028, 0x0103), Vr::Us, Value::from_u16s(&[1]));
        set.set(tags::PIXEL_DATA, Vr::Ow, Value::from_i16s(&[-1, 2]));
        let file = DicomFile::new(set, TransferSyntax::ExplicitVrLittleEndian).expect("file");
        assert_eq!(file.apply_modality_lut(0).expect("lut"), vec![-1.0, 2.0]);
    }

    #[test]
    fn the_photometric_interpretation_is_not_needed_for_a_transform() {
        let mut set = base_set();
        set.set(Tag::new(0x0028, 0x0002), Vr::Us, Value::from_u16s(&[1]));
        set.set(
            Tag::new(0x0028, 0x0004),
            Vr::Cs,
            Value::from_str("MONOCHROME2"),
        );
        set.set(Tag::new(0x0028, 0x0010), Vr::Us, Value::from_u16s(&[1]));
        set.set(Tag::new(0x0028, 0x0011), Vr::Us, Value::from_u16s(&[1]));
        set.set(Tag::new(0x0028, 0x0100), Vr::Us, Value::from_u16s(&[8]));
        set.set(Tag::new(0x0028, 0x0101), Vr::Us, Value::from_u16s(&[8]));
        set.set(Tag::new(0x0028, 0x0102), Vr::Us, Value::from_u16s(&[7]));
        set.set(Tag::new(0x0028, 0x0103), Vr::Us, Value::from_u16s(&[0]));
        set.set(tags::PIXEL_DATA, Vr::Ob, Value::from_bytes([200u8]));
        let file = DicomFile::new(set, TransferSyntax::ExplicitVrLittleEndian).expect("file");
        assert_eq!(file.apply_modality_lut(0).expect("lut"), vec![200.0]);
        assert_eq!(
            file.pixel_descriptor().expect("descriptor").photometric,
            PhotometricInterpretation::Monochrome2
        );
    }
}
