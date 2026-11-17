//! Writing behaviour, mirroring pydicom's `tests/test_filewriter.py`.
//!
//! Covered: byte-level output for the native syntaxes, padding, empty values, sequence length
//! computation, encapsulated pixel data, canonicalization and the errors the writer raises.
//!
//! Divergences from pydicom asserted below and listed in the README: group length elements are
//! dropped, the file meta carries no `(0002,0000)`, values are written exactly as stored (no
//! byte-order conversion, no ambiguous-VR correction) and the preamble is always zeroed.

mod common;

use dcm4rs::{DataSet, DicomFile, Endian, Error, Tag, TransferSyntax, Value, Vr};

use common::{
    delimiter_le, element_be, element_implicit, element_le, element_le_undefined, item_le,
    meta_transfer_syntax, part10, tag, tag_le, SEQUENCE_DELIMITER,
};

const EXPLICIT_LE: TransferSyntax = TransferSyntax::ExplicitVrLittleEndian;
const IMPLICIT_LE: TransferSyntax = TransferSyntax::ImplicitVrLittleEndian;
const EXPLICIT_BE: TransferSyntax = TransferSyntax::ExplicitVrBigEndian;

fn sample_dataset() -> DataSet {
    let mut set = DataSet::new();
    set.set_string(tag(0x0008, 0x0016), Vr::Ui, "1.2.840.10008.5.1.4.1.1.2");
    set.set_string(tag(0x0008, 0x0018), Vr::Ui, "1.2.3.4.5");
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    set
}

#[test]
fn explicit_le_output_is_byte_exact() {
    let mut set = DataSet::new();
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    set.set_u16(tag(0x0028, 0x0010), 512);

    let mut expected = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    expected.extend(element_le("US", tag(0x0028, 0x0010), &[0x00, 0x02]));
    assert_eq!(set.write(EXPLICIT_LE).unwrap(), expected);
    assert_eq!(DataSet::read(&expected, EXPLICIT_LE).unwrap(), set);
}

#[test]
fn implicit_le_output_is_byte_exact() {
    let mut set = DataSet::new();
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    set.set_u16(tag(0x0028, 0x0010), 512);

    let mut expected = element_implicit(tag(0x0010, 0x0010), b"Doe^John");
    expected.extend(element_implicit(tag(0x0028, 0x0010), &[0x00, 0x02]));
    assert_eq!(set.write(IMPLICIT_LE).unwrap(), expected);

    let decoded = DataSet::read(&expected, IMPLICIT_LE).unwrap();
    assert_eq!(decoded.u16(tag(0x0028, 0x0010)).unwrap(), 512);
}

#[test]
fn explicit_be_output_is_byte_exact() {
    let mut set = DataSet::with_endian(Endian::Big);
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    set.set(
        tag(0x0028, 0x0010),
        Vr::Us,
        Value::from_bytes(vec![0x02, 0x00]),
    );

    let mut expected = element_be("PN", tag(0x0010, 0x0010), b"Doe^John");
    expected.extend(element_be("US", tag(0x0028, 0x0010), &[0x02, 0x00]));
    assert_eq!(set.write(EXPLICIT_BE).unwrap(), expected);

    let decoded = DataSet::read(&expected, EXPLICIT_BE).unwrap();
    assert_eq!(decoded.u16(tag(0x0028, 0x0010)).unwrap(), 512);
}

#[test]
fn empty_data_set_writes_no_bytes() {
    assert!(DataSet::new().write(EXPLICIT_LE).unwrap().is_empty());
}

