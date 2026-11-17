//! Pixel descriptors, frame extraction and RLE Lossless decoding.

#[cfg(feature = "pixel-jpegls")]
use charls::{CharLS, FrameInfo, InterleaveMode};
use dcm4rs::dictionary::tags;
use dcm4rs::{DataSet, DicomFile, Error, PhotometricInterpretation, Value, Vr};
#[cfg(feature = "pixel-jpeg")]
use jpeg_encoder::{ColorType, Encoder};

const RLE_UID: &str = "1.2.840.10008.1.2.5";
const EXPLICIT_LE_UID: &str = "1.2.840.10008.1.2.1";
const JPEG_LOSSLESS_UID: &str = "1.2.840.10008.1.2.4.57";
const MPEG_UID: &str = "1.2.840.10008.1.2.4.100";

fn pixel_set(
    rows: u16,
    columns: u16,
    samples: u16,
    bits: u16,
    frames: u32,
    photometric: &str,
) -> DataSet {
    let mut set = DataSet::new();
    let _ = set.set(tags::ROWS, Vr::Us, Value::from_u16s(&[rows]));
    let _ = set.set(tags::COLUMNS, Vr::Us, Value::from_u16s(&[columns]));
    let _ = set.set(
        tags::SAMPLES_PER_PIXEL,
        Vr::Us,
        Value::from_u16s(&[samples]),
    );
    let _ = set.set(tags::BITS_ALLOCATED, Vr::Us, Value::from_u16s(&[bits]));
    let _ = set.set(tags::BITS_STORED, Vr::Us, Value::from_u16s(&[bits]));
    let _ = set.set(tags::HIGH_BIT, Vr::Us, Value::from_u16s(&[bits - 1]));
    let _ = set.set(tags::PIXEL_REPRESENTATION, Vr::Us, Value::from_u16s(&[0]));
    if samples > 1 {
        let _ = set.set(tags::PLANAR_CONFIGURATION, Vr::Us, Value::from_u16s(&[0]));
    }
    let _ = set.set(
        tags::PHOTOMETRIC_INTERPRETATION,
        Vr::Cs,
        Value::from_str(photometric),
    );
    if frames != 1 {
        let _ = set.set(
            tags::NUMBER_OF_FRAMES,
            Vr::Is,
            Value::from_str(&frames.to_string()),
        );
    }
    set
}

fn file(mut dataset: DataSet, uid: &str) -> DicomFile {
    let mut meta = DataSet::new();
    let _ = meta.set(tags::TRANSFER_SYNTAX_UID, Vr::Ui, Value::from_str(uid));
    dataset.set_endian(dcm4rs::Endian::Little);
    DicomFile { meta, dataset }
}

/// Encode a literal PackBits run of at most 128 bytes.
fn literal(values: &[u8]) -> Vec<u8> {
    assert!(!values.is_empty() && values.len() <= 128);
    let mut segment = Vec::with_capacity(values.len() + 1);
    segment.push((values.len() - 1) as u8);
    segment.extend_from_slice(values);
    segment
}

/// Assemble an RLE frame from already encoded segments.
fn rle_frame(segments: &[Vec<u8>]) -> Vec<u8> {
    let mut frame = vec![0u8; 64];
    frame[..4].copy_from_slice(&(segments.len() as u32).to_le_bytes());
    let mut offset = 64;
    for (index, segment) in segments.iter().enumerate() {
        frame[4 + index * 4..8 + index * 4].copy_from_slice(&(offset as u32).to_le_bytes());
        offset += segment.len();
    }
    for segment in segments {
        frame.extend_from_slice(segment);
    }
    frame
}

fn rle_file(rows: u16, columns: u16, samples: u16, bits: u16, segments: &[Vec<u8>]) -> DicomFile {
    let mut set = pixel_set(rows, columns, samples, bits, 1, "MONOCHROME2");
    let frame = rle_frame(segments);
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), frame]),
    );
    file(set, RLE_UID)
}

