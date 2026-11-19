//! Reading behaviour, mirroring pydicom's `tests/test_filereader.py`.
//!
//! Covered: explicit and implicit VR headers, both length forms, big endian, defined and
//! undefined length sequences and items, encapsulated pixel data, Part 10 structure and
//! malformed input.
//!
//! Divergences from pydicom asserted below and listed in the README: an implicit VR group
//! length element becomes `UN` instead of `UL` (the dictionary has no such entries), an
//! undefined length on `OB`/`OW` is read as fragments wherever it appears (pydicom only treats
//! Pixel Data that way), and there is no `force`/deferred read.

mod common;

use dcm4rs::{DataSet, Element, Endian, Error, ReadOptions, Tag, TransferSyntax, Value, Vr};

use common::{
    delimiter_le, element_be, element_implicit, element_le, element_le_undefined, item_be, item_le,
    item_undefined_le, meta_transfer_syntax, meta_version, part10, tag, tag_le, ITEM,
    ITEM_DELIMITER, SEQUENCE_DELIMITER, UNDEFINED,
};

const EXPLICIT_LE: TransferSyntax = TransferSyntax::ExplicitVrLittleEndian;
const IMPLICIT_LE: TransferSyntax = TransferSyntax::ImplicitVrLittleEndian;
const EXPLICIT_BE: TransferSyntax = TransferSyntax::ExplicitVrBigEndian;

fn read(bytes: &[u8], syntax: TransferSyntax) -> DataSet {
    DataSet::read(bytes, syntax).expect("fixture should decode")
}

#[test]
fn explicit_short_and_long_length_forms() {
    let us = element_le("US", tag(0x0028, 0x0010), &[0x00, 0x02]);
    // Short form: two-byte length right after the VR.
    assert_eq!(&us[4..6], b"US");
    assert_eq!(&us[6..8], &[0x02, 0x00]);

    let ob = element_le("OB", tag(0x7FE0, 0x0010), &[1, 2, 3, 4]);
    // Long form: two reserved bytes, then a four-byte length.
    assert_eq!(&ob[6..8], &[0x00, 0x00]);
    assert_eq!(&ob[8..12], &[0x04, 0x00, 0x00, 0x00]);

    let set = read(&us, EXPLICIT_LE);
    assert_eq!(set.u16(tag(0x0028, 0x0010)).unwrap(), 512);

    let set = read(&ob, EXPLICIT_LE);
    assert_eq!(
        set.get(tag(0x7FE0, 0x0010)).unwrap().raw_value(),
        &[1, 2, 3, 4]
    );
}

#[test]
fn explicit_zero_length_elements() {
    for vr in ["PN", "US", "OB", "SQ", "UT", "AT"] {
        let bytes = element_le(vr, tag(0x0010, 0x0010), &[]);
        let set = read(&bytes, EXPLICIT_LE);
        let element = set.get(tag(0x0010, 0x0010)).unwrap();
        assert_eq!(element.vr.as_str(), vr);
        assert!(element.is_empty(), "{vr}");
    }
}

#[test]
fn explicit_odd_length_text_is_trimmed_on_access() {
    let bytes = element_le("PN", tag(0x0010, 0x0010), b"AB ");
    let set = read(&bytes, EXPLICIT_LE);
    let element = set.get(tag(0x0010, 0x0010)).unwrap();
    assert_eq!(element.raw_value().len(), 3);
    assert_eq!(element.string().unwrap(), "AB");
}

#[test]
fn explicit_value_past_the_input_is_an_eof() {
    // Header says four bytes of value, none follow.
    let mut bytes = tag_le(tag(0x0010, 0x0010)).to_vec();
    bytes.extend_from_slice(b"PN");
    bytes.extend_from_slice(&[0x04, 0x00]);
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::UnexpectedEof { offset: 8 })
    ));
}

#[test]
fn explicit_unknown_vr_code_is_rejected() {
    let mut bytes = tag_le(tag(0x0010, 0x0010)).to_vec();
    bytes.extend_from_slice(b"ZZ");
    bytes.extend_from_slice(&[0x00, 0x00]);
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::InvalidData {
            offset: 4,
            reason: "unknown VR code"
        })
    ));
}