#[test]
fn odd_values_are_padded_per_vr() {
    // Textual VRs pad with a space.
    let mut set = DataSet::new();
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^Johns");
    let bytes = set.write(EXPLICIT_LE).unwrap();
    assert_eq!(bytes.last().copied(), Some(b' '));
    let length = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    assert_eq!(length, 10, "the length field counts the pad byte");
    assert_eq!(
        DataSet::read(&bytes, EXPLICIT_LE)
            .unwrap()
            .string(tag(0x0010, 0x0010))
            .unwrap(),
        "Doe^Johns"
    );

    // UIDs pad with NUL.
    let mut set = DataSet::new();
    set.set_string(tag(0x0020, 0x000D), Vr::Ui, "1.2.3");
    let bytes = set.write(EXPLICIT_LE).unwrap();
    assert_eq!(bytes.last().copied(), Some(0));
    assert_eq!(
        DataSet::read(&bytes, EXPLICIT_LE)
            .unwrap()
            .string(tag(0x0020, 0x000D))
            .unwrap(),
        "1.2.3"
    );

    // Binary VRs pad with NUL and use the long length form.
    let mut set = DataSet::new();
    set.set(
        tag(0x0011, 0x1001),
        Vr::Ob,
        Value::from_bytes(vec![1, 2, 3]),
    );
    let bytes = set.write(EXPLICIT_LE).unwrap();
    assert_eq!(bytes.last().copied(), Some(0));
    assert_eq!(&bytes[8..12], &[0x04, 0x00, 0x00, 0x00]);
}

#[test]
fn zero_length_values_round_trip_for_every_vr() {
    let mut set = DataSet::new();
    // Elements start at 0x0100 so none falls in the private creator range (0x0010-0x00FF),
    // which the reader always restores to `LO` (pydicom/hooks.py:139-152).
    for (index, vr) in Vr::ALL.into_iter().enumerate() {
        set.set(tag(0x7777, 0x0100 + index as u16), vr, Value::Empty);
    }
    let bytes = set.write(EXPLICIT_LE).unwrap();
    let decoded = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    assert_eq!(decoded.len(), Vr::ALL.len());
    for (written, read_back) in set.iter().zip(decoded.iter()) {
        assert_eq!(written.tag, read_back.tag);
        assert_eq!(written.vr, read_back.vr);
        assert!(read_back.is_empty());
    }
}

#[test]
fn sequences_get_computed_lengths() {
    let mut item = DataSet::new();
    item.set_string(tag(0x0008, 0x0100), Vr::Sh, "CODE");
    let mut set = DataSet::new();
    set.set_sequence(tag(0x0040, 0x0275), vec![item]);

    let item_body = element_le("SH", tag(0x0008, 0x0100), b"CODE");
    let mut stream = item_le(item_body.len() as u32);
    stream.extend_from_slice(&item_body);
    let expected = element_le("SQ", tag(0x0040, 0x0275), &stream);

    assert_eq!(set.write(EXPLICIT_LE).unwrap(), expected);
    assert_eq!(DataSet::read(&expected, EXPLICIT_LE).unwrap(), set);
}

#[test]
fn empty_sequences_are_written_with_a_zero_length() {
    let mut set = DataSet::new();
    set.set_sequence(tag(0x0040, 0x0275), Vec::new());
    let bytes = set.write(EXPLICIT_LE).unwrap();
    assert_eq!(bytes, element_le("SQ", tag(0x0040, 0x0275), &[]));
    assert_eq!(&bytes[8..12], &[0x00, 0x00, 0x00, 0x00]);
}

#[test]
fn nested_sequences_are_written_inside_out() {
    let mut leaf = DataSet::new();
    leaf.set_string(tag(0x0008, 0x0100), Vr::Sh, "CODE");
    let mut middle = DataSet::new();
    middle.set_sequence(tag(0x0040, 0xA730), vec![leaf]);
    let mut top = DataSet::new();
    top.set_sequence(tag(0x0040, 0x0275), vec![middle]);

    let leaf_body = element_le("SH", tag(0x0008, 0x0100), b"CODE");
    let mut leaf_item = item_le(leaf_body.len() as u32);
    leaf_item.extend_from_slice(&leaf_body);
    let middle_element = element_le("SQ", tag(0x0040, 0xA730), &leaf_item);
    let mut middle_item = item_le(middle_element.len() as u32);
    middle_item.extend_from_slice(&middle_element);
    let expected = element_le("SQ", tag(0x0040, 0x0275), &middle_item);

    assert_eq!(top.write(EXPLICIT_LE).unwrap(), expected);
}