#[test]
fn descriptor_reads_pixel_attributes() {
    let set = pixel_set(2, 3, 1, 16, 4, "MONOCHROME2");
    let descriptor = set.pixel_descriptor().unwrap();
    assert_eq!(descriptor.rows, 2);
    assert_eq!(descriptor.columns, 3);
    assert_eq!(descriptor.samples_per_pixel, 1);
    assert_eq!(descriptor.bits_allocated, 16);
    assert_eq!(descriptor.bits_stored, 16);
    assert_eq!(descriptor.high_bit, 15);
    assert_eq!(descriptor.number_of_frames, 4);
    assert_eq!(
        descriptor.photometric,
        PhotometricInterpretation::Monochrome2
    );
    assert_eq!(descriptor.bytes_per_sample(), 2);
    assert_eq!(descriptor.frame_size(), 2 * 3 * 2);
}

#[test]
fn rle_8bit_mono_decodes() {
    let file = rle_file(2, 2, 1, 8, &[literal(&[1, 2, 3, 4])]);
    assert_eq!(file.decode_frame(0).unwrap(), vec![1, 2, 3, 4]);
}

#[test]
fn rle_16bit_mono_decodes_little_endian() {
    let file = rle_file(
        2,
        2,
        1,
        16,
        &[
            literal(&[0x12, 0x34, 0x12, 0x34]),
            literal(&[0x56, 0x78, 0x56, 0x78]),
        ],
    );
    assert_eq!(
        file.decode_frame(0).unwrap(),
        vec![0x56, 0x12, 0x78, 0x34, 0x56, 0x12, 0x78, 0x34]
    );
}

#[test]
fn rle_rgb_decodes_interleaved() {
    let file = rle_file(
        1,
        1,
        3,
        8,
        &[literal(&[10]), literal(&[20]), literal(&[30])],
    );
    assert_eq!(file.decode_frame(0).unwrap(), vec![10, 20, 30]);
}

#[test]
fn rle_rejects_wrong_segment_count() {
    let file = rle_file(1, 1, 1, 8, &[literal(&[1]), literal(&[2])]);
    assert!(matches!(
        file.decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[test]
fn unsupported_codec_is_reported() {
    let mut set = pixel_set(1, 1, 1, 8, 1, "MONOCHROME2");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), vec![0xFF, 0xD8]]),
    );
    let file = file(set, MPEG_UID);
    assert!(matches!(
        file.decode_frame(0),
        Err(Error::UnsupportedPixelData(_))
    ));
}

#[test]
fn jpeg_lossless_decodes() {
    // 3x2, 8-bit: values 100 101 100 / 100 99 100 with predictor 1, so the differences are
    // -28 +1 -1 / 0 -1 +1 and the first sample is predicted from half the precision.
    let mut set = pixel_set(3, 2, 1, 8, 1, "MONOCHROME2");
    let stream = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xC3, 0x00, 0x0B, 0x08, 0x00, 0x02, 0x00, 0x03, 0x01, 0x01, 0x11, 0x00, // SOF3
        0xFF, 0xC4, 0x00, 0x16, 0x00, // DHT: categories 0, 1 and 5 as 0, 10 and 110
        0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x05, //
        0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x01, 0x00, 0x00, // SOS
        0xC3, 0xB1, 0x2F, // entropy-coded differences
        0xFF, 0xD9, // EOI
    ];
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), stream]),
    );
    let file = file(set, JPEG_LOSSLESS_UID);
    assert_eq!(
        file.decode_frame(0).unwrap(),
        vec![100, 101, 100, 100, 99, 100]
    );
}

/// Build a JPEG marker segment: `FF marker`, the two-byte length and the body.
fn jpeg_segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xFF, marker];
    bytes.extend_from_slice(&(body.len() as u16 + 2).to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}

