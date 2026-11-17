//! JPEG Lossless (Process 14) decoding, ISO/IEC 10918-1 Annex H.
//!
//! DICOM uses this codec for the `.57` (Process 14) and `.70` (Process 14, Selection Value 1)
//! transfer syntaxes. Every sample is coded as the difference to a predictor built from its
//! neighbours; `Ss` selects the predictor, restart intervals recut it and the point transform in
//! `Al` right-shifted the source samples before differencing.
//!
//! One scan covering every component is what DICOM streams carry, and that is what is decoded
//! here: the components of a colour image are interleaved sample by sample, with sampling factors
//! 1x1.

use super::PixelDescriptor;
use crate::error::{Error, Result};

/// A Huffman table for the difference categories (`Tc=0`).
struct Huffman {
    /// Smallest code of each length, indexed by length (1..=16).
    min_code: [i32; 17],
    /// Largest code of each length, `-1` when the length carries no codes.
    max_code: [i32; 17],
    /// Index of the first symbol of each length in `values`.
    value_index: [usize; 17],
    /// Symbols in code order.
    values: Vec<u8>,
}

impl Huffman {
    fn from_segment(table_id: u8, counts: &[u8], values: &[u8]) -> Result<Self> {
        if table_id >> 4 != 0 {
            return Err(Error::InvalidPixelData(
                "JPEG lossless: AC Huffman table in a lossless scan".to_owned(),
            ));
        }
        let symbols: usize = counts.iter().map(|&count| usize::from(count)).sum();
        if symbols != values.len() {
            return Err(Error::InvalidPixelData(format!(
                "JPEG lossless: Huffman table declares {symbols} symbols, carries {}",
                values.len()
            )));
        }

        let mut min_code = [0i32; 17];
        let mut max_code = [-1i32; 17];
        let mut value_index = [0usize; 17];
        let mut code = 0i32;
        let mut index = 0usize;
        for length in 1..=16 {
            let count = i32::from(counts[length - 1]);
            if count > 0 {
                value_index[length] = index;
                min_code[length] = code;
                code += count;
                index += count as usize;
                max_code[length] = code - 1;
                if code > 1 << length {
                    return Err(Error::InvalidPixelData(
                        "JPEG lossless: over-subscribed Huffman table".to_owned(),
                    ));
                }
            }
            // Every length reserves one more code space for the next length.
            code <<= 1;
        }

        Ok(Self {
            min_code,
            max_code,
            value_index,
            values: values.to_vec(),
        })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> std::result::Result<u8, Failure> {
        let mut code = 0i32;
        for length in 1..=16 {
            code = (code << 1) | i32::from(bits.bit()?);
            let max = self.max_code[length];
            if max >= 0 && code <= max {
                let index = self.value_index[length] + (code - self.min_code[length]) as usize;
                return self
                    .values
                    .get(index)
                    .copied()
                    .ok_or(Failure::Invalid("Huffman symbol out of range"));
            }
        }
        Err(Failure::Invalid("no Huffman code matched"))
    }
}

/// Why a byte or symbol could not be read.
///
/// The corpus carries a stream that ends before the image is complete, which decoders tolerate;
/// anything else is a malformed stream and stays an error.
#[derive(Debug)]
enum Failure {
    /// The entropy-coded data ended or a marker appeared where a sample was due.
    Ended,
    /// The stream is malformed; the text completes the error message.
    Invalid(&'static str),
}

/// Bit reader over the entropy-coded segment, undoing `0xFF00` stuffing.
struct Bits<'a> {
    data: &'a [u8],
    position: usize,
    /// The byte being consumed, most significant bit first.
    current: u32,
    /// Bits still unread in `current`.
    remaining: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            current: 0,
            remaining: 0,
        }
    }

    fn bit(&mut self) -> std::result::Result<u8, Failure> {
        if self.remaining == 0 {
            self.current = u32::from(self.byte()?);
            self.remaining = 8;
        }
        self.remaining -= 1;
        Ok(((self.current >> self.remaining) & 1) as u8)
    }

    /// Read `count` bits as an unsigned integer, most significant bit first.
    fn bits(&mut self, count: u8) -> std::result::Result<u32, Failure> {
        let mut value = 0u32;
        for _ in 0..count {
            value = (value << 1) | u32::from(self.bit()?);
        }
        Ok(value)
    }

    /// Next entropy-coded byte, skipping the stuffed `0x00` after `0xFF`.
    fn byte(&mut self) -> std::result::Result<u8, Failure> {
        while self.position < self.data.len() {
            let value = self.data[self.position];
            self.position += 1;
            if value != 0xFF {
                return Ok(value);
            }
            match self.data.get(self.position) {
                Some(0x00) => {
                    self.position += 1;
                    return Ok(0xFF);
                }
                // Fill bytes carry no data.
                Some(0xFF) => self.position += 1,
                // A marker where entropy-coded data was due ends the stream.
                Some(_) => return Err(Failure::Ended),
                None => break,
            }
        }
        Err(Failure::Ended)
    }

    /// Drop the partial byte and return the next marker, reading past fill and stuffed bytes.
    fn marker(&mut self) -> std::result::Result<u8, Failure> {
        self.remaining = 0;
        while self.position + 1 < self.data.len() {
            if self.data[self.position] != 0xFF {
                self.position += 1;
                continue;
            }
            match self.data[self.position + 1] {
                0x00 => self.position += 2,
                0xFF => self.position += 1,
                marker => {
                    self.position += 2;
                    return Ok(marker);
                }
            }
        }
        Err(Failure::Ended)
    }
}