#[test]
fn explicit_undefined_length_on_a_non_sequence_is_rejected() {
    // UT uses the long length form, so the undefined length is legal to encode but not here.
    let mut bytes = tag_le(tag(0x0010, 0x0010)).to_vec();
    bytes.extend_from_slice(b"UT");
    bytes.extend_from_slice(&[0x00, 0x00]);
    bytes.extend_from_slice(&UNDEFINED.to_le_bytes());
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::InvalidData {
            offset: 12,
            reason: "undefined length on a non-sequence element"
        })
    ));
}

/// PS3.5 does not allow an undefined length on `UT`, but some writers use it and end the value
/// with an item delimiter. pydicom reads to that delimiter and keeps what it finds;
/// `ReadOptions::lenient_undefined_length` asks for the same, while the default still refuses.
#[test]
fn lenient_reading_keeps_the_bytes_before_the_delimiter() {
    let mut bytes = tag_le(tag(0x0010, 0x0010)).to_vec();
    bytes.extend_from_slice(b"UT");
    bytes.extend_from_slice(&[0x00, 0x00]);
    bytes.extend_from_slice(&UNDEFINED.to_le_bytes());
    bytes.extend_from_slice(b"Doe^John");
    bytes.extend_from_slice(&delimiter_le(ITEM_DELIMITER));

    let lenient = ReadOptions {
        lenient_undefined_length: true,
        ..ReadOptions::default()
    };
    let set = DataSet::read_with_options(&bytes, EXPLICIT_LE, lenient).expect("lenient read");
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Doe^John");

    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::InvalidData { .. })
    ));
}

#[test]
fn duplicate_elements_keep_the_first_and_are_reported() {
    let mut bytes = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    bytes.extend_from_slice(&element_le("PN", tag(0x0010, 0x0010), b"Roe^Jane"));
    let set = read(&bytes, EXPLICIT_LE);
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Doe^John");
    assert_eq!(set.duplicates(), &[tag(0x0010, 0x0010)]);
}

/// The parser walks items with the same reader, so a duplicate inside an item also shows up on
/// the data set that contains it.
#[test]
fn duplicates_inside_an_item_are_visible_on_the_parent() {
    let mut stream = item_undefined_le();
    stream.extend_from_slice(&element_le("PN", tag(0x0010, 0x0010), b"Doe^John"));
    stream.extend_from_slice(&element_le("PN", tag(0x0010, 0x0010), b"Roe^Jane"));
    stream.extend_from_slice(&delimiter_le(ITEM_DELIMITER));
    stream.extend_from_slice(&delimiter_le(SEQUENCE_DELIMITER));
    let bytes = element_le_undefined("SQ", tag(0x0040, 0x0275), &stream);
    let set = read(&bytes, EXPLICIT_LE);
    assert_eq!(set.duplicates(), &[tag(0x0010, 0x0010)]);
}

/// A defined-length item bounds its elements, so a value running past it is malformed. Without
/// the check the value silently consumes the bytes that follow the item.
#[test]
fn value_running_past_its_item_is_rejected() {
    // The item claims 2 bytes but holds a 16-byte PN element; the sequence length covers the
    // whole stream, so only the item bound can catch it.
    let inner = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    let mut items = item_le(2);
    items.extend_from_slice(&inner);
    let bytes = element_le("SQ", tag(0x0040, 0x0275), &items);
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::InvalidData {
            offset: 20,
            reason: "value runs past the end of its container"
        })
    ));

    // The same item with the correct length reads.
    let mut items = item_le(inner.len() as u32);
    items.extend_from_slice(&inner);
    let bytes = element_le("SQ", tag(0x0040, 0x0275), &items);
    let set = read(&bytes, EXPLICIT_LE);
    assert_eq!(
        set.sequence(tag(0x0040, 0x0275)).unwrap()[0]
            .string(tag(0x0010, 0x0010))
            .unwrap(),
        "Doe^John"
    );
}

#[test]
fn truncated_tag_is_an_eof() {
    // The group reads, the element number does not.
    assert!(matches!(
        DataSet::read(&[0x10, 0x00], EXPLICIT_LE),
        Err(Error::UnexpectedEof { offset: 2 })
    ));
}

#[test]
fn empty_input_reads_as_an_empty_data_set() {
    let set = read(&[], EXPLICIT_LE);
    assert!(set.is_empty());
    let set = read(&[], IMPLICIT_LE);
    assert!(set.is_empty());
}