/// SOF3 body for a `height` x `width` frame with components given as (identifier, sampling).
fn sof3_body(precision: u8, height: u16, width: u16, components: &[(u8, u8)]) -> Vec<u8> {
    let mut body = vec![precision];
    body.extend_from_slice(&height.to_be_bytes());
    body.extend_from_slice(&width.to_be_bytes());
    body.push(components.len() as u8);
    for &(identifier, sampling) in components {
        body.push(identifier);
        body.push(sampling);
        body.push(0);
    }
    body
}

/// Huffman table segment for the difference categories with `counts` codes per length.
fn dht_segment(class: u8, table: u8, counts: [u8; 16], symbols: &[u8]) -> Vec<u8> {
    let mut body = vec![(class << 4) | table];
    body.extend_from_slice(&counts);
    body.extend_from_slice(symbols);
    jpeg_segment(0xC4, &body)
}

/// Scan header for the components given as (identifier, table selector).
fn sos_segment(predictor: u8, transform: u8, selectors: &[(u8, u8)]) -> Vec<u8> {
    let mut body = vec![selectors.len() as u8];
    for &(identifier, table) in selectors {
        body.push(identifier);
        body.push(table);
    }
    body.push(predictor);
    body.push(0);
    body.push(transform);
    jpeg_segment(0xDA, &body)
}

/// Assemble a JPEG Lossless file from a codestream.
fn lossless_file(rows: u16, columns: u16, samples: u16, bits: u16, stream: Vec<u8>) -> DicomFile {
    let mut set = pixel_set(
        rows,
        columns,
        samples,
        bits,
        1,
        if samples == 1 { "MONOCHROME2" } else { "RGB" },
    );
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), stream]),
    );
    file(set, JPEG_LOSSLESS_UID)
}

#[test]
fn jpeg_lossless_restart_intervals_reset_the_prediction() {
    // 2x2, 8-bit: 128 129 / 128 129 with predictor 2 and a restart after every sample, so each
    // sample is predicted from the sample above alone.
    let mut counts = [0u8; 16];
    counts[0] = 1;
    counts[1] = 1;
    let mut stream = vec![0xFF, 0xD8];
    stream.extend(jpeg_segment(0xC3, &sof3_body(8, 2, 2, &[(1, 0x11)])));
    stream.extend(dht_segment(0, 0, counts, &[0, 1]));
    stream.extend(jpeg_segment(0xDD, &[0x00, 0x01]));
    stream.extend(sos_segment(2, 0, &[(1, 0)]));
    stream.extend([0x7F, 0xFF, 0xD0, 0xBF, 0xFF, 0xD1, 0x7F, 0xFF, 0xD2, 0x7F]);
    stream.extend([0xFF, 0xD9]);
    assert_eq!(
        lossless_file(2, 2, 1, 8, stream).decode_frame(0).unwrap(),
        vec![128, 129, 128, 129]
    );
}

