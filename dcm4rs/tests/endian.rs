//! Byte-order transcoding with `DataSet::to_endian`.

use dcm4rs::{DataSet, Endian, Error, Tag, TransferSyntax, Value, Vr};

const US_TAG: Tag = Tag::new(0x0028, 0x0011);
const FL_TAG: Tag = Tag::new(0x0018, 0x605A);
const CS_TAG: Tag = Tag::new(0x0008, 0x0008);
const SQ_TAG: Tag = Tag::new(0x0008, 0x1140);
const AT_TAG: Tag = Tag::new(0x0020, 0x9165);
const OB_TAG: Tag = Tag::new(0x0042, 0x0011);
const OW_TAG: Tag = Tag::new(0x7FE0, 0x0010);
const FRAGMENT_TAG: Tag = Tag::new(0x7FE0, 0x0011);

#[test]
fn numeric_values_and_sequences_transcode() {
    let mut child = DataSet::new();
    child.set_u16(Tag::new(0x0028, 0x0010), 512);
    child.set_string(Tag::new(0x0010, 0x0010), Vr::Pn, "Doe^John");

    let mut set = DataSet::new();
    set.set_u16(US_TAG, 0x1234);
    set.set(FL_TAG, Vr::Fl, Value::from_f32s(&[1.5]));
    set.set_string(CS_TAG, Vr::Cs, "ORIGINAL");
    set.set_sequence(SQ_TAG, vec![child]);

    let big = set.to_endian(Endian::Big).unwrap();
    assert_eq!(big.endian(), Endian::Big);
    assert_eq!(big.u16(US_TAG).unwrap(), 0x1234);
    assert_eq!(
        big.get(FL_TAG).unwrap().f32s(Endian::Big).unwrap(),
        vec![1.5]
    );
    assert_eq!(big.string(CS_TAG).unwrap(), "ORIGINAL");
    let item = &big.sequence(SQ_TAG).unwrap()[0];
    assert_eq!(item.endian(), Endian::Big);
    assert_eq!(item.u16(Tag::new(0x0028, 0x0010)).unwrap(), 512);

    let bytes = big.write(TransferSyntax::ExplicitVrBigEndian).unwrap();
    let read = DataSet::read(&bytes, TransferSyntax::ExplicitVrBigEndian).unwrap();
    assert_eq!(read.u16(US_TAG).unwrap(), 0x1234);
    assert_eq!(
        read.get(FL_TAG).unwrap().f32s(Endian::Big).unwrap(),
        vec![1.5]
    );
    assert_eq!(
        read.sequence(SQ_TAG).unwrap()[0]
            .u16(Tag::new(0x0028, 0x0010))
            .unwrap(),
        512
    );

    let back = big.to_endian(Endian::Little).unwrap();
    assert_eq!(back.endian(), Endian::Little);
    assert_eq!(back.u16(US_TAG).unwrap(), 0x1234);
    assert_eq!(back.sequence(SQ_TAG).unwrap()[0].endian(), Endian::Little);
}

#[test]
fn attribute_tags_keep_group_and_element_order() {
    let mut set = DataSet::new();
    set.set(
        AT_TAG,
        Vr::At,
        Value::from_bytes(vec![0x10, 0x00, 0x20, 0x00]),
    );
    let big = set.to_endian(Endian::Big).unwrap();
    assert_eq!(
        big.get(AT_TAG).unwrap().tags(Endian::Big).unwrap(),
        vec![Tag::new(0x0010, 0x0020)]
    );
}

#[test]
fn ob_and_fragments_are_unchanged() {
    let fragments = Value::Fragments(vec![Vec::new(), vec![0xFF, 0xD8]]);
    let mut set = DataSet::new();
    set.set(OB_TAG, Vr::Ob, Value::from_bytes(vec![1, 2, 3]));
    set.set(OW_TAG, Vr::Ow, Value::from_bytes(vec![1, 2, 3, 4]));
    set.set(FRAGMENT_TAG, Vr::Ob, fragments.clone());

    let big = set.to_endian(Endian::Big).unwrap();
    assert_eq!(big.get(OB_TAG).unwrap().raw_value(), &[1, 2, 3]);
    assert_eq!(big.get(OW_TAG).unwrap().raw_value(), &[2, 1, 4, 3]);
    assert_eq!(big.get(FRAGMENT_TAG).unwrap().value, fragments);
}

#[test]
fn wrong_numeric_length_is_rejected() {
    let mut set = DataSet::new();
    set.set(US_TAG, Vr::Us, Value::from_bytes(vec![0x01]));
    assert!(matches!(
        set.to_endian(Endian::Big),
        Err(Error::WrongLength {
            vr: Vr::Us,
            len: 1,
            unit: 2
        })
    ));
}