#[test]
fn encapsulated_fragments_are_written_with_undefined_length() {
    let mut set = DataSet::new();
    set.set(
        tag(0x7FE0, 0x0010),
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), vec![1, 2]]),
    );

    let mut stream = item_le(0); // Basic Offset Table
    stream.extend(item_le(2));
    stream.extend_from_slice(&[1, 2]);
    stream.extend(delimiter_le(SEQUENCE_DELIMITER));
    let expected = element_le_undefined("OB", tag(0x7FE0, 0x0010), &stream);

    assert_eq!(set.write(EXPLICIT_LE).unwrap(), expected);
    let decoded = DataSet::read(&expected, EXPLICIT_LE).unwrap();
    assert!(decoded
        .get(tag(0x7FE0, 0x0010))
        .unwrap()
        .value
        .is_fragments());
}

#[test]
fn odd_fragments_are_padded() {
    // PS3.5 A.4: item lengths are even, so an odd fragment gains a trailing zero that its length
    // field counts (pydicom pads odd payloads on write too).
    let mut set = DataSet::new();
    set.set(
        tag(0x7FE0, 0x0010),
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), vec![1, 2, 3]]),
    );
    let bytes = set.write(EXPLICIT_LE).unwrap();
    // Header 12 bytes, then the BOT item (8 bytes), then the fragment item tag at 20..24 and
    // its length field at 24..28.
    assert_eq!(&bytes[24..28], &[0x04, 0x00, 0x00, 0x00]);
    assert_eq!(bytes.len(), 12 + 8 + 8 + 4 + 8);

    let decoded = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    let Value::Fragments(fragments) = &decoded.get(tag(0x7FE0, 0x0010)).unwrap().value else {
        panic!("expected fragments");
    };
    assert_eq!(fragments, &[Vec::new(), vec![1, 2, 3, 0]]);
}

#[test]
fn group_length_elements_are_dropped() {
    let mut set = DataSet::new();
    set.set(tag(0x0010, 0x0000), Vr::Ul, Value::from_u32s(&[100]));
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");

    let expected = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    assert_eq!(set.write(EXPLICIT_LE).unwrap(), expected);
    assert!(!DataSet::read(&expected, EXPLICIT_LE)
        .unwrap()
        .contains(tag(0x0010, 0x0000)));
}

#[test]
fn values_are_written_without_converting_byte_order() {
    let mut set = DataSet::with_endian(Endian::Big);
    set.set(
        tag(0x0028, 0x0010),
        Vr::Us,
        Value::from_bytes(vec![0x02, 0x00]),
    );
    assert_eq!(set.u16(tag(0x0028, 0x0010)).unwrap(), 512);

    let bytes = set.write(EXPLICIT_LE).unwrap();
    assert_eq!(bytes, element_le("US", tag(0x0028, 0x0010), &[0x02, 0x00]));
    let decoded = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    assert_eq!(decoded.u16(tag(0x0028, 0x0010)).unwrap(), 2);
}

#[test]
fn sequences_must_hold_items_not_bytes() {
    let mut set = DataSet::new();
    set.set(tag(0x0040, 0x0275), Vr::Sq, Value::from_bytes(vec![1, 2]));
    for syntax in [EXPLICIT_LE, IMPLICIT_LE] {
        assert!(matches!(
            set.write(syntax),
            Err(Error::InvalidData {
                reason: "sequence element holding a byte value",
                ..
            })
        ));
    }
}

#[test]
fn fragments_require_a_binary_vr() {
    let mut set = DataSet::new();
    set.set(
        tag(0x0010, 0x0010),
        Vr::Pn,
        Value::Fragments(vec![vec![1, 2]]),
    );
    assert!(matches!(
        set.write(EXPLICIT_LE),
        Err(Error::InvalidData {
            reason: "encapsulated fragments on a non-pixel-data element",
            ..
        })
    ));
}

#[test]
fn deflated_writing_round_trips() {
    let set = sample_dataset();
    let file = DicomFile::new(set, TransferSyntax::DeflatedExplicitVrLittleEndian).unwrap();
    let bytes = file.write().unwrap();
    let read = DicomFile::read(&bytes).unwrap();
    assert_eq!(
        read.transfer_syntax().unwrap(),
        TransferSyntax::DeflatedExplicitVrLittleEndian
    );
    assert_eq!(
        read.dataset.string(tag(0x0010, 0x0010)).unwrap(),
        "Doe^John"
    );
    assert_eq!(
        read.dataset.string(tag(0x0008, 0x0016)).unwrap(),
        "1.2.840.10008.5.1.4.1.1.2"
    );
}