#[test]
fn jpeg_lossless_rejects_a_point_transform() {
    // A non-zero successive approximation is a point transform this decoder does not apply.
    let mut counts = [0u8; 16];
    counts[0] = 1;
    counts[15] = 1;
    let mut stream = vec![0xFF, 0xD8];
    stream.extend(jpeg_segment(0xC3, &sof3_body(16, 1, 1, &[(1, 0x11)])));
    stream.extend(dht_segment(0, 0, counts, &[0, 15]));
    stream.extend(sos_segment(1, 0x01, &[(1, 0)]));
    stream.extend([0x80, 0x00, 0x00, 0x01]);
    stream.extend([0xFF, 0xD9]);
    assert!(matches!(
        lossless_file(1, 1, 1, 16, stream).decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[test]
fn jpeg_lossless_decodes_interleaved_components() {
    // 1x1 with three components, each coding a difference of zero from half the precision.
    let mut counts = [0u8; 16];
    counts[0] = 1;
    let mut stream = vec![0xFF, 0xD8];
    stream.extend(jpeg_segment(
        0xC3,
        &sof3_body(8, 1, 1, &[(1, 0x11), (2, 0x11), (3, 0x11)]),
    ));
    for table in 0..3 {
        stream.extend(dht_segment(0, table, counts, &[0]));
    }
    stream.extend(sos_segment(1, 0, &[(1, 0), (2, 1), (3, 2)]));
    stream.extend([0x1F]);
    stream.extend([0xFF, 0xD9]);
    assert_eq!(
        lossless_file(1, 1, 3, 8, stream).decode_frame(0).unwrap(),
        vec![128, 128, 128]
    );
}

#[test]
fn jpeg_lossless_keeps_a_stream_that_ends_early() {
    // 1x2 but only one sample is coded; the rest of the image stays zero.
    let mut counts = [0u8; 16];
    counts[0] = 1;
    counts[1] = 1;
    let mut stream = vec![0xFF, 0xD8];
    stream.extend(jpeg_segment(0xC3, &sof3_body(8, 1, 2, &[(1, 0x11)])));
    stream.extend(dht_segment(0, 0, counts, &[0, 1]));
    stream.extend(sos_segment(1, 0, &[(1, 0)]));
    stream.extend([0x7F]);
    stream.extend([0xFF, 0xD9]);
    assert_eq!(
        lossless_file(1, 2, 1, 8, stream).decode_frame(0).unwrap(),
        vec![128, 0]
    );
}

#[test]
fn jpeg_lossless_rejects_a_lossy_start_of_frame() {
    let mut stream = vec![0xFF, 0xD8];
    stream.extend(jpeg_segment(0xC0, &sof3_body(8, 1, 1, &[(1, 0x11)])));
    stream.extend([0xFF, 0xD9]);
    assert!(matches!(
        lossless_file(1, 1, 1, 8, stream).decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[test]
fn jpeg_lossless_rejects_an_unknown_predictor() {
    let mut counts = [0u8; 16];
    counts[0] = 1;
    let mut stream = vec![0xFF, 0xD8];
    stream.extend(jpeg_segment(0xC3, &sof3_body(8, 1, 1, &[(1, 0x11)])));
    stream.extend(dht_segment(0, 0, counts, &[0]));
    stream.extend(sos_segment(0, 0, &[(1, 0)]));
    stream.extend([0x1F]);
    stream.extend([0xFF, 0xD9]);
    assert!(matches!(
        lossless_file(1, 1, 1, 8, stream).decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[test]
fn jpeg_lossless_rejects_a_missing_huffman_table() {
    let mut counts = [0u8; 16];
    counts[0] = 1;
    let mut stream = vec![0xFF, 0xD8];
    stream.extend(jpeg_segment(0xC3, &sof3_body(8, 1, 1, &[(1, 0x11)])));
    stream.extend(dht_segment(0, 0, counts, &[0]));
    stream.extend(sos_segment(1, 0, &[(1, 1)]));
    stream.extend([0x1F]);
    stream.extend([0xFF, 0xD9]);
    assert!(matches!(
        lossless_file(1, 1, 1, 8, stream).decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[test]
fn jpeg_lossless_rejects_a_marker_where_a_restart_was_due() {
    let mut counts = [0u8; 16];
    counts[0] = 1;
    let mut stream = vec![0xFF, 0xD8];
    stream.extend(jpeg_segment(0xC3, &sof3_body(8, 1, 2, &[(1, 0x11)])));
    stream.extend(dht_segment(0, 0, counts, &[0]));
    stream.extend(jpeg_segment(0xDD, &[0x00, 0x01]));
    stream.extend(sos_segment(1, 0, &[(1, 0)]));
    stream.extend([0x1F, 0xFF, 0xD9]);
    assert!(matches!(
        lossless_file(1, 2, 1, 8, stream).decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[test]
fn jpeg_lossless_rejects_a_scan_that_does_not_cover_the_frame() {
    let mut counts = [0u8; 16];
    counts[0] = 1;
    let mut stream = vec![0xFF, 0xD8];
    stream.extend(jpeg_segment(0xC3, &sof3_body(8, 1, 1, &[(1, 0x11)])));
    stream.extend(dht_segment(0, 0, counts, &[0]));
    stream.extend(sos_segment(1, 0, &[(1, 0), (2, 0)]));
    stream.extend([0x1F]);
    stream.extend([0xFF, 0xD9]);
    assert!(matches!(
        lossless_file(1, 1, 2, 8, stream).decode_frame(0),
        Err(Error::UnsupportedPixelData(_))
    ));
}

#[test]
fn basic_offset_table_splits_frames() {
    let mut set = DataSet::new();
    let basic_offset_table = vec![0, 0, 0, 0, 4, 0, 0, 0];
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![basic_offset_table, vec![1, 2, 3, 4], vec![5, 6, 7, 8]]),
    );
    let frames = set.encapsulated_frames().unwrap();
    assert_eq!(frames, vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]]);
}

#[test]
fn native_frames_split_by_size() {
    let mut set = pixel_set(2, 2, 1, 8, 2, "MONOCHROME2");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::from_bytes(vec![1, 2, 3, 4, 5, 6, 7, 8]),
    );
    let file = file(set, EXPLICIT_LE_UID);
    let frames = file.pixel_frames().unwrap();
    assert_eq!(frames, vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]]);
    assert_eq!(file.decode_frame(1).unwrap(), vec![5, 6, 7, 8]);
}

#[cfg(feature = "pixel-jpeg")]
fn jpeg_frame(pixels: &[u8], width: u16, height: u16, color: ColorType) -> Vec<u8> {
    let mut out = Vec::new();
    Encoder::new(&mut out, 100)
        .encode(pixels, width, height, color)
        .unwrap();
    out
}

#[cfg(feature = "pixel-jpeg")]
#[test]
fn jpeg_grayscale_decodes() {
    let frame = jpeg_frame(&[128; 4], 2, 2, ColorType::Luma);
    let mut set = pixel_set(2, 2, 1, 8, 1, "MONOCHROME2");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), frame]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.50");
    let decoded = file.decode_frame(0).unwrap();
    assert_eq!(decoded.len(), 4);
    for value in decoded {
        assert!(value.abs_diff(128) <= 4, "decoded {value}, expected 128");
    }
}

#[cfg(feature = "pixel-jpeg")]
#[test]
fn jpeg_rgb_decodes() {
    let frame = jpeg_frame(
        &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
        2,
        2,
        ColorType::Rgb,
    );
    let mut set = pixel_set(2, 2, 3, 8, 1, "YBR_FULL_422");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), frame]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.50");
    assert_eq!(file.decode_frame(0).unwrap().len(), 12);
}

#[cfg(feature = "pixel-jpeg")]
#[test]
fn jpeg_extended_12bit_decodes() {
    // JPEG Extended (Process 2/4), the process DICOM uses for 12-bit samples in `.51`.
    let width = 16u16;
    let height = 16u16;
    let samples: Vec<u16> = (0..u32::from(width) * u32::from(height))
        .map(|index| ((index * 293) % 4096) as u16)
        .collect();
    let options = oxiarc_jpeg::EncodeOptions {
        precision: 12,
        ..Default::default()
    };
    let frame = oxiarc_jpeg::encode_u16_to_vec_with_options(
        &samples,
        width,
        height,
        oxiarc_jpeg::InputColor::Luma,
        &options,
    )
    .expect("encode a 12-bit JPEG");
    let mut set = pixel_set(height, width, 1, 16, 1, "MONOCHROME2");
    let _ = set.set(tags::BITS_STORED, Vr::Us, Value::from_u16s(&[12]));
    let _ = set.set(tags::HIGH_BIT, Vr::Us, Value::from_u16s(&[11]));
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), frame]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.51");
    let decoded = file.decode_frame(0).expect("decode a 12-bit JPEG");
    assert_eq!(decoded.len(), usize::from(width) * usize::from(height) * 2);
    let values: Vec<u16> = decoded
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    assert!(
        values.iter().all(|value| *value <= 4095),
        "12-bit samples exceed the stored precision"
    );
    let mean = values.iter().map(|value| i32::from(*value)).sum::<i32>() / values.len() as i32;
    let expected =
        samples.iter().map(|value| i32::from(*value)).sum::<i32>() / samples.len() as i32;
    assert!(
        (mean - expected).abs() <= 16,
        "decoded mean {mean} differs from {expected}"
    );
}

/// Rewrite the first `SOS` segment's spectral end as an encoder in the field would.
#[cfg(feature = "pixel-jpeg")]
fn zero_spectral_end(frame: &mut [u8]) {
    let sos = frame
        .windows(2)
        .position(|pair| pair == [0xFF, 0xDA])
        .expect("SOS marker");
    let components = usize::from(frame[sos + 4]);
    frame[sos + 5 + 2 * components + 1] = 0;
}

#[cfg(feature = "pixel-jpeg")]
#[test]
fn jpeg_sequential_scan_with_a_zero_spectral_end_decodes() {
    // T.81 wants `Se == 63`; files in the wild carry `Se == 0`, and libjpeg warns and decodes.
    // `pylibjpeg-data` publishes a fixed copy of such a file that differs in this byte alone,
    // so a repaired decode has to equal the untouched one exactly.
    let width = 8u16;
    let height = 8u16;
    let samples: Vec<u16> = (0..u32::from(width) * u32::from(height))
        .map(|index| ((index * 397) % 4096) as u16)
        .collect();
    let options = oxiarc_jpeg::EncodeOptions {
        precision: 12,
        ..Default::default()
    };
    let frame = oxiarc_jpeg::encode_u16_to_vec_with_options(
        &samples,
        width,
        height,
        oxiarc_jpeg::InputColor::Luma,
        &options,
    )
    .expect("encode a 12-bit JPEG");
    let mut broken = frame.clone();
    zero_spectral_end(&mut broken);
    let clean = jpeg_extended_file(width, height, &frame)
        .decode_frame(0)
        .expect("decode the encoder's own stream");
    let repaired = jpeg_extended_file(width, height, &broken)
        .decode_frame(0)
        .expect("decode the stream with a zero spectral end");
    assert_eq!(repaired, clean);
}

#[cfg(feature = "pixel-jpeg")]
fn jpeg_extended_file(rows: u16, columns: u16, frame: &[u8]) -> DicomFile {
    let mut set = pixel_set(rows, columns, 1, 16, 1, "MONOCHROME2");
    let _ = set.set(tags::BITS_STORED, Vr::Us, Value::from_u16s(&[12]));
    let _ = set.set(tags::HIGH_BIT, Vr::Us, Value::from_u16s(&[11]));
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), frame.to_vec()]),
    );
    file(set, "1.2.840.10008.1.2.4.51")
}

