//! Data set behaviour, mirroring pydicom's `tests/test_dataset.py`.
//!
//! Covered: element insertion, lookup, removal, ordering, keyword-driven access, equality,
//! endianness effects on numeric decoding, sequence nesting, private tags and group length
//! elements.
//!
//! Divergences from pydicom, asserted below and listed in the README:
//! `DataSet` equality includes the byte order, and there is no `__str__` dump, no
//! `walk`/`iterall` callback API and no `private_block` helper yet.

mod common;

use dcm4rs::{dictionary, DataSet, Endian, Error, Tag, Value, Vr};

use common::tag;

#[test]
fn set_get_remove_and_len() {
    let mut set = DataSet::new();
    assert!(set.is_empty());
    assert_eq!(set.len(), 0);

    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    assert_eq!(set.len(), 1);
    assert!(set.contains(tag(0x0010, 0x0010)));
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Doe^John");

    let removed = set.remove(tag(0x0010, 0x0010)).unwrap();
    assert_eq!(removed.tag, tag(0x0010, 0x0010));
    assert!(!set.contains(tag(0x0010, 0x0010)));
    assert!(set.is_empty());
}

#[test]
fn insert_replaces_and_returns_the_previous_element() {
    let mut set = DataSet::new();
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    let previous = set.set(tag(0x0010, 0x0010), Vr::Pn, Value::from_str("Roe^Jane"));
    assert_eq!(previous.unwrap().string().unwrap(), "Doe^John");
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Roe^Jane");
    assert_eq!(set.len(), 1);
}

#[test]
fn elements_are_stored_in_ascending_tag_order() {
    let mut set = DataSet::new();
    set.set_string(tag(0x0028, 0x0010), Vr::Us, "512"); // out of order on purpose
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    set.set_string(tag(0x0008, 0x0020), Vr::Da, "20260101");

    assert_eq!(
        set.tags().collect::<Vec<_>>(),
        vec![
            tag(0x0008, 0x0020),
            tag(0x0010, 0x0010),
            tag(0x0028, 0x0010)
        ]
    );
    assert_eq!(
        set.iter().map(|element| element.tag).collect::<Vec<_>>(),
        set.tags().collect::<Vec<_>>()
    );
}

#[test]
fn keyword_lookup_drives_access() {
    // pydicom's `ds.PatientName` equivalent: resolve the keyword through the dictionary first.
    let name = dictionary::lookup_keyword("PatientName").unwrap();
    assert_eq!(name.tag, tag(0x0010, 0x0010));
    assert_eq!(name.vr.fixed(), Some(Vr::Pn));

    let mut set = DataSet::new();
    set.set_string(name.tag, Vr::Pn, "Doe^John");
    assert_eq!(set.string(name.tag).unwrap(), "Doe^John");
}

#[test]
fn equality_compares_elements_and_endian() {
    let mut a = DataSet::new();
    a.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    let b = a.clone();
    assert_eq!(a, b);

    let mut different_value = DataSet::new();
    different_value.set_string(tag(0x0010, 0x0010), Vr::Pn, "Roe^Jane");
    assert_ne!(a, different_value);

    let mut different_vr = DataSet::new();
    different_vr.set_string(tag(0x0010, 0x0010), Vr::Lo, "Doe^John");
    assert_ne!(a, different_vr);

    // Divergence from pydicom: the byte order is part of equality.
    let mut big = DataSet::with_endian(Endian::Big);
    big.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    assert_ne!(a, big);
}

#[test]
fn clone_is_independent() {
    let mut original = DataSet::new();
    original.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");

    let mut copy = original.clone();
    copy.set_string(tag(0x0010, 0x0010), Vr::Pn, "Roe^Jane");
    assert_eq!(original.string(tag(0x0010, 0x0010)).unwrap(), "Doe^John");
}

#[test]
fn endian_changes_numeric_interpretation() {
    let mut set = DataSet::with_endian(Endian::Big);
    set.set(
        tag(0x0028, 0x0010),
        Vr::Us,
        Value::from_bytes(vec![0x02, 0x00]),
    );
    assert_eq!(set.endian(), Endian::Big);
    assert_eq!(set.u16(tag(0x0028, 0x0010)).unwrap(), 512);

    set.set_endian(Endian::Little);
    assert_eq!(set.u16(tag(0x0028, 0x0010)).unwrap(), 2);
}