#[test]
fn short_form_overflow_is_rejected() {
    let mut set = DataSet::new();
    set.set(
        tag(0x0010, 0x0010),
        Vr::Pn,
        Value::from_bytes(vec![b'A'; 70_000]),
    );
    assert!(matches!(
        set.write(EXPLICIT_LE),
        Err(Error::InvalidData {
            reason: "value too long for a short-length VR",
            ..
        })
    ));
}

#[test]
fn new_file_fills_the_meta_group() {
    let file = DicomFile::new(sample_dataset(), EXPLICIT_LE).unwrap();
    assert_eq!(
        file.meta.string(tag(0x0002, 0x0010)).unwrap(),
        EXPLICIT_LE.uid()
    );
    assert_eq!(
        file.meta.string(tag(0x0002, 0x0002)).unwrap(),
        "1.2.840.10008.5.1.4.1.1.2"
    );
    assert_eq!(file.meta.string(tag(0x0002, 0x0003)).unwrap(), "1.2.3.4.5");
    assert_eq!(
        file.meta.get(tag(0x0002, 0x0001)).unwrap().raw_value(),
        &[0, 1]
    );

    let class_uid = file.meta.string(tag(0x0002, 0x0012)).unwrap();
    assert!(class_uid.starts_with("2.25."), "{class_uid}");
    let version = file.meta.string(tag(0x0002, 0x0013)).unwrap();
    assert!(version.starts_with("DCM4RS_"), "{version}");
    assert!(version.len() <= 16, "{version}");

    // Divergence from pydicom: the file meta group length is not written.
    assert!(file.meta.get(tag(0x0002, 0x0000)).is_none());
}

#[test]
fn new_file_requires_the_sop_uids() {
    assert!(matches!(
        DicomFile::new(DataSet::new(), EXPLICIT_LE),
        Err(Error::MissingTag(tag)) if tag == common::tag(0x0008, 0x0016)
    ));

    let mut set = DataSet::new();
    set.set_string(tag(0x0008, 0x0016), Vr::Ui, "1.2.840.10008.5.1.4.1.1.2");
    assert!(matches!(
        DicomFile::new(set, EXPLICIT_LE),
        Err(Error::MissingTag(tag)) if tag == common::tag(0x0008, 0x0018)
    ));
}

#[test]
fn part10_written_bytes_round_trip() {
    let file = DicomFile::new(sample_dataset(), EXPLICIT_LE).unwrap();
    let bytes = file.write().unwrap();
    let decoded = DicomFile::read(&bytes).unwrap();
    assert_eq!(decoded.transfer_syntax().unwrap(), EXPLICIT_LE);
    assert_eq!(
        decoded.dataset.string(tag(0x0010, 0x0010)).unwrap(),
        "Doe^John"
    );
    // Writing again is a fixed point.
    assert_eq!(decoded.write().unwrap(), bytes);
}

#[test]
fn written_meta_is_explicit_little_endian() {
    let file = DicomFile::new(sample_dataset(), EXPLICIT_LE).unwrap();
    let bytes = file.write().unwrap();
    assert_eq!(&bytes[132..136], &tag_le(tag(0x0002, 0x0001)));
    assert_eq!(&bytes[136..138], b"OB");
    assert_eq!(&bytes[138..140], &[0x00, 0x00]);
    assert_eq!(&bytes[140..144], &[0x02, 0x00, 0x00, 0x00]);
    assert_eq!(&bytes[144..146], &[0x00, 0x01]);
}

#[test]
fn written_body_uses_the_declared_syntax() {
    let file = DicomFile::new(sample_dataset(), IMPLICIT_LE).unwrap();
    let bytes = file.write().unwrap();
    let expected = element_implicit(tag(0x0010, 0x0010), b"Doe^John");
    assert!(bytes
        .windows(expected.len())
        .any(|window| window == expected));
    let decoded = DicomFile::read(&bytes).unwrap();
    assert_eq!(
        decoded.dataset.string(tag(0x0010, 0x0010)).unwrap(),
        "Doe^John"
    );
}