#[test]
fn implicit_fixed_vrs_come_from_the_dictionary() {
    let mut bytes = element_implicit(tag(0x0010, 0x0010), b"Doe^John");
    bytes.extend(element_implicit(tag(0x0028, 0x0010), &[0x00, 0x02]));
    bytes.extend(element_implicit(tag(0x0008, 0x0008), b"ORIGINAL"));

    let set = read(&bytes, IMPLICIT_LE);
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Doe^John");
    assert_eq!(set.u16(tag(0x0028, 0x0010)).unwrap(), 512);
    assert_eq!(set.string(tag(0x0008, 0x0008)).unwrap(), "ORIGINAL");
    assert_eq!(set.get(tag(0x0010, 0x0010)).unwrap().vr, Vr::Pn);
    assert_eq!(set.get(tag(0x0028, 0x0010)).unwrap().vr, Vr::Us);
}

#[test]
fn implicit_private_tags_follow_the_private_vr_rules() {
    let mut bytes = element_implicit(tag(0x0011, 0x0010), b"MY CREATOR");
    bytes.extend(element_implicit(tag(0x0011, 0x1001), &[1, 2, 3, 4]));
    let set = read(&bytes, IMPLICIT_LE);

    // pydicom's `hooks._private_vr_for_tag` (pydicom/hooks.py:139-152): a private creator is
    // `LO`; a private data element is `UN` unless a private dictionary supplies its VR (the
    // private dictionary is not implemented here).
    let creator = set.get(tag(0x0011, 0x0010)).unwrap();
    assert_eq!(creator.vr, Vr::Lo);
    assert_eq!(creator.string().unwrap(), "MY CREATOR");
    let data = set.get(tag(0x0011, 0x1001)).unwrap();
    assert_eq!(data.vr, Vr::Un);
    assert_eq!(data.raw_value(), &[1, 2, 3, 4]);
}

#[test]
fn implicit_group_length_is_ul() {
    // The registry has no group length entries, so pydicom special-cases `element == 0` as `UL`
    // (pydicom/hooks.py:191-193); the reader does the same.
    let bytes = element_implicit(tag(0x0008, 0x0000), &[0x10, 0x00, 0x00, 0x00]);
    let set = read(&bytes, IMPLICIT_LE);
    assert_eq!(set.get(tag(0x0008, 0x0000)).unwrap().vr, Vr::Ul);
    assert_eq!(
        set.get(tag(0x0008, 0x0000))
            .unwrap()
            .u32s(Endian::Little)
            .unwrap(),
        vec![16]
    );
}

#[test]
fn implicit_pixel_data_is_ow() {
    let bytes = element_implicit(tag(0x7FE0, 0x0010), &[1, 2, 3, 4, 5, 6]);
    let set = read(&bytes, IMPLICIT_LE);
    assert_eq!(set.get(tag(0x7FE0, 0x0010)).unwrap().vr, Vr::Ow);
}

#[test]
fn implicit_ambiguous_us_or_ss_defaults_to_us() {
    for tag in [tag(0x0018, 0x9810), tag(0x0028, 0x0120)] {
        let bytes = element_implicit(tag, &[0xFF, 0xFF]);
        let set = read(&bytes, IMPLICIT_LE);
        let element = set.get(tag).unwrap();
        assert_eq!(element.vr, Vr::Us, "{tag}");
        // The bytes are untouched, so a signed reading is still available.
        assert_eq!(element.u16s(Endian::Little).unwrap(), vec![65535]);
        assert_eq!(element.i16s(Endian::Little).unwrap(), vec![-1]);
    }
}

#[test]
fn implicit_sequence_structure_comes_from_the_dictionary() {
    let item_body = element_implicit(tag(0x0008, 0x0100), b"CODE");
    let mut sequence = item_le(item_body.len() as u32);
    sequence.extend_from_slice(&item_body);
    let bytes = element_implicit(tag(0x0040, 0x0275), &sequence);

    let set = read(&bytes, IMPLICIT_LE);
    let element = set.get(tag(0x0040, 0x0275)).unwrap();
    assert!(element.is_sequence());
    assert_eq!(element.vr, Vr::Sq);
    let items = set.sequence(tag(0x0040, 0x0275)).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].string(tag(0x0008, 0x0100)).unwrap(), "CODE");
}