/// One frame component as described by the start-of-frame segment.
#[derive(Clone, Copy)]
struct Component {
    /// Horizontal sampling factor (`H`).
    horizontal: u8,
    /// Vertical sampling factor (`V`).
    vertical: u8,
    /// Huffman table selector from the scan (`Td`).
    table: usize,
}

struct Frame {
    precision: u8,
    width: usize,
    height: usize,
    components: Vec<Component>,
    restart_interval: usize,
    /// Predictor selection value from `Ss` (1..=7).
    predictor: u8,
    /// Point transform from `Al`.
    point_transform: u8,
    huffman: [Option<Huffman>; 4],
    /// Offset of the entropy-coded segment inside the frame.
    scan_start: usize,
}

/// Decode one encapsulated JPEG lossless frame into little-endian samples.
pub(super) fn decode(frame: &[u8], descriptor: &PixelDescriptor) -> Result<Vec<u8>> {
    let layout = parse(frame)?;
    let samples = decode_scan(&layout, frame)?;

    if layout.components.len() != usize::from(descriptor.samples_per_pixel) {
        return Err(Error::InvalidPixelData(format!(
            "JPEG lossless codes {} components, descriptor declares {}",
            layout.components.len(),
            descriptor.samples_per_pixel
        )));
    }

    let expected = descriptor.frame_size();
    let bytes_per_sample = usize::from(descriptor.bits_allocated) / 8;
    let mut output = Vec::with_capacity(expected);
    for sample in samples {
        match bytes_per_sample {
            1 => {
                let byte = u8::try_from(sample).map_err(|_| {
                    Error::InvalidPixelData(format!(
                        "JPEG lossless sample {sample} does not fit eight bits"
                    ))
                })?;
                output.push(byte);
            }
            2 => output.extend_from_slice(&sample.to_le_bytes()),
            _ => {
                return Err(Error::UnsupportedPixelData(format!(
                    "JPEG lossless decoding supports 8- and 16-bit samples, got {} bits",
                    descriptor.bits_allocated
                )));
            }
        }
    }
    Ok(output)
}

