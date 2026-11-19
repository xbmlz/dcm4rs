//! Colour conversion, palette lookup, pixel write-back and decompression.
//!
//! pydicom pins these behaviours in `test_pixels_processing.py` (conversions), `test_lut.py`
//! (palette tables) and `test_dataset.py` (`decompress`, `set_pixel_data`); these cases port the
//! parts that apply to the API this crate exposes.

use dcm4rs::dictionary::tags;
use dcm4rs::pixels::{convert_color_space, pack_bits, unpack_bits, ColorSpace, PaletteColorLut};
use dcm4rs::{DataSet, DicomFile, PhotometricInterpretation, Value, Vr};

const EXPLICIT_LITTLE: &str = "1.2.840.10008.1.2.1";

/// A data set carrying the attributes a pixel descriptor requires.
fn pixel_set(
    rows: u16,
    columns: u16,
    samples: u16,
    bits: u16,
    photometric: &str,
    data: &[u8],
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
    let _ = set.set(tags::PIXEL_DATA, Vr::Ob, Value::from_bytes(data.to_vec()));
    set
}

fn file(mut dataset: DataSet, uid: &str) -> DicomFile {
    let mut meta = DataSet::new();
    let _ = meta.set(tags::TRANSFER_SYNTAX_UID, Vr::Ui, Value::from_str(uid));
    dataset.set_endian(dcm4rs::Endian::Little);
    DicomFile::from_parts(meta, dataset, [0u8; 128])
}

fn descriptor(photometric: PhotometricInterpretation, samples: u16) -> dcm4rs::PixelDescriptor {
    dcm4rs::PixelDescriptor {
        rows: 1,
        columns: 2,
        samples_per_pixel: samples,
        bits_allocated: 8,
        bits_stored: 8,
        high_bit: 7,
        pixel_representation: dcm4rs::PixelRepresentation::Unsigned,
        planar_configuration: dcm4rs::PlanarConfiguration::Interleaved,
        photometric,
        number_of_frames: 1,
    }
}

#[test]
fn ybr_full_decodes_to_rgb() {
    let set = pixel_set(2, 1, 3, 8, "YBR_FULL", &[255, 128, 128, 0, 128, 128]);
    let file = file(set, EXPLICIT_LITTLE);
    let rgb = file.decode_rgb_frame(0).unwrap();
    assert_eq!(rgb, [255, 255, 255, 0, 0, 0]);
}

#[test]
fn ybr_full_422_decodes_to_rgb() {
    let set = pixel_set(1, 2, 3, 8, "YBR_FULL_422", &[255, 255, 128, 128]);
    let file = file(set, EXPLICIT_LITTLE);
    let rgb = file.decode_rgb_frame(0).unwrap();
    assert_eq!(rgb, [255, 255, 255, 255, 255, 255]);
}

#[test]
fn wide_samples_are_refused_by_the_converter() {
    let frame = [1u8, 2, 3];
    let mut descriptor = descriptor(PhotometricInterpretation::YbrFull, 3);
    descriptor.bits_allocated = 16;
    let error = convert_color_space(&frame, &descriptor, ColorSpace::Rgb).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::UnsupportedPixelData(_)));
}

#[test]
fn ybr_ict_is_refused_because_the_decoder_already_applies_it() {
    let frame = [1u8, 2, 3];
    let error = convert_color_space(
        &frame,
        &descriptor(PhotometricInterpretation::YbrIct, 3),
        ColorSpace::Rgb,
    )
    .unwrap_err();
    assert!(matches!(error, dcm4rs::Error::UnsupportedPixelData(_)));
}

#[test]
fn bit_packing_round_trips_through_the_public_api() {
    let packed = [0b1000_0001u8];
    assert_eq!(unpack_bits(&packed)[..2], [1, 0]);
    assert_eq!(pack_bits(&unpack_bits(&packed)).unwrap(), packed);
}

/// Build a data set whose palette tables map index 0 to black and index 1 to white.
fn palette_set(bits_per_entry: u16) -> DataSet {
    let mut set = pixel_set(1, 2, 1, 8, "PALETTE COLOR", &[0, 1]);
    // Eight-bit entries sit in the low byte of each word; sixteen-bit entries fill the word.
    let white = if bits_per_entry == 8 { 0x00FF } else { 0xFFFF };
    for (descriptor_tag, data_tag) in [
        (
            tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        ),
        (
            tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        ),
        (
            tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        ),
    ] {
        let _ = set.set(
            descriptor_tag,
            Vr::Us,
            Value::from_u16s(&[0, 2, bits_per_entry]),
        );
        let _ = set.set(data_tag, Vr::Ow, Value::from_u16s(&[0, white]));
    }
    set
}

#[test]
fn eight_bit_palette_maps_indexes_to_rgb() {
    let file = file(palette_set(8), EXPLICIT_LITTLE);
    assert_eq!(file.decode_rgb_frame(0).unwrap(), [0, 0, 0, 255, 255, 255]);
}