#[cfg(feature = "pixel-jpeg")]
#[test]
fn jpeg_rejects_malformed_stream() {
    let mut set = pixel_set(1, 1, 1, 8, 1, "MONOCHROME2");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), vec![0xFF, 0xD8, 0xFF]]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.50");
    assert!(matches!(
        file.decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[cfg(feature = "pixel-jpegls")]
fn jpegls_frame(pixels: &[u8], width: u32, height: u32, bits: i32, components: i32) -> Vec<u8> {
    let mut encoder = CharLS::default();
    if components > 1 {
        encoder.set_interleave_mode(InterleaveMode::Sample).unwrap();
    }
    encoder
        .encode(
            FrameInfo {
                width,
                height,
                bits_per_sample: bits,
                component_count: components,
            },
            0,
            pixels,
        )
        .unwrap()
}

#[cfg(feature = "pixel-jpegls")]
#[test]
fn jpegls_grayscale_decodes() {
    let values = [10u8, 20, 30, 40];
    let frame = jpegls_frame(&values, 2, 2, 8, 1);
    let mut set = pixel_set(2, 2, 1, 8, 1, "MONOCHROME2");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), frame]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.80");
    assert_eq!(file.decode_frame(0).unwrap(), values);
}

#[cfg(feature = "pixel-jpegls")]
#[test]
fn jpegls_rgb_decodes() {
    let values: [u8; 3] = [1, 2, 3];
    let frame = jpegls_frame(&values, 1, 1, 8, 3);
    let mut set = pixel_set(1, 1, 3, 8, 1, "RGB");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), frame]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.80");
    assert_eq!(file.decode_frame(0).unwrap(), values);
}