#[test]
fn big_endian_numbers_and_tags() {
    let bytes = element_be("US", tag(0x0028, 0x0010), &[0x02, 0x00]);
    let set = read(&bytes, EXPLICIT_BE);
    assert_eq!(set.endian(), Endian::Big);
    assert_eq!(set.u16(tag(0x0028, 0x0010)).unwrap(), 512);
}

#[test]
fn big_endian_sequence_items_inherit_the_byte_order() {
    let item_body = element_be("SH", tag(0x0008, 0x0100), b"CODE");
    let mut items = item_be(item_body.len() as u32);
    items.extend_from_slice(&item_body);
    let bytes = element_be("SQ", tag(0x0040, 0x0275), &items);

    let set = read(&bytes, EXPLICIT_BE);
    let items = set.sequence(tag(0x0040, 0x0275)).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].endian(), Endian::Big);
    assert_eq!(items[0].string(tag(0x0008, 0x0100)).unwrap(), "CODE");
}

#[test]
fn undefined_length_sequence_with_mixed_items() {
    let first_body = element_le("SH", tag(0x0008, 0x0100), b"ONE");
    let second_body = element_le("SH", tag(0x0008, 0x0100), b"TWO");

    let mut stream = Vec::new();
    stream.extend(item_le(first_body.len() as u32));
    stream.extend_from_slice(&first_body);
    stream.extend(item_undefined_le());
    stream.extend_from_slice(&second_body);
    stream.extend(delimiter_le(ITEM_DELIMITER));
    stream.extend(delimiter_le(SEQUENCE_DELIMITER));

    let bytes = element_le_undefined("SQ", tag(0x0040, 0x0275), &stream);
    let set = read(&bytes, EXPLICIT_LE);
    let items = set.sequence(tag(0x0040, 0x0275)).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].string(tag(0x0008, 0x0100)).unwrap(), "ONE");
    assert_eq!(items[1].string(tag(0x0008, 0x0100)).unwrap(), "TWO");
}

#[test]
fn empty_items_and_empty_sequences() {
    // A defined-length sequence holding one zero-length item.
    let bytes = element_le("SQ", tag(0x0040, 0x0275), &item_le(0));
    let set = read(&bytes, EXPLICIT_LE);
    let items = set.sequence(tag(0x0040, 0x0275)).unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].is_empty());

    // An undefined-length sequence with only the delimiter.
    let bytes = element_le_undefined("SQ", tag(0x0040, 0x0275), &delimiter_le(SEQUENCE_DELIMITER));
    let set = read(&bytes, EXPLICIT_LE);
    assert!(set.sequence(tag(0x0040, 0x0275)).unwrap().is_empty());
}

#[test]
fn nested_sequences_three_levels() {
    let leaf = element_le("SH", tag(0x0008, 0x0100), b"CODE");
    let mut leaf_item = item_le(leaf.len() as u32);
    leaf_item.extend_from_slice(&leaf);

    let middle = element_le("SQ", tag(0x0040, 0xA730), &leaf_item);
    let mut middle_item = item_le(middle.len() as u32);
    middle_item.extend_from_slice(&middle);

    let bytes = element_le("SQ", tag(0x0040, 0x0275), &middle_item);
    let set = read(&bytes, EXPLICIT_LE);
    let first = set.sequence(tag(0x0040, 0x0275)).unwrap();
    let second = first[0].sequence(tag(0x0040, 0xA730)).unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].string(tag(0x0008, 0x0100)).unwrap(), "CODE");
}

#[test]
fn missing_item_delimiter_is_reported() {
    let body = element_le("SH", tag(0x0008, 0x0100), b"ONE");
    let mut stream = item_undefined_le();
    stream.extend_from_slice(&body);
    let bytes = element_le_undefined("SQ", tag(0x0040, 0x0275), &stream);
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::UnexpectedEof { .. })
    ));
}

#[test]
fn missing_sequence_delimiter_is_reported() {
    let body = element_le("SH", tag(0x0008, 0x0100), b"ONE");
    let mut stream = item_le(body.len() as u32);
    stream.extend_from_slice(&body);
    let bytes = element_le_undefined("SQ", tag(0x0040, 0x0275), &stream);
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::UnexpectedEof { .. })
    ));
}

