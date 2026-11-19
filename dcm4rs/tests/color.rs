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

#[test]
fn colour_spaces_map_to_their_photometric_interpretation() {
    for photometric in [
        PhotometricInterpretation::Rgb,
        PhotometricInterpretation::YbrFull,
        PhotometricInterpretation::YbrFull422,
        PhotometricInterpretation::YbrPartial422,
        PhotometricInterpretation::YbrIct,
        PhotometricInterpretation::YbrRct,
    ] {
        let space = ColorSpace::from_photometric(&photometric).expect("colour space");
        assert_eq!(space.photometric(), photometric);
        assert_eq!(space.to_string(), photometric.to_string());
    }
    assert!(ColorSpace::from_photometric(&PhotometricInterpretation::Monochrome2).is_none());
}

#[test]
fn converting_to_the_same_space_returns_the_frame() {
    let frame = [1u8, 2, 3];
    let rgb = convert_color_space(
        &frame,
        &descriptor(PhotometricInterpretation::Rgb, 3),
        ColorSpace::Rgb,
    )
    .unwrap();
    assert_eq!(rgb, frame);
}

#[test]
fn monochrome_frames_have_no_colour_space() {
    let frame = [1u8];
    let error = convert_color_space(
        &frame,
        &descriptor(PhotometricInterpretation::Monochrome2, 1),
        ColorSpace::Rgb,
    )
    .unwrap_err();
    assert!(matches!(error, dcm4rs::Error::UnsupportedPixelData(_)));

    let frame = [1u8, 2, 3];
    let error = convert_color_space(
        &frame,
        &descriptor(PhotometricInterpretation::Hsv, 3),
        ColorSpace::Rgb,
    )
    .unwrap_err();
    assert!(matches!(error, dcm4rs::Error::UnsupportedPixelData(_)));
}

#[test]
fn ybr_partial_422_decodes_to_rgb() {
    // Y=235, Cb=Cr=128 is white in the partial range.
    let set = pixel_set(1, 2, 3, 8, "YBR_PARTIAL_422", &[235, 235, 128, 128]);
    let file = file(set, EXPLICIT_LITTLE);
    let rgb = file.decode_rgb_frame(0).unwrap();
    assert_eq!(rgb, [255, 255, 255, 255, 255, 255]);
}

#[test]
fn unsupported_conversion_pairs_are_refused() {
    let frame = [1u8, 2, 3];
    let error = convert_color_space(
        &frame,
        &descriptor(PhotometricInterpretation::YbrFull422, 3),
        ColorSpace::YbrFull,
    )
    .unwrap_err();
    assert!(matches!(error, dcm4rs::Error::UnsupportedPixelData(_)));
}

#[test]
fn rgb_frames_decode_unchanged() {
    let set = pixel_set(1, 1, 3, 8, "RGB", &[1, 2, 3]);
    let file = file(set, EXPLICIT_LITTLE);
    assert_eq!(file.decode_rgb_frame(0).unwrap(), [1, 2, 3]);
}

#[test]
fn monochrome_frames_decode_unchanged() {
    let set = pixel_set(1, 2, 1, 8, "MONOCHROME2", &[7, 9]);
    let file = file(set, EXPLICIT_LITTLE);
    assert_eq!(file.decode_rgb_frame(0).unwrap(), [7, 9]);
}

#[test]
fn apply_color_lut_maps_the_frame() {
    let file = file(palette_set(8), EXPLICIT_LITTLE);
    assert_eq!(file.apply_color_lut(0).unwrap(), [0, 0, 0, 255, 255, 255]);
}

/// A palette data set whose descriptors and tables use the Large attributes.
fn large_palette_set() -> DataSet {
    let mut set = pixel_set(1, 2, 1, 8, "PALETTE COLOR", &[0, 1]);
    for (descriptor, data) in [
        (
            tags::LARGE_RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            tags::LARGE_RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        ),
        (
            tags::LARGE_GREEN_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            tags::LARGE_GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        ),
        (
            tags::LARGE_BLUE_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            tags::LARGE_BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        ),
    ] {
        let _ = set.set(descriptor, Vr::Us, Value::from_u16s(&[0, 2, 16]));
        let _ = set.set(data, Vr::Ow, Value::from_u16s(&[0x0000, 0xFFFF]));
    }
    set
}

#[test]
fn large_tables_are_read_from_their_own_attributes() {
    let file = file(large_palette_set(), EXPLICIT_LITTLE);
    let rgb = file.decode_rgb_frame(0).unwrap();
    assert_eq!(rgb, [0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
}

#[test]
fn a_segmented_table_is_reported_as_unsupported() {
    let mut set = palette_set(8);
    let _ = set.set(
        tags::SEGMENTED_RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        Vr::Ow,
        Value::from_u16s(&[0, 1, 2]),
    );
    let error = PaletteColorLut::from_dataset(&set).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::UnsupportedPixelData(_)));
}