#[test]
fn preamble_is_zeroed_on_write() {
    let body = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    let mut input = part10(&meta_transfer_syntax(EXPLICIT_LE), &body);
    input[0..128].fill(0xAB);

    let file = DicomFile::read(&input).unwrap();
    let written = file.write().unwrap();
    assert_eq!(&written[0..128], &[0u8; 128]);
    assert_eq!(&written[128..132], b"DICM");
}

#[test]
fn file_meta_group_length_is_not_written() {
    let mut meta = element_le("UL", tag(0x0002, 0x0000), &12u32.to_le_bytes());
    meta.extend(meta_transfer_syntax(EXPLICIT_LE));
    let file = DicomFile::read(&part10(&meta, &[])).unwrap();
    let written = file.write().unwrap();
    assert!(!written
        .windows(4)
        .any(|window| window == tag_le(tag(0x0002, 0x0000))));
}

#[test]
fn files_read_from_disk_round_trip() {
    let file = DicomFile::new(sample_dataset(), EXPLICIT_LE).unwrap();
    let mut path = std::env::temp_dir();
    path.push(format!("dcm4rs-{}.dcm", std::process::id()));

    std::fs::write(&path, file.write().unwrap()).unwrap();
    let decoded = DicomFile::read_file(&path).unwrap();
    assert_eq!(decoded.transfer_syntax().unwrap(), EXPLICIT_LE);
    assert_eq!(
        decoded.dataset.string(tag(0x0010, 0x0010)).unwrap(),
        "Doe^John"
    );
    std::fs::remove_file(&path).unwrap();
}

/// Mirrors pydicom's `test_write_empty_AT`: `AT` uses the short length form, so an empty value
/// is an eight-byte element.
#[test]
fn empty_at_is_written_with_the_short_form() {
    let mut set = DataSet::new();
    set.set(tag(0x0028, 0x0009), Vr::At, Value::Empty);
    let expected = element_le("AT", tag(0x0028, 0x0009), &[]);
    assert_eq!(expected.len(), 8);
    assert_eq!(set.write(EXPLICIT_LE).unwrap(), expected);
}

/// Mirrors pydicom's `test_write_empty_LO` and `test_write_OB_odd`.
#[test]
fn empty_text_and_binary_values_write_zero_length() {
    let mut set = DataSet::new();
    set.set(tag(0x0008, 0x0070), Vr::Lo, Value::Empty);
    assert_eq!(
        set.write(EXPLICIT_LE).unwrap(),
        element_le("LO", tag(0x0008, 0x0070), &[])
    );

    let mut set = DataSet::new();
    set.set(tag(0x7FE0, 0x0010), Vr::Ob, Value::Empty);
    assert_eq!(
        set.write(EXPLICIT_LE).unwrap(),
        element_le("OB", tag(0x7FE0, 0x0010), &[])
    );
}

/// Mirrors pydicom's `test_write_ascii_vr_with_padding`, `test_write_multi_DA`,
/// `test_write_multi_TM` and `test_write_multi_DT`.
#[test]
fn text_values_are_padded_like_pydicom() {
    // Single values: AE pads with a space, CS too.
    let mut set = DataSet::new();
    set.set_string(tag(0x0008, 0x0054), Vr::Ae, "CONQUESTSRV");
    assert_eq!(
        set.write(EXPLICIT_LE).unwrap(),
        element_le("AE", tag(0x0008, 0x0054), b"CONQUESTSRV ")
    );

    let mut set = DataSet::new();
    set.set_string(tag(0x0008, 0x0060), Vr::Cs, "REG");
    assert_eq!(
        set.write(EXPLICIT_LE).unwrap(),
        element_le("CS", tag(0x0008, 0x0060), b"REG ")
    );

    // Multiple values keep their backslashes and only the whole value is padded.
    let mut set = DataSet::new();
    set.set_string(tag(0x0014, 0x407E), Vr::Da, "20100101\\20101231");
    assert_eq!(
        set.write(EXPLICIT_LE).unwrap(),
        element_le("DA", tag(0x0014, 0x407E), b"20100101\\20101231 ")
    );

    let mut set = DataSet::new();
    set.set_string(tag(0x0014, 0x407C), Vr::Tm, "082500\\092655");
    assert_eq!(
        set.write(EXPLICIT_LE).unwrap(),
        element_le("TM", tag(0x0014, 0x407C), b"082500\\092655 ")
    );

    let mut set = DataSet::new();
    set.set_string(
        tag(0x0040, 0xA13A),
        Vr::Dt,
        "20120820120804\\20130901111111",
    );
    let bytes = set.write(EXPLICIT_LE).unwrap();
    assert_eq!(
        bytes,
        element_le(
            "DT",
            tag(0x0040, 0xA13A),
            b"20120820120804\\20130901111111 "
        )
    );
    let decoded = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    assert_eq!(
        decoded.strings(tag(0x0040, 0xA13A)).unwrap(),
        vec!["20120820120804", "20130901111111"]
    );
}