fn parse(frame: &[u8]) -> Result<Frame> {
    if frame.get(..2) != Some(&[0xFFu8, 0xD8][..]) {
        return Err(Error::InvalidPixelData(
            "JPEG lossless: stream does not start with SOI".to_owned(),
        ));
    }

    let mut precision = None;
    let mut width = 0usize;
    let mut height = 0usize;
    let mut components: Vec<Component> = Vec::new();
    let mut restart_interval = 0usize;
    let mut huffman: [Option<Huffman>; 4] = [None, None, None, None];
    let mut scan: Option<(u8, u8, usize)> = None;

    let mut position = 2usize;
    while position + 3 < frame.len() {
        if frame[position] != 0xFF {
            position += 1;
            continue;
        }
        let marker = frame[position + 1];
        match marker {
            0x00 | 0xFF | 0xD0..=0xD9 => {
                position += if marker == 0xFF { 1 } else { 2 };
                continue;
            }
            _ => {}
        }
        let length = usize::from(frame[position + 2]) * 256 + usize::from(frame[position + 3]);
        if length < 2 {
            return Err(Error::InvalidPixelData(
                "JPEG lossless: malformed segment length".to_owned(),
            ));
        }
        let body = frame
            .get(position + 4..position + 2 + length)
            .ok_or_else(|| {
                Error::InvalidPixelData("JPEG lossless: segment extends past the stream".to_owned())
            })?;

        match marker {
            0xC3 => {
                let header = body.get(..6).ok_or_else(|| {
                    Error::InvalidPixelData("JPEG lossless: short SOF3 segment".to_owned())
                })?;
                precision = Some(header[0]);
                height = usize::from(header[1]) * 256 + usize::from(header[2]);
                width = usize::from(header[3]) * 256 + usize::from(header[4]);
                let count = usize::from(header[5]);
                components.clear();
                for index in 0..count {
                    let at = 6 + index * 3;
                    let entry = body.get(at..at + 3).ok_or_else(|| {
                        Error::InvalidPixelData(
                            "JPEG lossless: truncated component list".to_owned(),
                        )
                    })?;
                    if entry[2] != 0 {
                        return Err(Error::InvalidPixelData(
                            "JPEG lossless: quantized component".to_owned(),
                        ));
                    }
                    components.push(Component {
                        horizontal: entry[1] >> 4,
                        vertical: entry[1] & 0x0F,
                        table: 0,
                    });
                }
            }
            // SOF0..SOF2 and SOF5..SOF15 are lossy or hierarchical.
            0xC0..=0xC2 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err(Error::InvalidPixelData(
                    "JPEG lossless: start of frame is not Process 14".to_owned(),
                ));
            }
            0xC4 => {
                let mut at = 0usize;
                while at < body.len() {
                    let table_id = *body.get(at).ok_or_else(|| {
                        Error::InvalidPixelData("JPEG lossless: truncated Huffman table".to_owned())
                    })?;
                    let counts = body.get(at + 1..at + 17).ok_or_else(|| {
                        Error::InvalidPixelData(
                            "JPEG lossless: truncated Huffman table counts".to_owned(),
                        )
                    })?;
                    let symbols: usize = counts.iter().map(|&count| usize::from(count)).sum();
                    let values = body.get(at + 17..at + 17 + symbols).ok_or_else(|| {
                        Error::InvalidPixelData(
                            "JPEG lossless: truncated Huffman symbols".to_owned(),
                        )
                    })?;
                    let class = table_id >> 4;
                    let table = usize::from(table_id & 0x0F);
                    if class == 0 && table < 4 {
                        huffman[table] = Some(Huffman::from_segment(table_id, counts, values)?);
                    }
                    at += 17 + symbols;
                }
            }
            0xDD => {
                let interval = body.get(..2).ok_or_else(|| {
                    Error::InvalidPixelData("JPEG lossless: short DRI segment".to_owned())
                })?;
                restart_interval = usize::from(interval[0]) * 256 + usize::from(interval[1]);
            }
            0xDA => {
                // `Ns`, one `Cs`/`Td` pair per component, then `Ss`, `Se` and `Ah`/`Al`.
                let component_count = usize::from(*body.first().ok_or_else(|| {
                    Error::InvalidPixelData("JPEG lossless: empty SOS segment".to_owned())
                })?);
                let tail = body.get(1 + component_count * 2..).ok_or_else(|| {
                    Error::InvalidPixelData("JPEG lossless: short SOS segment".to_owned())
                })?;
                let header = tail.get(..3).ok_or_else(|| {
                    Error::InvalidPixelData("JPEG lossless: short SOS segment".to_owned())
                })?;
                let predictor = header[0];
                let spectral_end = header[1];
                let transform = header[2];
                if !(1..=7).contains(&predictor) {
                    return Err(Error::InvalidPixelData(format!(
                        "JPEG lossless: invalid predictor selection {predictor}"
                    )));
                }
                if spectral_end != 0 || transform & 0x0F != 0 {
                    return Err(Error::InvalidPixelData(format!(
                        "JPEG lossless: spectral selection {spectral_end} and successive \
                         approximation {} in a lossless scan",
                        transform & 0x0F
                    )));
                }
                if component_count != components.len() {
                    return Err(Error::UnsupportedPixelData(format!(
                        "JPEG lossless: scan covers {component_count} of {} components",
                        components.len()
                    )));
                }
                for (index, component) in components.iter_mut().enumerate() {
                    let selector = *body.get(1 + index * 2 + 1).ok_or_else(|| {
                        Error::InvalidPixelData(
                            "JPEG lossless: truncated scan component".to_owned(),
                        )
                    })?;
                    let table = usize::from(selector & 0x0F);
                    if table >= 4 {
                        return Err(Error::InvalidPixelData(format!(
                            "JPEG lossless: Huffman table selector {table}"
                        )));
                    }
                    component.table = table;
                }
                scan = Some((predictor, transform >> 4, position + 2 + length));
                break;
            }
            _ => {}
        }

        position += 2 + length;
    }

    let precision = precision
        .ok_or_else(|| Error::InvalidPixelData("JPEG lossless: no SOF3 segment".to_owned()))?;
    let (predictor, point_transform, scan_start) =
        scan.ok_or_else(|| Error::InvalidPixelData("JPEG lossless: no SOS segment".to_owned()))?;
    if !(1..=16).contains(&precision) {
        return Err(Error::InvalidPixelData(format!(
            "JPEG lossless: sample precision {precision}"
        )));
    }
    if width == 0 || height == 0 {
        return Err(Error::InvalidPixelData(
            "JPEG lossless: empty image".to_owned(),
        ));
    }
    for component in &components {
        if component.horizontal != 1 || component.vertical != 1 {
            return Err(Error::UnsupportedPixelData(format!(
                "JPEG lossless: sampling factors {}x{}",
                component.horizontal, component.vertical
            )));
        }
    }

    Ok(Frame {
        precision,
        width,
        height,
        components,
        restart_interval,
        predictor,
        point_transform,
        huffman,
        scan_start,
    })
}