#[test]
fn a_short_descriptor_is_rejected() {
    let mut set = palette_set(8);
    let _ = set.set(
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
        Vr::Us,
        Value::from_u16s(&[0, 2]),
    );
    let error = PaletteColorLut::from_dataset(&set).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::InvalidPixelData(_)));
}

#[test]
fn entry_widths_other_than_eight_and_sixteen_are_refused() {
    let mut set = palette_set(8);
    let _ = set.set(
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
        Vr::Us,
        Value::from_u16s(&[0, 2, 12]),
    );
    let error = PaletteColorLut::from_dataset(&set).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::UnsupportedPixelData(_)));
}

#[test]
fn a_table_of_one_byte_per_entry_is_read() {
    let mut set = palette_set(8);
    for tag in [
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
    ] {
        let _ = set.set(tag, Vr::Ow, Value::from_bytes(vec![0, 255]));
    }
    let lut = PaletteColorLut::from_dataset(&set).unwrap();
    assert_eq!(lut.apply(&[0, 1], 8).unwrap(), [0, 0, 0, 255, 255, 255]);
}

#[test]
fn an_entry_wider_than_eight_bits_is_rejected() {
    let mut set = palette_set(8);
    let _ = set.set(
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        Vr::Ow,
        Value::from_u16s(&[0x1234, 0x5678]),
    );
    let error = PaletteColorLut::from_dataset(&set).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::InvalidPixelData(_)));
}

#[test]
fn short_tables_are_rejected() {
    let mut set = palette_set(8);
    let _ = set.set(
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
        Vr::Us,
        Value::from_u16s(&[0, 4, 8]),
    );
    let _ = set.set(
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        Vr::Ow,
        Value::from_bytes(vec![0, 1]),
    );
    let error = PaletteColorLut::from_dataset(&set).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::InvalidPixelData(_)));

    let mut set = palette_set(16);
    let _ = set.set(
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
        Vr::Us,
        Value::from_u16s(&[0, 4, 16]),
    );
    let _ = set.set(
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        Vr::Ow,
        Value::from_bytes(vec![0, 1]),
    );
    let error = PaletteColorLut::from_dataset(&set).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::InvalidPixelData(_)));
}

#[test]
fn tables_are_read_from_a_big_endian_data_set() {
    let mut set = pixel_set(1, 2, 1, 8, "PALETTE COLOR", &[0, 1]);
    set.set_endian(dcm4rs::Endian::Big);
    for (descriptor, data) in [
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
            descriptor,
            Vr::Us,
            Value::from_bytes(vec![0, 0, 0, 2, 0, 8]),
        );
        let _ = set.set(data, Vr::Ow, Value::from_bytes(vec![0, 0, 0, 255]));
    }
    let lut = PaletteColorLut::from_dataset(&set).unwrap();
    assert_eq!(lut.apply(&[0, 1], 8).unwrap(), [0, 0, 0, 255, 255, 255]);

    // Sixteen-bit entries take the same path, byte-swapped on the way out.
    set.set_endian(dcm4rs::Endian::Big);
    let _ = set.set(
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
        Vr::Us,
        Value::from_bytes(vec![0, 0, 0, 2, 0, 16]),
    );
    for tag in [
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
    ] {
        let _ = set.set(tag, Vr::Ow, Value::from_bytes(vec![0, 0, 0xFF, 0xFF]));
    }
    let lut = PaletteColorLut::from_dataset(&set).unwrap();
    assert_eq!(lut.bits_per_entry(), 16);
    assert_eq!(
        lut.apply(&[1], 8).unwrap(),
        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
    );
}

#[test]
fn indexes_wider_than_sixteen_bits_are_refused() {
    let lut = PaletteColorLut::from_dataset(&palette_set(8)).unwrap();
    let error = lut.apply(&[0, 0, 0, 0], 32).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::UnsupportedPixelData(_)));

    let error = lut.apply(&[0], 16).unwrap_err();
    assert!(matches!(error, dcm4rs::Error::InvalidPixelData(_)));
}

#[test]
fn sixteen_bit_indexes_select_entries() {
    let mut set = pixel_set(1, 2, 1, 16, "PALETTE COLOR", &[0, 0, 1, 0]);
    for (descriptor, data) in [
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
        let _ = set.set(descriptor, Vr::Us, Value::from_u16s(&[0, 2, 8]));
        let _ = set.set(data, Vr::Ow, Value::from_u16s(&[0x0000, 0x00FF]));
    }
    let file = file(set, EXPLICIT_LITTLE);
    assert_eq!(file.decode_rgb_frame(0).unwrap(), [0, 0, 0, 255, 255, 255]);
}

#[test]
fn sixteen_bit_samples_are_stored_as_ow() {
    let mut file = file(
        pixel_set(1, 1, 1, 16, "MONOCHROME2", &[0, 0]),
        EXPLICIT_LITTLE,
    );
    file.set_pixel_data(vec![1, 0]).unwrap();
    assert_eq!(file.dataset.element(tags::PIXEL_DATA).unwrap().vr, Vr::Ow);
}