#[test]
fn delimiters_are_rejected_at_the_top_level() {
    for tag in [ITEM, ITEM_DELIMITER, SEQUENCE_DELIMITER] {
        let mut bytes = tag_le(tag).to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert!(
            matches!(
                DataSet::read(&bytes, EXPLICIT_LE),
                Err(Error::InvalidData {
                    offset: 0,
                    reason: "unexpected item or delimiter tag"
                })
            ),
            "{tag}"
        );
    }
}

#[test]
fn defined_sequence_rejects_non_item_tags() {
    let body = element_le("US", tag(0x0028, 0x0010), &[1, 0]);
    let bytes = element_le("SQ", tag(0x0040, 0x0275), &body);
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::InvalidData {
            reason: "expected an item tag inside a sequence",
            ..
        })
    ));
}

#[test]
fn item_running_past_the_sequence_is_rejected() {
    let bytes = element_le("SQ", tag(0x0040, 0x0275), &item_le(8));
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::InvalidData {
            reason: "item runs past the end of the sequence",
            ..
        })
    ));
}

#[test]
fn encapsulated_fragments_are_read() {
    let mut stream = Vec::new();
    stream.extend(item_le(0)); // empty Basic Offset Table
    stream.extend(item_le(4));
    stream.extend_from_slice(&[1, 2, 3, 4]);
    stream.extend(delimiter_le(SEQUENCE_DELIMITER));

    let bytes = element_le_undefined("OB", tag(0x7FE0, 0x0010), &stream);
    let set = read(&bytes, EXPLICIT_LE);
    let element = set.get(tag(0x7FE0, 0x0010)).unwrap();
    match &element.value {
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
fn non_empty_offset_table_is_kept() {
    let mut stream = Vec::new();
    stream.extend(item_le(4));
    stream.extend_from_slice(&[0, 0, 0, 0]);
    stream.extend(delimiter_le(SEQUENCE_DELIMITER));

    let bytes = element_le_undefined("OW", tag(0x7FE0, 0x0010), &stream);
    let set = read(&bytes, EXPLICIT_LE);
    let element = set.get(tag(0x7FE0, 0x0010)).unwrap();
    match &element.value {
        Value::Fragments(fragments) => {
            assert_eq!(fragments[0], vec![0, 0, 0, 0]);
        }
        other => panic!("expected fragments, got {other:?}"),
    }
}

#[test]
fn fragment_items_require_a_defined_length() {
    let mut stream = item_undefined_le();
    stream.extend(delimiter_le(SEQUENCE_DELIMITER));
    let bytes = element_le_undefined("OB", tag(0x7FE0, 0x0010), &stream);
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::InvalidData {
            reason: "fragment items must have a defined length",
            ..
        })
    ));
}

#[test]
fn fragment_stream_rejects_non_item_tags() {
    let body = element_le("US", tag(0x0028, 0x0010), &[1, 0]);
    let bytes = element_le_undefined("OB", tag(0x7FE0, 0x0010), &body);
    assert!(matches!(
        DataSet::read(&bytes, EXPLICIT_LE),
        Err(Error::InvalidData {
            reason: "expected a fragment item or the sequence delimiter",
            ..
        })
    ));
}

#[test]
fn undefined_length_ob_outside_pixel_data_is_treated_as_fragments() {
    // Divergence from pydicom: any undefined-length OB/OW is parsed as fragments, not only
    // Pixel Data. The structure survives a rewrite unchanged.
    let mut stream = item_le(2);
    stream.extend_from_slice(&[1, 2]);
    stream.extend(delimiter_le(SEQUENCE_DELIMITER));
    let bytes = element_le_undefined("OB", tag(0x0011, 0x1001), &stream);
    let set = read(&bytes, EXPLICIT_LE);
    assert!(set.get(tag(0x0011, 0x1001)).unwrap().value.is_fragments());
}