/// Decode the entropy-coded segment into samples, interleaved per pixel.
fn decode_scan(layout: &Frame, data: &[u8]) -> Result<Vec<u16>> {
    let precision = layout.precision;
    if layout.point_transform >= precision {
        return Err(Error::InvalidPixelData(format!(
            "JPEG lossless: point transform {} with precision {precision}",
            layout.point_transform
        )));
    }
    let modulus = 1i32 << precision;
    let initial = 1i32 << (precision - 1);

    let tables: Vec<&Huffman> = layout
        .components
        .iter()
        .map(|component| {
            layout.huffman[component.table].as_ref().ok_or_else(|| {
                Error::InvalidPixelData(format!(
                    "JPEG lossless: missing Huffman table {}",
                    component.table
                ))
            })
        })
        .collect::<Result<_>>()?;

    let mut bits = Bits::new(data.get(layout.scan_start..).ok_or_else(|| {
        Error::InvalidPixelData("JPEG lossless: scan starts past the stream".to_owned())
    })?);
    let width = layout.width;
    let count = layout.components.len();
    let mut planes: Vec<Vec<u16>> = vec![vec![0u16; width * layout.height]; count];
    let mut restarts_left = layout.restart_interval;
    let mut start_of_row = vec![true; count];

    // A stream that ends before the image is complete keeps the samples it already carried and
    // leaves the rest at zero, as libjpeg does after a "premature end of data" warning; the
    // corpus has such a file.
    'rows: for row in 0..layout.height {
        for column in 0..width {
            for (index, table) in tables.iter().enumerate() {
                if layout.restart_interval > 0 && restarts_left == 0 {
                    let marker = match bits.marker() {
                        Ok(marker) => marker,
                        Err(Failure::Ended) => break 'rows,
                        Err(Failure::Invalid(message)) => {
                            return Err(Error::InvalidPixelData(format!(
                                "JPEG lossless: {message}"
                            )));
                        }
                    };
                    if !(0xD0..=0xD7).contains(&marker) {
                        return Err(Error::InvalidPixelData(format!(
                            "JPEG lossless: marker {marker:02X} where a restart was due"
                        )));
                    }
                    start_of_row.iter_mut().for_each(|flag| *flag = true);
                    restarts_left = layout.restart_interval;
                }

                let category = match table.decode(&mut bits) {
                    Ok(category) => category,
                    Err(Failure::Ended) => break 'rows,
                    Err(Failure::Invalid(message)) => {
                        return Err(Error::InvalidPixelData(format!("JPEG lossless: {message}")));
                    }
                };
                let difference = if category == 0 {
                    0
                } else if category > 16 {
                    return Err(Error::InvalidPixelData(format!(
                        "JPEG lossless: difference category {category}"
                    )));
                } else {
                    let bits_read = match bits.bits(category) {
                        Ok(value) => value,
                        Err(Failure::Ended) => break 'rows,
                        Err(Failure::Invalid(message)) => {
                            return Err(Error::InvalidPixelData(format!(
                                "JPEG lossless: {message}"
                            )));
                        }
                    };
                    extend(bits_read, category)
                };

                let plane = &mut planes[index];
                let left = if column > 0 && !start_of_row[index] {
                    Some(i32::from(plane[row * width + column - 1]))
                } else {
                    None
                };
                let above = if row > 0 {
                    Some(i32::from(plane[(row - 1) * width + column]))
                } else {
                    None
                };
                let above_left = if row > 0 && column > 0 {
                    Some(i32::from(plane[(row - 1) * width + column - 1]))
                } else {
                    None
                };
                let predicted = predict(layout.predictor, left, above, above_left, initial);
                let sample = (predicted + difference).rem_euclid(modulus);
                plane[row * width + column] = sample as u16;
                start_of_row[index] = false;

                if layout.restart_interval > 0 {
                    restarts_left -= 1;
                }
            }
        }
    }

    let shift = layout.point_transform;
    let mut samples = Vec::with_capacity(width * layout.height * count);
    for row in 0..layout.height {
        for column in 0..width {
            for plane in &planes {
                let sample = u32::from(plane[row * width + column]) << shift;
                samples.push(sample.min(u32::from(u16::MAX)) as u16);
            }
        }
    }
    Ok(samples)
}

