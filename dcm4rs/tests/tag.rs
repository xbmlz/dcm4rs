//! Tag behaviour, mirroring pydicom's `tests/test_tag.py`.
//!
//! Covered here: parsing of the text forms, ordering, private/creator/group-length classification,
//! and `Display`/`Debug` spellings. pydicom additionally carries Python-only helpers
//! (`Tag.__repr__` with keyword names, `Tag.is_private` on keyword-based tags); those map to
//! [`dcm4rs::dictionary::lookup`] and are tested in `dictionary.rs`.

use std::str::FromStr;

use dcm4rs::Tag;

#[test]
fn parse_accepts_every_text_form() {
    let expected = Tag::new(0x0010, 0x0010);
    assert_eq!(Tag::parse("(0010,0010)").unwrap(), expected);
    assert_eq!(Tag::parse("0010,0010").unwrap(), expected);
    assert_eq!(Tag::parse("00100010").unwrap(), expected);
    assert_eq!(Tag::parse("  ( 0010 , 0010 )  ").unwrap(), expected);
    assert_eq!(Tag::parse("0010 , 0010").unwrap(), expected);
    assert_eq!(Tag::from_str("(0010,0010)").unwrap(), expected);
}

#[test]
fn parse_is_case_insensitive_for_hex_digits() {
    assert_eq!(Tag::parse("(00AF,00b0)").unwrap(), Tag::new(0x00AF, 0x00B0));
}

#[test]
fn parse_rejects_malformed_text() {
    for text in [
        "(0010,001)",
        "0010,0010,0010",
        "zzzzzzzz",
        "(0010,0010",
        "0010)",
        "0010",
        "",
        "(0010,00100)",
        "0x00100010",
    ] {
        assert!(
            Tag::parse(text).is_err(),
            "expected {text:?} to be rejected, got {:?}",
            Tag::parse(text)
        );
    }
}

#[test]
fn display_and_debug_spellings() {
    let tag = Tag::new(0x0008, 0x0000);
    assert_eq!(tag.to_string(), "(0008,0000)");
    assert_eq!(format!("{tag:?}"), "Tag(0008,0000)");
    assert_eq!(tag.group(), 0x0008);
    assert_eq!(tag.element(), 0x0000);
    assert_eq!(tag.as_u32(), 0x0008_0000);
}

#[test]
fn from_u32_round_trips() {
    for raw in [0u32, 1, 0x0010_0010, 0x7FE0_0010, 0xFFFF_FFFF] {
        let tag = Tag::from_u32(raw);
        assert_eq!(tag.as_u32(), raw);
        assert_eq!(Tag::new(tag.group(), tag.element()), tag);
    }
}

#[test]
fn ordering_is_numeric_group_then_element() {
    // DICOM requires ascending (group, element) order, which is exactly what the derived
    // ordering gives.
    assert!(Tag::new(0x0008, 0x9999) < Tag::new(0x0010, 0x0000));
    assert!(Tag::new(0x0010, 0x0010) < Tag::new(0x0010, 0x0020));
    assert_eq!(Tag::new(0x7FE0, 0x0010), Tag::new(0x7FE0, 0x0010));
}

#[test]
fn private_group_detection() {
    assert!(Tag::new(0x0009, 0x0000).is_private());
    assert!(Tag::new(0x0011, 0x0001).is_private());
    assert!(!Tag::new(0x0010, 0x0010).is_private());
    assert!(!Tag::new(0x0008, 0x0000).is_private());

    // Groups reserved by the standard are not "private" even though they are odd.
    for group in [0x0001u16, 0x0003, 0x0005, 0x0007, 0xFFFF] {
        assert!(!Tag::new(group, 0x0010).is_private());
    }
}

#[test]
fn private_creator_range() {
    assert!(Tag::new(0x0011, 0x0010).is_private_creator());
    assert!(Tag::new(0x0011, 0x00FF).is_private_creator());
    assert!(!Tag::new(0x0011, 0x000F).is_private_creator());
    assert!(!Tag::new(0x0011, 0x0100).is_private_creator());
    // Private creator tags only exist in private groups.
    assert!(!Tag::new(0x0010, 0x0010).is_private_creator());
}

#[test]
fn group_length_detection() {
    assert!(Tag::new(0x0008, 0x0000).is_group_length());
    assert!(Tag::new(0x7FE0, 0x0000).is_group_length());
    assert!(!Tag::new(0x0008, 0x0001).is_group_length());
}

#[test]
fn successor_steps_one_tag() {
    assert_eq!(
        Tag::new(0x0008, 0xFFFF).successor(),
        Some(Tag::new(0x0009, 0x0000))
    );
    assert_eq!(
        Tag::new(0x0010, 0x0010).successor(),
        Some(Tag::new(0x0010, 0x0011))
    );
    assert_eq!(Tag::new(0xFFFF, 0xFFFF).successor(), None);
}
