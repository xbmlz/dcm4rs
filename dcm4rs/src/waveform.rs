//! Waveform decoding: the multiplex groups of `(5400,0100)` (PS3.3 C.10.9.1).
//!
//! A Waveform Sequence holds one item per multiplex group. Each item lists its channels in the
//! Channel Definition Sequence and stores its samples in Waveform Data, interleaved so that one
//! sample of every channel comes before the next sample. Decoding follows pydicom's
//! `decode_waveform`: the stored sample is scaled by Channel Sensitivity and its correction
//! factor, and Channel Baseline is added.

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::tag::Tag;

/// One channel of a decoded multiplex group.
#[derive(Clone, Debug, PartialEq)]
pub struct WaveformChannel {
    /// The samples of this channel, with sensitivity, correction factor and baseline applied.
    pub samples: Vec<f64>,
    /// `(003A,0210)` Channel Sensitivity; 1 when the element is absent.
    pub sensitivity: f64,
    /// `(003A,0213)` Channel Baseline; 0 when the element is absent.
    pub baseline: f64,
}

/// One decoded multiplex group.
#[derive(Clone, Debug, PartialEq)]
pub struct Waveform {
    /// Sample times in seconds: sample `i` is at `i / Sampling Frequency`.
    pub time: Vec<f64>,
    /// The channels, in the order the Channel Definition Sequence lists them.
    pub channels: Vec<WaveformChannel>,
}

impl Waveform {
    /// Decode the first multiplex group of `(5400,0100)`.
    pub fn decode(dataset: &DataSet) -> Result<Self> {
        let groups = dataset.element(tags::WAVEFORM_SEQUENCE)?.as_sequence()?;
        let group = groups.first().ok_or(Error::InvalidData {
            offset: 0,
            reason: "the waveform sequence holds no multiplex group",
        })?;
        Self::decode_group(group)
    }