/// Mirrors pydicom's `test_duplicate_elem_non_SQ_explicit_VR` and
/// `test_duplicate_elem_non_SQ_implicit_VR`: the later instance is ignored, so the first one wins
/// (pydicom/filereader.py:441-444, 517-532). pydicom also logs a warning; there is no logging
/// here.
#[test]
fn duplicate_tags_keep_the_first_occurrence() {
    let mut bytes = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    bytes.extend(element_le("PN", tag(0x0010, 0x0010), b"Roe^Jane"));
    let set = read(&bytes, EXPLICIT_LE);
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Doe^John");
    assert_eq!(set.len(), 1);

    // The same rule under implicit VR encoding.
    let mut bytes = element_implicit(tag(0x0010, 0x0010), b"Doe^John");
    bytes.extend(element_implicit(tag(0x0010, 0x0010), b"Roe^Jane"));
    let set = read(&bytes, IMPLICIT_LE);
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Doe^John");
    assert_eq!(set.len(), 1);
}

#[test]
fn large_values_are_read_whole() {
    let body = vec![0xA5u8; 70_000];
    let bytes = element_le("OB", tag(0x7FE0, 0x0010), &body);
    let set = read(&bytes, EXPLICIT_LE);
    assert_eq!(
        set.get(tag(0x7FE0, 0x0010)).unwrap().raw_value().len(),
        70_000
    );
}

#[test]
fn part10_meta_and_dataset_are_split() {
    let body = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    let bytes = part10(&meta_transfer_syntax(EXPLICIT_LE), &body);
    let file = dcm4rs::DicomFile::read(&bytes).unwrap();
    assert_eq!(file.transfer_syntax().unwrap(), EXPLICIT_LE);
    assert_eq!(
        file.meta.string(tag(0x0002, 0x0010)).unwrap(),
        EXPLICIT_LE.uid()
    );
    assert_eq!(
        file.dataset.string(tag(0x0010, 0x0010)).unwrap(),
        "Doe^John"
    );
}

#[test]
fn part10_dispatches_on_the_declared_syntax() {
    let implicit = part10(
        &meta_transfer_syntax(IMPLICIT_LE),
        &element_implicit(tag(0x0010, 0x0010), b"Doe^John"),
    );
    let file = dcm4rs::DicomFile::read(&implicit).unwrap();
    assert_eq!(
        file.dataset.string(tag(0x0010, 0x0010)).unwrap(),
        "Doe^John"
    );
    assert_eq!(file.dataset.endian(), Endian::Little);

    let big = part10(
        &meta_transfer_syntax(EXPLICIT_BE),
        &element_be("US", tag(0x0028, 0x0010), &[0x02, 0x00]),
    );
    let file = dcm4rs::DicomFile::read(&big).unwrap();
    assert_eq!(file.dataset.endian(), Endian::Big);
    assert_eq!(file.dataset.u16(tag(0x0028, 0x0010)).unwrap(), 512);
}

#[test]
fn part10_encapsulated_body() {
    let mut stream = item_le(0);
    stream.extend(item_le(2));
    stream.extend_from_slice(&[1, 2]);
    stream.extend(delimiter_le(SEQUENCE_DELIMITER));
    let body = element_le_undefined("OB", tag(0x7FE0, 0x0010), &stream);

    let bytes = part10(
        &meta_transfer_syntax(TransferSyntax::Encapsulated("1.2.840.10008.1.2.4.50")),
        &body,
    );
    let file = dcm4rs::DicomFile::read(&bytes).unwrap();
    assert!(file
        .dataset
        .get(tag(0x7FE0, 0x0010))
        .unwrap()
        .value
        .is_fragments());
}

#[test]
fn part10_meta_group_length_element_is_read() {
    let mut meta = element_le("UL", tag(0x0002, 0x0000), &12u32.to_le_bytes());
    meta.extend(meta_transfer_syntax(EXPLICIT_LE));
    let file = dcm4rs::DicomFile::read(&part10(&meta, &[])).unwrap();
    assert!(file.meta.contains(tag(0x0002, 0x0000)));
    assert_eq!(
        file.meta
            .get(tag(0x0002, 0x0000))
            .unwrap()
            .u32s(Endian::Little)
            .unwrap(),
        vec![12]
    );
}