#[cfg(feature = "pixel-jpegls")]
#[test]
fn jpegls_rejects_dimension_mismatch() {
    let frame = jpegls_frame(&[1u8; 4], 2, 2, 8, 1);
    let mut set = pixel_set(1, 1, 1, 8, 1, "MONOCHROME2");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), frame]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.80");
    assert!(matches!(
        file.decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[cfg(feature = "pixel-j2k")]
#[test]
fn j2k_dicom_file_decodes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/jpeg2000.dcm");
    let file = DicomFile::read_file(path).unwrap();
    let descriptor = file.pixel_descriptor().unwrap();
    let decoded = file.decode_frame(0).unwrap();
    assert_eq!(decoded.len(), descriptor.frame_size());
}

#[cfg(feature = "pixel-jxl")]
const JXL_SAMPLE: &[u8] = &[
    0xff, 0x0a, 0x30, 0x54, 0x10, 0x09, 0x08, 0x06, 0x01, 0x00, 0x78, 0x00, 0x4b, 0x38, 0x41, 0x3c,
    0xb6, 0x3a, 0x51, 0xfe, 0x00, 0x47, 0x1e, 0xa0, 0x85, 0xb8, 0x27, 0x1a, 0x48, 0x45, 0x84, 0x1b,
    0x71, 0x4f, 0xa8, 0x3e, 0x8e, 0x30, 0x03, 0x92, 0x84, 0x01,
];

#[cfg(feature = "pixel-jxl")]
#[test]
fn jxl_sample_codestream_decodes() {
    let image = jxl_oxide::JxlImage::builder().read(JXL_SAMPLE).unwrap();
    let bits = match image.image_header().metadata.bit_depth {
        jxl_oxide::image::BitDepth::IntegerSample { bits_per_sample } => bits_per_sample as u16,
        _ => panic!("expected integer samples"),
    };
    let samples = image.pixel_format().channels() as u16;
    let rows = image.height() as u16;
    let columns = image.width() as u16;
    let mut set = pixel_set(
        rows,
        columns,
        samples,
        bits,
        1,
        if samples == 1 { "MONOCHROME2" } else { "RGB" },
    );
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), JXL_SAMPLE.to_vec()]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.110");
    let descriptor = file.pixel_descriptor().unwrap();
    assert_eq!(file.decode_frame(0).unwrap().len(), descriptor.frame_size());
}