/// Signed value of a `category`-bit magnitude: JPEG's "receive and extend".
fn extend(value: u32, category: u8) -> i32 {
    let value = value as i32;
    if value < 1 << (category - 1) {
        value - ((1 << category) - 1)
    } else {
        value
    }
}

/// Predictor from the neighbours of one component (ISO/IEC 10918-1 H.1).
///
/// With no neighbour above and none to the left this is the first sample of the image, so the
/// prediction is half the precision. A sample that starts a row, or a restart interval, is
/// predicted from the sample above alone.
fn predict(
    selection: u8,
    left: Option<i32>,
    above: Option<i32>,
    above_left: Option<i32>,
    initial: i32,
) -> i32 {
    match (left, above) {
        (None, None) => initial,
        (None, Some(above)) => above,
        (Some(left), None) => left,
        (Some(left), Some(above)) => {
            let above_left = above_left.unwrap_or(0);
            match selection {
                1 => left,
                2 => above,
                3 => above_left,
                4 => left + above - above_left,
                5 => left + (above - above_left) / 2,
                6 => above + (left - above_left) / 2,
                _ => (left + above) / 2,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extend_reads_signed_magnitudes() {
        assert_eq!(extend(0, 1), -1);
        assert_eq!(extend(1, 1), 1);
        assert_eq!(extend(0, 16), -65535);
        assert_eq!(extend(32768, 16), 32768);
    }

    #[test]
    fn predictors_follow_the_selection_value() {
        let predicted = |selection| predict(selection, Some(10), Some(20), Some(30), 128);
        assert_eq!(predicted(1), 10);
        assert_eq!(predicted(2), 20);
        assert_eq!(predicted(3), 30);
        assert_eq!(predicted(4), 0);
        assert_eq!(predicted(5), 5);
        assert_eq!(predicted(6), 10);
        assert_eq!(predicted(7), 15);
    }

    #[test]
    fn prediction_at_the_edges_uses_the_available_neighbour() {
        assert_eq!(predict(1, None, None, None, 128), 128);
        assert_eq!(predict(1, None, Some(20), None, 128), 20);
        assert_eq!(predict(1, Some(10), None, None, 128), 10);
        assert_eq!(predict(3, Some(10), Some(20), None, 128), 0);
    }

    #[test]
    fn huffman_tables_reject_an_ac_class() {
        assert!(matches!(
            Huffman::from_segment(0x10, &[0u8; 16], &[]),
            Err(Error::InvalidPixelData(_))
        ));
    }

    #[test]
    fn huffman_tables_reject_a_symbol_count_mismatch() {
        let mut counts = [0u8; 16];
        counts[0] = 1;
        assert!(Huffman::from_segment(0x00, &counts, &[]).is_err());
    }

    #[test]
    fn huffman_tables_reject_an_over_subscribed_table() {
        let mut counts = [0u8; 16];
        counts[0] = 3;
        assert!(Huffman::from_segment(0x00, &counts, &[0, 1, 2]).is_err());
    }

    #[test]
    fn huffman_decodes_a_symbol_from_its_code() {
        let mut counts = [0u8; 16];
        counts[0] = 1;
        counts[1] = 1;
        let table = Huffman::from_segment(0x00, &counts, &[0, 1]).unwrap();
        let mut bits = Bits::new(&[0b0100_0000]);
        assert_eq!(table.decode(&mut bits).unwrap(), 0);
        assert_eq!(table.decode(&mut bits).unwrap(), 1);
    }

    #[test]
    fn huffman_reports_a_code_it_cannot_match() {
        let table = Huffman::from_segment(0x00, &[0u8; 16], &[]).unwrap();
        let mut bits = Bits::new(&[0x00, 0x00]);
        assert!(matches!(table.decode(&mut bits), Err(Failure::Invalid(_))));
    }
}