/// Mirrors pydicom's `test_known_tags_with_UN_VR`: a `UN` value on a known public tag is an
/// encoding error and the dictionary VR takes over (`config.replace_un_with_known_vr`, default
/// on; pydicom/hooks.py:201-210).
#[test]
fn explicit_un_on_a_known_tag_uses_the_dictionary_vr() {
    let bytes = element_le("UN", tag(0x0010, 0x0010), b"Doe^John");
    let set = read(&bytes, EXPLICIT_LE);
    let element = set.get(tag(0x0010, 0x0010)).unwrap();
    assert_eq!(element.vr, Vr::Pn);
    assert_eq!(element.string().unwrap(), "Doe^John");

    // pydicom's `test_tag_with_long_value_UN_VR`: values that need a 32-bit length stay UN.
    let body = vec![b'A'; 0xFFFF];
    let bytes = element_le("UN", tag(0x3004, 0x0058), &body);
    let set = read(&bytes, EXPLICIT_LE);
    assert_eq!(set.get(tag(0x3004, 0x0058)).unwrap().vr, Vr::Un);

    // Unknown tags keep UN in both libraries.
    let bytes = element_le("UN", tag(0x1111, 0x1010), &[1, 2]);
    let set = read(&bytes, EXPLICIT_LE);
    assert_eq!(set.get(tag(0x1111, 0x1010)).unwrap().vr, Vr::Un);
}

/// A defined-length `UN` keeps its bytes even when they look like an item stream: pydicom only
/// infers `SQ` for undefined length (`infer_sq_for_un_vr`, pydicom/filereader.py:284-287).
#[test]
fn defined_length_un_holding_item_bytes_stays_un() {
    let item_body = element_le("SH", tag(0x0008, 0x0100), b"CODE");
    let mut items = item_le(item_body.len() as u32);
    items.extend_from_slice(&item_body);
    let bytes = element_le("UN", tag(0x4453, 0x100C), &items);

    let set = read(&bytes, EXPLICIT_LE);
    let element = set.get(tag(0x4453, 0x100C)).unwrap();
    assert_eq!(element.vr, Vr::Un);
    assert!(!element.is_sequence());
    assert_eq!(element.raw_value(), items.as_slice());
}

/// Mirrors pydicom's `test_un_sequence`: an undefined-length `UN` is read as `SQ`
/// (`infer_sq_for_un_vr`, on by default; pydicom/filereader.py:284-287).
#[test]
fn undefined_length_un_is_read_as_a_sequence() {
    let item_body = element_le("SH", tag(0x0008, 0x0100), b"CODE");
    let mut stream = item_le(item_body.len() as u32);
    stream.extend_from_slice(&item_body);
    stream.extend(delimiter_le(SEQUENCE_DELIMITER));
    let bytes = element_le_undefined("UN", tag(0x4453, 0x100C), &stream);

    let set = read(&bytes, EXPLICIT_LE);
    let element = set.get(tag(0x4453, 0x100C)).unwrap();
    assert_eq!(element.vr, Vr::Sq);
    assert_eq!(
        element.as_sequence().unwrap()[0]
            .string(tag(0x0008, 0x0100))
            .unwrap(),
        "CODE"
    );
}

/// Mirrors pydicom's `test_no_meta_group_length`: the file meta need not carry `(0002,0000)`.
#[test]
fn part10_meta_without_group_length_reads() {
    let body = element_le("DA", tag(0x0008, 0x0012), b"20111130");
    let bytes = part10(&meta_transfer_syntax(EXPLICIT_LE), &body);
    let file = dcm4rs::DicomFile::read(&bytes).unwrap();
    assert!(!file.meta.contains(tag(0x0002, 0x0000)));
    assert_eq!(
        file.dataset.string(tag(0x0008, 0x0012)).unwrap(),
        "20111130"
    );
}

/// Mirrors pydicom's `test_group_length_wrong`: a wrong `(0002,0000)` value is ignored instead of
/// used to delimit the meta group.
#[test]
fn part10_meta_with_wrong_group_length_reads() {
    let mut meta = element_le("UL", tag(0x0002, 0x0000), &999u32.to_le_bytes());
    meta.extend(meta_transfer_syntax(EXPLICIT_LE));
    let body = element_le("DA", tag(0x0008, 0x0012), b"20111130");
    let bytes = part10(&meta, &body);
    let file = dcm4rs::DicomFile::read(&bytes).unwrap();
    assert_eq!(
        file.meta
            .get(tag(0x0002, 0x0000))
            .unwrap()
            .u32s(Endian::Little)
            .unwrap(),
        vec![999]
    );
    assert_eq!(
        file.dataset.string(tag(0x0008, 0x0012)).unwrap(),
        "20111130"
    );
}

