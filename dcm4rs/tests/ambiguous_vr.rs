//! Ambiguous VR correction when writing (`US/SS`, Pixel Data, LUT Data, overlays).

use dcm4rs::dictionary::tags;
use dcm4rs::{DataSet, Endian, Tag, TransferSyntax, Value, Vr};

const PIXEL_DATA: Tag = tags::PIXEL_DATA;
const SMALLEST_IMAGE_PIXEL: Tag = Tag::new(0x0028, 0x0106);
const LUT_DESCRIPTOR: Tag = Tag::new(0x0028, 0x3002);
const LUT_DATA: Tag = Tag::new(0x0028, 0x3006);
const OVERLAY_DATA: Tag = Tag::new(0x6000, 0x3000);

#[test]
fn pixel_data_vr_follows_bits_allocated() {
    let mut set = DataSet::new();
    set.set_u16(tags::BITS_ALLOCATED, 8);
    set.set(PIXEL_DATA, Vr::Ow, Value::from_bytes(vec![1, 2, 3, 4]));
    let bytes = set.write(TransferSyntax::ExplicitVrLittleEndian).unwrap();
    let read = DataSet::read(&bytes, TransferSyntax::ExplicitVrLittleEndian).unwrap();
    assert_eq!(read.get(PIXEL_DATA).unwrap().vr, Vr::Ob);

    let mut set = DataSet::new();
    set.set_u16(tags::BITS_ALLOCATED, 16);
    set.set(PIXEL_DATA, Vr::Ob, Value::from_bytes(vec![1, 2, 3, 4]));
    let bytes = set.write(TransferSyntax::ExplicitVrLittleEndian).unwrap();
    let read = DataSet::read(&bytes, TransferSyntax::ExplicitVrLittleEndian).unwrap();
    assert_eq!(read.get(PIXEL_DATA).unwrap().vr, Vr::Ow);
}

#[test]
fn implicit_pixel_data_is_ow() {
    let mut set = DataSet::new();
    set.set_u16(tags::BITS_ALLOCATED, 8);
    set.set(PIXEL_DATA, Vr::Ob, Value::from_bytes(vec![1, 2, 3, 4]));
    let bytes = set.write(TransferSyntax::ImplicitVrLittleEndian).unwrap();
    let read = DataSet::read(&bytes, TransferSyntax::ImplicitVrLittleEndian).unwrap();
    assert_eq!(read.get(PIXEL_DATA).unwrap().vr, Vr::Ow);
}

#[test]
fn encapsulated_pixel_data_is_ob() {
    let mut set = DataSet::new();
    set.set(
        PIXEL_DATA,
        Vr::Ow,
        Value::Fragments(vec![Vec::new(), vec![1, 2]]),
    );
    let syntax = TransferSyntax::Encapsulated("1.2.840.10008.1.2.5");
    let bytes = set.write(syntax).unwrap();
    let read = DataSet::read(&bytes, syntax).unwrap();
    assert_eq!(read.get(PIXEL_DATA).unwrap().vr, Vr::Ob);
}

#[test]
fn us_ss_follows_pixel_representation() {
    let mut set = DataSet::new();
    set.set_u16(tags::PIXEL_REPRESENTATION, 1);
    set.set(SMALLEST_IMAGE_PIXEL, Vr::Us, Value::from_u16s(&[0xFFFF]));
    let bytes = set.write(TransferSyntax::ExplicitVrLittleEndian).unwrap();
    let read = DataSet::read(&bytes, TransferSyntax::ExplicitVrLittleEndian).unwrap();
    let element = read.get(SMALLEST_IMAGE_PIXEL).unwrap();
    assert_eq!(element.vr, Vr::Ss);
    assert_eq!(element.i16s(Endian::Little).unwrap(), vec![-1]);
}

#[test]
fn lut_data_depends_on_lut_descriptor() {
    let mut set = DataSet::new();
    set.set_u16(LUT_DESCRIPTOR, 1);
    set.set(LUT_DATA, Vr::Ow, Value::from_u16s(&[7]));
    let bytes = set.write(TransferSyntax::ExplicitVrLittleEndian).unwrap();
    let read = DataSet::read(&bytes, TransferSyntax::ExplicitVrLittleEndian).unwrap();
    assert_eq!(read.get(LUT_DATA).unwrap().vr, Vr::Us);

    let mut set = DataSet::new();
    set.set_u16(LUT_DESCRIPTOR, 2);
    set.set(LUT_DATA, Vr::Us, Value::from_u16s(&[7, 8]));
    let bytes = set.write(TransferSyntax::ExplicitVrLittleEndian).unwrap();
    let read = DataSet::read(&bytes, TransferSyntax::ExplicitVrLittleEndian).unwrap();
    assert_eq!(read.get(LUT_DATA).unwrap().vr, Vr::Ow);
}

#[test]
fn overlay_data_is_ow() {
    let mut set = DataSet::new();
    set.set(OVERLAY_DATA, Vr::Ob, Value::from_bytes(vec![1, 2]));
    let bytes = set.write(TransferSyntax::ExplicitVrLittleEndian).unwrap();
    let read = DataSet::read(&bytes, TransferSyntax::ExplicitVrLittleEndian).unwrap();
    assert_eq!(read.get(OVERLAY_DATA).unwrap().vr, Vr::Ow);
}

#[test]
fn sequence_items_use_the_root_pixel_representation() {
    let mut item = DataSet::new();
    item.set(SMALLEST_IMAGE_PIXEL, Vr::Us, Value::from_u16s(&[0xFFFF]));
    let mut root = DataSet::new();
    root.set_u16(tags::PIXEL_REPRESENTATION, 1);
    root.set_sequence(Tag::new(0x0008, 0x1140), vec![item]);
    let bytes = root.write(TransferSyntax::ExplicitVrLittleEndian).unwrap();
    let read = DataSet::read(&bytes, TransferSyntax::ExplicitVrLittleEndian).unwrap();
    let item = &read.sequence(Tag::new(0x0008, 0x1140)).unwrap()[0];
    assert_eq!(item.get(SMALLEST_IMAGE_PIXEL).unwrap().vr, Vr::Ss);
}