#[test]
fn default_endian_is_little() {
    assert_eq!(DataSet::new().endian(), Endian::Little);
}

#[test]
fn sequences_nest_three_levels() {
    let mut leaf = DataSet::new();
    leaf.set_string(tag(0x0008, 0x0100), Vr::Sh, "CODE");

    let mut middle = DataSet::new();
    middle.set_sequence(tag(0x0040, 0xA730), vec![leaf]);

    let mut top = DataSet::new();
    top.set_sequence(tag(0x0040, 0x0275), vec![middle]);

    let first = top.sequence(tag(0x0040, 0x0275)).unwrap();
    assert_eq!(first.len(), 1);
    let second = first[0].sequence(tag(0x0040, 0xA730)).unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].string(tag(0x0008, 0x0100)).unwrap(), "CODE");
}

#[test]
fn empty_sequence_reads_as_no_items() {
    let mut set = DataSet::new();
    set.set_sequence(tag(0x0008, 0x1110), Vec::new());
    assert!(set.sequence(tag(0x0008, 0x1110)).unwrap().is_empty());
    assert_eq!(set.get(tag(0x0008, 0x1110)).unwrap().vr, Vr::Sq);
    assert!(set.get(tag(0x0008, 0x1110)).unwrap().is_empty());
}

#[test]
fn sequence_access_rejects_a_non_sequence_vr() {
    let mut set = DataSet::new();
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    assert!(matches!(
        set.sequence(tag(0x0010, 0x0010)),
        Err(Error::NotASequence(Vr::Pn))
    ));
}

#[test]
fn mutable_sequence_access_promotes_an_empty_value() {
    let mut set = DataSet::new();
    set.set(tag(0x0008, 0x1110), Vr::Sq, Value::Empty);
    {
        let element = set.get_mut(tag(0x0008, 0x1110)).unwrap();
        let items = element.as_sequence_mut().unwrap();
        items.push(DataSet::new());
    }
    assert_eq!(set.sequence(tag(0x0008, 0x1110)).unwrap().len(), 1);
}

#[test]
fn iter_mut_mutation_is_visible() {
    let mut set = DataSet::new();
    set.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    for element in set.iter_mut() {
        element.value = Value::from_str("Roe^Jane");
    }
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Roe^Jane");
}

#[test]
fn private_tags_are_recognised_in_a_data_set() {
    let mut set = DataSet::new();
    set.set_string(tag(0x0011, 0x0010), Vr::Lo, "MY CREATOR");
    set.set(tag(0x0011, 0x1001), Vr::Un, Value::from_bytes(vec![1, 2]));

    let tags: Vec<Tag> = set.tags().collect();
    assert!(tags.contains(&tag(0x0011, 0x0010)));
    assert!(tag(0x0011, 0x0010).is_private());
    assert!(tag(0x0011, 0x0010).is_private_creator());
    assert!(!tag(0x0011, 0x1001).is_private_creator());
    assert_eq!(set.string(tag(0x0011, 0x0010)).unwrap(), "MY CREATOR");
}

#[test]
fn group_length_elements_are_stored_and_classified() {
    let mut set = DataSet::new();
    set.set(tag(0x0008, 0x0000), Vr::Ul, Value::from_u32s(&[16]));
    let element = set.element(tag(0x0008, 0x0000)).unwrap();
    assert!(element.tag.is_group_length());
    assert_eq!(element.u32s(Endian::Little).unwrap(), vec![16]);
}

#[test]
fn large_data_sets_keep_ordering() {
    let mut set = DataSet::new();
    // Insert in descending order so ordering cannot come from insertion order.
    for element in (1..=500u16).rev() {
        set.set_string(tag(0x0008, element), Vr::Sh, "X");
    }
    assert_eq!(set.len(), 500);
    let tags: Vec<Tag> = set.tags().collect();
    assert_eq!(tags.len(), 500);
    for (index, tag) in tags.iter().enumerate() {
        assert_eq!(tag.element(), (index + 1) as u16);
        if index > 0 {
            assert!(tags[index - 1] < *tag);
        }
    }
}