/// Mirrors pydicom's `test_empty_pn`: an empty text value reads as `""`, not as "no value".
#[test]
fn zero_length_text_reads_as_empty_string() {
    let bytes = element_le("PN", tag(0x0010, 0x0010), &[]);
    let set = read(&bytes, EXPLICIT_LE);
    let element = set.get(tag(0x0010, 0x0010)).unwrap();
    assert!(element.is_empty());
    assert_eq!(element.string().unwrap(), "");
    assert!(element.string_values().unwrap().is_empty());
}

/// Mirrors pydicom's `test_no_transfer_syntax_in_meta`: with no `(0002,0010)` the file is read
/// with the default transfer syntax (Implicit VR Little Endian).
#[test]
fn part10_without_transfer_syntax_defaults_to_implicit_vr_le() {
    let body = element_implicit(tag(0x0010, 0x0010), b"Doe^John");
    let file = part10(&meta_version(), &body);
    let file = dcm4rs::DicomFile::read(&file).unwrap();
    assert_eq!(
        file.transfer_syntax().unwrap(),
        TransferSyntax::ImplicitVrLittleEndian
    );
    assert_eq!(
        file.dataset.string(tag(0x0010, 0x0010)).unwrap(),
        "Doe^John"
    );
}

#[test]
fn part10_with_unknown_transfer_syntax_is_rejected() {
    let mut meta = Vec::new();
    meta.extend(element_le("UI", tag(0x0002, 0x0010), b"1.2.3.4"));
    assert!(matches!(
        dcm4rs::DicomFile::read(&part10(&meta, &[])),
        Err(Error::UnsupportedTransferSyntax(uid)) if uid == "1.2.3.4"
    ));
}

#[test]
fn part10_deflated_syntax_is_read() {
    use std::io::Write;

    let body = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&body).unwrap();
    let deflated = encoder.finish().unwrap();
    let bytes = part10(
        &meta_transfer_syntax(TransferSyntax::DeflatedExplicitVrLittleEndian),
        &deflated,
    );
    let file = dcm4rs::DicomFile::read(&bytes).unwrap();
    assert_eq!(
        file.dataset.string(tag(0x0010, 0x0010)).unwrap(),
        "Doe^John"
    );
}

#[test]
fn part10_deflated_corrupt_stream_is_rejected() {
    let bytes = part10(
        &meta_transfer_syntax(TransferSyntax::DeflatedExplicitVrLittleEndian),
        &[0xFF, 0x00],
    );
    assert!(matches!(
        dcm4rs::DicomFile::read(&bytes),
        Err(Error::InvalidData {
            reason: "deflate: invalid compressed data",
            ..
        })
    ));
}

#[test]
fn part10_without_the_dicom_prefix_is_rejected() {
    assert!(matches!(
        dcm4rs::DicomFile::read(&[0u8; 100]),
        Err(Error::InvalidData {
            offset: 128,
            reason: "missing DICM prefix"
        })
    ));
    // A bare data set is not a Part 10 file.
    let bare = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    assert!(dcm4rs::DicomFile::read(&bare).is_err());
}

#[test]
fn truncated_meta_is_an_eof() {
    let mut bytes = vec![0u8; 128];
    bytes.extend_from_slice(b"DICM");
    bytes.extend(tag_le(tag(0x0002, 0x0001)));
    bytes.extend_from_slice(b"OB");
    bytes.extend_from_slice(&[0x00, 0x00]);
    bytes.extend_from_slice(&2u32.to_le_bytes());
    assert!(matches!(
        dcm4rs::DicomFile::read(&bytes),
        Err(Error::UnexpectedEof { .. })
    ));
}

#[test]
fn element_equality_round_trips_through_bytes() {
    let element = Element::new(tag(0x0010, 0x0010), Vr::Pn, Value::from_str("Doe^John"));
    let bytes = element_le("PN", tag(0x0010, 0x0010), b"Doe^John");
    let set = read(&bytes, EXPLICIT_LE);
    assert_eq!(set.get(tag(0x0010, 0x0010)).unwrap(), &element);
    assert_eq!(Tag::parse("0010,0010").unwrap(), tag(0x0010, 0x0010));
}