/// Mirrors pydicom's `test_implementation_class_uid_length` and
/// `test_implementation_version_name_length`.
#[test]
fn implementation_identifiers_fit_their_vr() {
    let file = DicomFile::new(sample_dataset(), EXPLICIT_LE).unwrap();
    let class_uid = file.meta.string(tag(0x0002, 0x0012)).unwrap();
    assert!(class_uid.len() <= 64, "{class_uid}");

    let version = file.meta.string(tag(0x0002, 0x0013)).unwrap();
    assert!(version.len() <= 16, "{version}");
}

/// Mirrors pydicom's `test_write_ffff_ffff`: the last possible tag survives a write and read.
#[test]
fn ffffffff_tag_writes_and_reads() {
    let mut set = DataSet::new();
    set.set_string(Tag::new(0xFFFF, 0xFFFF), Vr::Lo, "123456");
    let bytes = set.write(IMPLICIT_LE).unwrap();
    assert_eq!(bytes, element_implicit(Tag::new(0xFFFF, 0xFFFF), b"123456"));

    let decoded = DataSet::read(&bytes, IMPLICIT_LE).unwrap();
    let element = decoded.get(Tag::new(0xFFFF, 0xFFFF)).unwrap();
    assert_eq!(element.vr, Vr::Un);
    assert_eq!(element.raw_value(), b"123456");
}

/// Mirrors pydicom's `test_bad_elements`: the file meta may only hold group 2 elements, so
/// writing anything else is rejected. On read the group-2 scan stops at the first other group,
/// which hands the element to the data set.
#[test]
fn non_group_2_meta_elements_are_rejected_on_write() {
    let mut file = DicomFile::new(sample_dataset(), EXPLICIT_LE).unwrap();
    file.meta
        .set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    assert_eq!(file.write(), Err(Error::InvalidMeta(tag(0x0010, 0x0010))));

    let mut meta = meta_transfer_syntax(EXPLICIT_LE);
    meta.extend(element_le("PN", tag(0x0010, 0x0010), b"Doe^John"));
    let decoded = DicomFile::read(&part10(&meta, &[])).unwrap();
    assert!(!decoded.meta.contains(tag(0x0010, 0x0010)));
    assert_eq!(
        decoded.dataset.string(tag(0x0010, 0x0010)).unwrap(),
        "Doe^John"
    );
}

/// Mirrors pydicom's `TestWriteUndefinedLengthPixelData` for correctly sized fragments.
#[test]
fn encapsulated_pixel_data_round_trips_through_a_file() {
    let syntax = TransferSyntax::Encapsulated("1.2.840.10008.1.2.4.50");
    let mut set = sample_dataset();
    set.set(
        tag(0x7FE0, 0x0010),
        Vr::Ob,
        Value::Fragments(vec![Vec::new(), vec![1, 2, 3, 4]]),
    );

    let file = DicomFile::new(set, syntax).unwrap();
    let bytes = file.write().unwrap();
    let decoded = DicomFile::read(&bytes).unwrap();
    match &decoded.dataset.get(tag(0x7FE0, 0x0010)).unwrap().value {
        Value::Fragments(fragments) => {
            assert_eq!(fragments.len(), 2);
            assert!(
                fragments[0].is_empty(),
                "the first item is the offset table"
            );
            assert_eq!(fragments[1], vec![1, 2, 3, 4]);
        }
        other => panic!("expected fragments, got {other:?}"),
    }
}

#[test]
fn missing_files_report_io_errors() {
    let mut path = std::env::temp_dir();
    path.push("dcm4rs-does-not-exist.dcm");
    assert!(matches!(DicomFile::read_file(&path), Err(Error::Io(_))));
}