#[test]
fn missing_elements_report_missing_tag() {
    let set = DataSet::new();
    assert!(matches!(
        set.element(tag(0x0010, 0x0010)),
        Err(Error::MissingTag(tag)) if tag == Tag::new(0x0010, 0x0010)
    ));
    assert!(matches!(
        set.string(tag(0x0010, 0x0010)),
        Err(Error::MissingTag(_))
    ));
    assert!(matches!(
        set.sequence(tag(0x0088, 0x0140)),
        Err(Error::MissingTag(_))
    ));
}

#[test]
fn empty_value_reports_no_value() {
    let mut set = DataSet::new();
    set.set(tag(0x0028, 0x0010), Vr::Us, Value::Empty);
    assert!(matches!(
        set.u16(tag(0x0028, 0x0010)),
        Err(Error::NoValue(tag)) if tag == Tag::new(0x0028, 0x0010)
    ));
}

#[test]
fn multi_value_strings() {
    let mut set = DataSet::new();
    set.set_string(tag(0x0008, 0x0008), Vr::Cs, "ORIGINAL\\PRIMARY");
    let values = set.strings(tag(0x0008, 0x0008)).unwrap();
    let values: Vec<&str> = values.iter().map(|value| value.as_ref()).collect();
    assert_eq!(values, vec!["ORIGINAL", "PRIMARY"]);
    assert_eq!(set.string(tag(0x0008, 0x0008)).unwrap(), "ORIGINAL");
}

/// Mirrors pydicom's equality tests: insertion order never matters.
#[test]
fn equality_is_independent_of_insertion_order() {
    let mut a = DataSet::new();
    a.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");
    a.set_u16(tag(0x0028, 0x0010), 512);

    let mut b = DataSet::new();
    b.set_u16(tag(0x0028, 0x0010), 512);
    b.set_string(tag(0x0010, 0x0010), Vr::Pn, "Doe^John");

    assert_eq!(a, b);
}

/// Mirrors pydicom's `test_equality_sequence` and `test_inequality_sequence`.
#[test]
fn equality_compares_sequence_content() {
    fn build(code: &str) -> DataSet {
        let mut item = DataSet::new();
        item.set_string(tag(0x0008, 0x0100), Vr::Sh, code);
        let mut top = DataSet::new();
        top.set_sequence(tag(0x0040, 0x0275), vec![item]);
        top
    }

    assert_eq!(build("CODE"), build("CODE"));
    assert_ne!(build("CODE"), build("OTHER"));
}

/// Mirrors pydicom's `test_equality_private`.
#[test]
fn equality_compares_private_elements() {
    let mut a = DataSet::new();
    a.set_string(tag(0x0011, 0x0010), Vr::Lo, "MY CREATOR");
    a.set(tag(0x0011, 0x1001), Vr::Un, Value::from_bytes(vec![1, 2]));

    let mut b = a.clone();
    assert_eq!(a, b);

    b.set(tag(0x0011, 0x1001), Vr::Un, Value::from_bytes(vec![1, 3]));
    assert_ne!(a, b);
}

#[test]
fn typed_helpers_write_canonical_values() {
    let mut set = DataSet::new();
    set.set_u16(tag(0x0028, 0x0010), 512);
    assert_eq!(set.u16(tag(0x0028, 0x0010)).unwrap(), 512);
    assert_eq!(set.get(tag(0x0028, 0x0010)).unwrap().vr, Vr::Us);

    set.set_string(tag(0x0018, 0x0050), Vr::Ds, "1.5\\2.25");
    assert_eq!(
        set.decimal_values(tag(0x0018, 0x0050)).unwrap(),
        vec![1.5, 2.25]
    );

    set.set(tag(0x0028, 0x0120), Vr::Fd, Value::from_f64s(&[1.0, 2.0]));
    assert_eq!(set.f64s(tag(0x0028, 0x0120)).unwrap(), vec![1.0, 2.0]);
}
