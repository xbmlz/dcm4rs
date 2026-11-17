//! Private creator and private VR resolution with `PrivateDictionary`.

use dcm4rs::dictionary::tags;
use dcm4rs::{DataSet, DicomFile, PrivateDictionary, Tag, TransferSyntax, Value, Vr};

const CREATOR: Tag = Tag::new(0x0029, 0x0010);
const PRIVATE: Tag = Tag::new(0x0029, 0x1010);
const SEQUENCE: Tag = Tag::new(0x0008, 0x1140);

#[test]
fn implicit_private_vr_is_resolved() {
    let mut set = DataSet::new();
    set.set_string(CREATOR, Vr::Lo, "ACME 1.0");
    set.set(PRIVATE, Vr::Us, Value::from_u16s(&[0x1234]));
    let bytes = set.write(TransferSyntax::ImplicitVrLittleEndian).unwrap();

    let without = DataSet::read(&bytes, TransferSyntax::ImplicitVrLittleEndian).unwrap();
    assert_eq!(without.get(PRIVATE).unwrap().vr, Vr::Un);

    let mut dictionary = PrivateDictionary::new();
    dictionary.insert("ACME 1.0", PRIVATE, Vr::Us);
    let with = DataSet::read_with_private_dictionary(
        &bytes,
        TransferSyntax::ImplicitVrLittleEndian,
        &dictionary,
    )
    .unwrap();
    assert_eq!(with.get(PRIVATE).unwrap().vr, Vr::Us);
    assert_eq!(with.u16(PRIVATE).unwrap(), 0x1234);
}

#[test]
fn explicit_un_private_vr_is_resolved() {
    let mut set = DataSet::new();
    set.set_string(CREATOR, Vr::Lo, "ACME 1.0");
    set.set(PRIVATE, Vr::Un, Value::from_u16s(&[0x1234]));
    let bytes = set.write(TransferSyntax::ExplicitVrLittleEndian).unwrap();

    let mut dictionary = PrivateDictionary::new();
    dictionary.insert("ACME 1.0", PRIVATE, Vr::Us);
    let with = DataSet::read_with_private_dictionary(
        &bytes,
        TransferSyntax::ExplicitVrLittleEndian,
        &dictionary,
    )
    .unwrap();
    assert_eq!(with.get(PRIVATE).unwrap().vr, Vr::Us);
}

#[test]
fn sequence_item_private_creator_is_used() {
    let mut item = DataSet::new();
    item.set_string(CREATOR, Vr::Lo, "ACME ITEM");
    item.set(PRIVATE, Vr::Us, Value::from_u16s(&[0x1234]));
    let mut parent = DataSet::new();
    parent.set_sequence(SEQUENCE, vec![item]);
    let bytes = parent
        .write(TransferSyntax::ImplicitVrLittleEndian)
        .unwrap();

    let mut dictionary = PrivateDictionary::new();
    dictionary.insert("ACME ITEM", PRIVATE, Vr::Us);
    let with = DataSet::read_with_private_dictionary(
        &bytes,
        TransferSyntax::ImplicitVrLittleEndian,
        &dictionary,
    )
    .unwrap();
    let item = &with.sequence(SEQUENCE).unwrap()[0];
    assert_eq!(item.get(PRIVATE).unwrap().vr, Vr::Us);
}

#[test]
fn part10_private_vrs_are_resolved() {
    let mut set = DataSet::new();
    set.set_string(tags::SOP_CLASS_UID, Vr::Ui, "1.2.840.10008.5.1.4.1.1.2");
    set.set_string(tags::SOP_INSTANCE_UID, Vr::Ui, "1.2.3.4.5");
    set.set_string(CREATOR, Vr::Lo, "ACME 1.0");
    set.set(PRIVATE, Vr::Us, Value::from_u16s(&[0x1234]));
    let file = DicomFile::new(set, TransferSyntax::ImplicitVrLittleEndian).unwrap();
    let bytes = file.write().unwrap();

    let mut dictionary = PrivateDictionary::new();
    dictionary.insert("ACME 1.0", PRIVATE, Vr::Us);
    let read = DicomFile::read_with_private_dictionary(&bytes, &dictionary).unwrap();
    assert_eq!(read.dataset.get(PRIVATE).unwrap().vr, Vr::Us);
    assert_eq!(read.dataset.u16(PRIVATE).unwrap(), 0x1234);
}