#[cfg(feature = "pixel-jxl")]
#[test]
fn jxl_rejects_malformed_stream() {
    let mut set = pixel_set(1, 1, 1, 8, 8, "MONOCHROME2");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), vec![0xFF, 0x0A, 0x00]]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.110");
    assert!(matches!(
        file.decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[cfg(feature = "pixel-j2k")]
#[test]
fn j2k_rejects_malformed_stream() {
    let mut set = pixel_set(1, 1, 1, 8, 1, "MONOCHROME2");
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), vec![0xFF, 0x4F, 0xFF, 0x51]]),
    );
    let file = file(set, "1.2.840.10008.1.2.4.90");
    assert!(matches!(
        file.decode_frame(0),
        Err(Error::InvalidPixelData(_))
    ));
}

#[test]
fn decode_pixel_data_decodes_every_frame() {
    let mut set = pixel_set(1, 1, 1, 8, 2, "MONOCHROME2");
    let frame = rle_frame(&[literal(&[0x7F])]);
    let _ = set.set(
        tags::PIXEL_DATA,
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), frame.clone(), frame]),
    );
    let file = file(set, RLE_UID);
    assert_eq!(
        file.decode_pixel_data().unwrap(),
        vec![vec![0x7F], vec![0x7F]]
    );
}