#[test]
fn sixteen_bit_palette_writes_two_bytes_per_sample() {
    let file = file(palette_set(16), EXPLICIT_LITTLE);
    let rgb = file.decode_rgb_frame(0).unwrap();
    assert_eq!(rgb, [0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn indexes_beyond_the_table_are_clamped() {
    let mut set = palette_set(8);
    let _ = set.set(tags::PIXEL_DATA, Vr::Ob, Value::from_bytes(vec![7, 200]));
    let file = file(set, EXPLICIT_LITTLE);
    let rgb = file.decode_rgb_frame(0).unwrap();
    assert_eq!(rgb.len(), 6);
    assert_eq!(&rgb[..3], &rgb[3..]);
}

#[test]
fn the_lookup_table_is_read_from_the_data_set() {
    let set = palette_set(8);
    let lut = PaletteColorLut::from_dataset(&set).unwrap();
    assert_eq!(lut.entries(), 2);
    assert_eq!(lut.first_mapped_index(), 0);
    assert_eq!(lut.bits_per_entry(), 8);
}

#[test]
fn a_lookup_table_needs_a_palette_image() {
    let file = file(pixel_set(1, 1, 1, 8, "MONOCHROME2", &[0]), EXPLICIT_LITTLE);
    let error = file.apply_color_lut(0).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::InvalidPixelData(_)));
}

#[test]
fn set_pixel_data_keeps_the_frame_count_in_step() {
    let mut file = file(
        pixel_set(2, 2, 1, 8, "MONOCHROME2", &[0, 1, 2, 3]),
        EXPLICIT_LITTLE,
    );
    file.set_pixel_data(vec![9, 8, 7, 6, 5, 4, 3, 2]).unwrap();
    assert_eq!(file.dataset.string(tags::NUMBER_OF_FRAMES).unwrap(), "2");
    assert_eq!(file.pixel_frames().unwrap().len(), 2);

    file.set_pixel_data(vec![1, 2, 3, 4]).unwrap();
    assert!(file.dataset.get(tags::NUMBER_OF_FRAMES).is_none());
    assert_eq!(file.pixel_frames().unwrap().len(), 1);

    let error = file.set_pixel_data(vec![1, 2, 3]).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::InvalidPixelData(_)));
}

#[test]
fn an_encapsulated_file_refuses_native_samples() {
    let mut file = file(
        pixel_set(2, 2, 1, 8, "MONOCHROME2", &[0, 1, 2, 3]),
        EXPLICIT_LITTLE,
    );
    file.compress_rle().unwrap();
    let error = file.set_pixel_data(vec![1, 2, 3, 4]).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::InvalidPixelData(_)));
}

#[test]
fn rle_round_trips_through_decompress() {
    let samples = [10u8, 20, 30, 40, 50, 60];
    let mut file = file(
        pixel_set(2, 3, 1, 8, "MONOCHROME2", &samples),
        EXPLICIT_LITTLE,
    );
    file.compress_rle().unwrap();
    assert!(file.transfer_syntax().unwrap().is_encapsulated());
    file.decompress().unwrap();
    assert_eq!(file.transfer_syntax().unwrap().uid(), EXPLICIT_LITTLE);
    assert_eq!(file.pixel_frames().unwrap(), vec![samples.to_vec()]);
    // RLE keeps the coded samples, so the photometric interpretation still describes them.
    assert_eq!(
        file.dataset
            .string(tags::PHOTOMETRIC_INTERPRETATION)
            .unwrap(),
        "MONOCHROME2"
    );
}

#[test]
fn decompress_refuses_a_native_file() {
    let mut file = file(pixel_set(1, 1, 1, 8, "MONOCHROME2", &[0]), EXPLICIT_LITTLE);
    let error = file.decompress().unwrap_err();
    assert!(matches!(error, dcm4rs::Error::UnsupportedPixelData(_)));
}

#[cfg(feature = "pixel-jpeg")]
#[test]
fn jpeg_decodes_back_to_rgb_on_decompress() {
    let mut file = file(
        pixel_set(8, 8, 3, 8, "RGB", &[200u8; 8 * 8 * 3]),
        EXPLICIT_LITTLE,
    );
    file.compress_jpeg(90).unwrap();
    file.decompress().unwrap();
    assert_eq!(file.transfer_syntax().unwrap().uid(), EXPLICIT_LITTLE);
    assert_eq!(
        file.dataset
            .string(tags::PHOTOMETRIC_INTERPRETATION)
            .unwrap(),
        "RGB"
    );
    assert_eq!(file.dataset.u16(tags::PLANAR_CONFIGURATION).unwrap(), 0);
    let frame = file.pixel_frames().unwrap().remove(0);
    assert_eq!(frame.len(), 8 * 8 * 3);
    assert!(frame[0].abs_diff(200) <= 8, "first sample {}", frame[0]);
}