    /// Decode one item of the Waveform Sequence.
    pub fn decode_group(group: &DataSet) -> Result<Self> {
        let channel_count = usize::from(group.u16(tags::NUMBER_OF_WAVEFORM_CHANNELS)?);
        let sample_count = usize::from(group.u16(tags::NUMBER_OF_WAVEFORM_SAMPLES)?);
        if channel_count == 0 || sample_count == 0 {
            return Err(Error::InvalidData {
                offset: 0,
                reason: "a multiplex group needs samples and channels",
            });
        }
        let frequency = decimal(group, tags::SAMPLING_FREQUENCY)?.ok_or(Error::InvalidData {
            offset: 0,
            reason: "a multiplex group needs a sampling frequency",
        })?;
        if frequency <= 0.0 {
            return Err(Error::InvalidData {
                offset: 0,
                reason: "the sampling frequency must be positive",
            });
        }

        let bits = group.u16(tags::WAVEFORM_BITS_ALLOCATED)?;
        let interpretation = group.string(tags::WAVEFORM_SAMPLE_INTERPRETATION)?;
        let raw = group.element(tags::WAVEFORM_DATA)?.raw_value();
        let endian = group.endian();
        let values = samples(
            raw,
            bits,
            interpretation.trim(),
            endian,
            channel_count * sample_count,
        )?;

        let definitions = match group.get(tags::CHANNEL_DEFINITION_SEQUENCE) {
            Some(element) => element.as_sequence()?,
            None => &[],
        };
        if definitions.len() != channel_count {
            return Err(Error::InvalidData {
                offset: 0,
                reason: "the channel definition sequence does not match the channel count",
            });
        }

        // The samples of one channel are every `channel_count`th value: one sample of each
        // channel comes before the next sample of any channel.
        let channels = definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| {
                let sensitivity = decimal(definition, tags::CHANNEL_SENSITIVITY)?.unwrap_or(1.0);
                let correction = decimal(definition, tags::CHANNEL_SENSITIVITY_CORRECTION_FACTOR)?
                    .unwrap_or(1.0);
                let baseline = decimal(definition, tags::CHANNEL_BASELINE)?.unwrap_or(0.0);
                let samples = values
                    .iter()
                    .skip(index)
                    .step_by(channel_count)
                    .map(|value| value * sensitivity * correction + baseline)
                    .collect();
                Ok(WaveformChannel {
                    samples,
                    sensitivity,
                    baseline,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let time = (0..sample_count)
            .map(|index| index as f64 / frequency)
            .collect();
        Ok(Self { time, channels })
    }
}

/// A `DS` attribute as one number, or `None` when it is absent.
fn decimal(set: &DataSet, tag: Tag) -> Result<Option<f64>> {
    match set.get(tag) {
        Some(element) => Ok(element.decimal_values()?.first().copied()),
        None => Ok(None),
    }
}

/// The stored samples, widened to `f64` at their stored width.
fn samples(
    raw: &[u8],
    bits: u16,
    interpretation: &str,
    endian: Endian,
    count: usize,
) -> Result<Vec<f64>> {
    let width = match bits {
        8 | 16 | 32 | 64 => usize::from(bits) / 8,
        _ => {
            return Err(Error::InvalidData {
                offset: 0,
                reason: "Waveform Bits Allocated is not 8, 16, 32 or 64",
            })
        }
    };
    if raw.len() < count * width {
        return Err(Error::InvalidData {
            offset: 0,
            reason: "Waveform Data is shorter than the sample count",
        });
    }
    let chunk = |index: usize| &raw[index * width..(index + 1) * width];
    let value: Vec<i64> = match (bits, interpretation) {
        (8, "SS") => (0..count).map(|i| i64::from(chunk(i)[0] as i8)).collect(),
        (8, "US") => (0..count).map(|i| i64::from(chunk(i)[0])).collect(),
        (16, "SS") => (0..count)
            .map(|i| i64::from(i16_at(chunk(i), endian)))
            .collect(),
        (16, "US") => (0..count)
            .map(|i| i64::from(u16_at(chunk(i), endian)))
            .collect(),
        (32, "SL") | (32, "SS") => (0..count)
            .map(|i| i64::from(i32_at(chunk(i), endian)))
            .collect(),
        (32, "UL") | (32, "US") => (0..count)
            .map(|i| i64::from(u32_at(chunk(i), endian)))
            .collect(),
        (32, "FL") => {
            return Ok((0..count)
                .map(|i| f64::from(f32_at(chunk(i), endian)))
                .collect())
        }
        (64, "FD") => return Ok((0..count).map(|i| f64_at(chunk(i), endian)).collect()),
        _ => {
            return Err(Error::InvalidData {
                offset: 0,
                reason: "Waveform Sample Interpretation does not match the bit width",
            })
        }
    };
    Ok(value.into_iter().map(|value| value as f64).collect())
}

fn u16_at(bytes: &[u8], endian: Endian) -> u16 {
    match endian {
        Endian::Little => u16::from_le_bytes([bytes[0], bytes[1]]),
        Endian::Big => u16::from_be_bytes([bytes[0], bytes[1]]),
    }
}

fn i16_at(bytes: &[u8], endian: Endian) -> i16 {
    match endian {
        Endian::Little => i16::from_le_bytes([bytes[0], bytes[1]]),
        Endian::Big => i16::from_be_bytes([bytes[0], bytes[1]]),
    }
}

fn u32_at(bytes: &[u8], endian: Endian) -> u32 {
    match endian {
        Endian::Little => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        Endian::Big => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    }
}

fn i32_at(bytes: &[u8], endian: Endian) -> i32 {
    match endian {
        Endian::Little => i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        Endian::Big => i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    }
}

fn f32_at(bytes: &[u8], endian: Endian) -> f32 {
    match endian {
        Endian::Little => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        Endian::Big => f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    }
}

fn f64_at(bytes: &[u8], endian: Endian) -> f64 {
    match endian {
        Endian::Little => f64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]),
        Endian::Big => f64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Value;
    use crate::vr::Vr;

    /// A multiplex group with `channels` channels of `samples` 16-bit signed samples.
    fn group(interleaved: &[i16], channels: usize, sensitivity: f64, baseline: f64) -> DataSet {
        let mut group = DataSet::new();
        group.set(
            tags::NUMBER_OF_WAVEFORM_CHANNELS,
            Vr::Us,
            Value::from_u16s(&[channels as u16]),
        );
        group.set(
            tags::NUMBER_OF_WAVEFORM_SAMPLES,
            Vr::Us,
            Value::from_u16s(&[(interleaved.len() / channels) as u16]),
        );
        group.set_string(tags::SAMPLING_FREQUENCY, Vr::Ds, "1000");
        group.set(
            tags::WAVEFORM_BITS_ALLOCATED,
            Vr::Us,
            Value::from_u16s(&[16]),
        );
        group.set_string(tags::WAVEFORM_SAMPLE_INTERPRETATION, Vr::Cs, "SS");
        group.set(tags::WAVEFORM_DATA, Vr::Ow, Value::from_i16s(interleaved));

        let definitions = (0..channels)
            .map(|index| {
                let mut definition = DataSet::new();
                definition.set(
                    tags::WAVEFORM_CHANNEL_NUMBER,
                    Vr::Us,
                    Value::from_u16s(&[(index + 1) as u16]),
                );
                definition.set_string(tags::CHANNEL_SENSITIVITY, Vr::Ds, &sensitivity.to_string());
                definition.set_string(tags::CHANNEL_BASELINE, Vr::Ds, &baseline.to_string());
                definition
            })
            .collect();
        group.set(
            tags::CHANNEL_DEFINITION_SEQUENCE,
            Vr::Sq,
            Value::sequence(definitions),
        );
        group
    }

    #[test]
    fn samples_are_demultiplexed_by_channel() {
        let group = group(&[1, 2, 3, 4, 5, 6], 2, 1.0, 0.0);
        let waveform = Waveform::decode_group(&group).expect("waveform");

        assert_eq!(waveform.channels[0].samples, vec![1.0, 3.0, 5.0]);
        assert_eq!(waveform.channels[1].samples, vec![2.0, 4.0, 6.0]);
    }

    #[test]
    fn sensitivity_and_baseline_scale_the_samples() {
        let group = group(&[10, 10], 1, 2.0, -4.0);
        let waveform = Waveform::decode_group(&group).expect("waveform");
        assert_eq!(waveform.channels[0].samples, vec![16.0, 16.0]);
    }

    #[test]
    fn the_time_vector_counts_from_zero() {
        let group = group(&[0, 0, 0], 1, 1.0, 0.0);
        let waveform = Waveform::decode_group(&group).expect("waveform");
        assert_eq!(waveform.time, vec![0.0, 0.001, 0.002]);
    }

    #[test]
    fn a_channel_count_without_definitions_is_refused() {
        let mut group = group(&[1, 2], 2, 1.0, 0.0);
        group.set(
            tags::CHANNEL_DEFINITION_SEQUENCE,
            Vr::Sq,
            Value::sequence(vec![DataSet::new()]),
        );
        assert!(Waveform::decode_group(&group).is_err());
    }

    #[test]
    fn the_first_multiplex_group_is_decoded() {
        let mut set = DataSet::new();
        set.set(
            tags::WAVEFORM_SEQUENCE,
            Vr::Sq,
            Value::sequence(vec![group(&[7], 1, 1.0, 0.0)]),
        );
        let waveform = Waveform::decode(&set).expect("waveform");
        assert_eq!(waveform.channels[0].samples, vec![7.0]);
    }

    #[test]
    fn an_unmatched_interpretation_is_refused() {
        let mut group = group(&[1], 1, 1.0, 0.0);
        group.set_string(tags::WAVEFORM_SAMPLE_INTERPRETATION, Vr::Cs, "FL");
        assert!(Waveform::decode_group(&group).is_err());
    }
}
