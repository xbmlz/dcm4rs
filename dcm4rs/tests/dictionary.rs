//! Dictionary behaviour, mirroring pydicom's `tests/test_dictionary.py`.
//!
//! Covered: exact and repeating-group lookups, keyword lookups, VR classification and the
//! invariants of the generated tables. pydicom's dictionary tests also drive `DataElement`
//! construction from keywords; the equivalent here is `lookup_keyword` plus `DataSet` access,
//! tested in `dataset.rs`.

use dcm4rs::dictionary::{entries, lookup, lookup_keyword, repeaters, tags, vr, VrDef};
use dcm4rs::{Tag, Vr};

#[test]
fn lookup_common_attributes() {
    let entry = lookup(Tag::new(0x0010, 0x0010)).expect("Patient Name should be in the dictionary");
    assert_eq!(entry.keyword, "PatientName");
    assert_eq!(entry.name, "Patient's Name");
    assert_eq!(entry.vr, VrDef::Fixed(Vr::Pn));
    assert_eq!(entry.vm, "1");
    assert!(!entry.retired);

    let pixel_data =
        lookup(Tag::new(0x7FE0, 0x0010)).expect("Pixel Data should be in the dictionary");
    assert_eq!(pixel_data.keyword, "PixelData");
    assert!(pixel_data.vr.is_ambiguous());
    assert!(pixel_data.vr.contains(Vr::Ob));
    assert!(pixel_data.vr.contains(Vr::Ow));
    assert_eq!(pixel_data.vr.fixed(), None);

    let retired =
        lookup(Tag::new(0x0008, 0x0001)).expect("Length to End should be in the dictionary");
    assert!(retired.retired);
    assert_eq!(retired.name, "Length to End");
}

#[test]
fn lookup_by_keyword_is_case_insensitive() {
    let entry = lookup_keyword("patientid").unwrap();
    assert_eq!(entry.tag, Tag::new(0x0010, 0x0020));
    assert_eq!(entry.keyword, "PatientID");

    let upper = lookup_keyword("PATIENTNAME").unwrap();
    assert_eq!(upper.tag, Tag::new(0x0010, 0x0010));
    assert_eq!(lookup_keyword("  PatientName  ").map(|e| e.tag), None);
    assert!(lookup_keyword("NotAKeyword").is_none());
}

#[test]
fn lookup_repeating_groups() {
    let entry =
        lookup(Tag::new(0x6002, 0x0010)).expect("Overlay Rows should hit the repeating group");
    assert_eq!(entry.keyword, "OverlayRows");
    assert_eq!(entry.vr, VrDef::Fixed(Vr::Us));

    let entry =
        lookup(Tag::new(0x6002, 0x3000)).expect("Overlay Data should hit the repeating group");
    assert_eq!(entry.keyword, "OverlayData");

    let data = lookup(Tag::new(0x60A0, 0x3000)).unwrap();
    assert_eq!(data.keyword, "OverlayData");
    // Overlay Data is `OB or OW` in the registry, exactly as Pixel Data is.
    assert!(data.vr.is_ambiguous());
    assert!(data.vr.contains(Vr::Ow));

    let rows = lookup(Tag::new(0x60FF, 0x0010)).unwrap();
    assert_eq!(rows.keyword, "OverlayRows");
}

#[test]
fn private_creator_tags_have_no_dictionary_entry() {
    // The registry has no entries for private tags, so `vr` returns None; the reader supplies
    // the VR from the tag shape instead (private creator `LO`, other private elements `UN`),
    // as pydicom's `hooks._private_vr_for_tag` does (pydicom/hooks.py:139-152).
    for element in [0x0010u16, 0x0020, 0x00FF] {
        assert!(
            lookup(Tag::new(0x0011, element)).is_none(),
            "{element:#06X}"
        );
    }
    assert!(lookup(Tag::new(0x0011, 0x1001)).is_none());
}

#[test]
fn vr_lookup_for_common_tags() {
    assert_eq!(vr(Tag::new(0x0010, 0x0010)), Some(VrDef::Fixed(Vr::Pn)));
    assert_eq!(vr(Tag::new(0x0028, 0x0010)), Some(VrDef::Fixed(Vr::Us)));
    assert_eq!(vr(Tag::new(0x0008, 0x0016)), Some(VrDef::Fixed(Vr::Ui)));
    assert_eq!(vr(Tag::new(0x0018, 0x0050)), Some(VrDef::Fixed(Vr::Ds)));
    assert_eq!(vr(Tag::new(0x0020, 0x0013)), Some(VrDef::Fixed(Vr::Is)));
    assert!(vr(Tag::new(0x7FE0, 0x0010)).unwrap().is_ambiguous());
    assert_eq!(vr(Tag::new(0x0028, 0x0103)), Some(VrDef::Fixed(Vr::Us)));
    assert_eq!(vr(Tag::new(0x0011, 0x1234)), None);
}

#[test]
fn ambiguous_entries_are_binary_or_integer_only() {
    let mut count = 0;
    for entry in entries() {
        if let VrDef::Ambiguous(set) = entry.vr {
            count += 1;
            assert!(!set.is_empty(), "{:?}", entry.tag);
            for candidate in set {
                assert!(
                    matches!(*candidate, Vr::Us | Vr::Ss | Vr::Ow | Vr::Ob),
                    "{:?} offers {:?}",
                    entry.tag,
                    candidate
                );
            }
            // Sequences are never ambiguous: the wire structure has to be known up front.
            assert!(!set.contains(&Vr::Sq), "{:?}", entry.tag);
        }
    }
    assert!(count > 0, "the registry should contain ambiguous entries");
}

#[test]
fn entries_are_sorted_and_unique() {
    let all = entries();
    for pair in all.windows(2) {
        assert!(
            pair[0].tag < pair[1].tag,
            "dictionary order broken at {:?}",
            pair[1].tag
        );
    }
}

#[test]
fn repeaters_match_their_mask_pattern() {
    for repeater in repeaters() {
        let raw = repeater.entry.tag.as_u32();
        assert_eq!(
            raw & repeater.mask,
            repeater.value,
            "repeater {:?} does not match its own mask",
            repeater.entry.tag
        );
        assert!(!repeater.entry.keyword.is_empty());
    }
}

/// Mirrors pydicom's `test_repeaters` and `test_dictionary_VM`.
#[test]
fn descriptions_and_value_multiplicity() {
    let transform =
        lookup(Tag::new(0x0028, 0x0400)).expect("Transform Label should be in the dictionary");
    assert_eq!(transform.name, "Transform Label");
    assert_eq!(transform.vm, "1");

    // The repeating group shares one description across a whole block of elements.
    let rows = lookup(Tag::new(0x0028, 0x0410)).expect("the repeater should cover (0028,0410)");
    assert_eq!(rows.name, "Rows For Nth Order Coefficients");

    assert_eq!(lookup(Tag::new(0x0008, 0x0008)).unwrap().vm, "2-n");
    assert_eq!(lookup(Tag::new(0x0002, 0x0001)).unwrap().vm, "1");
}

/// Mirrors pydicom's `test_dict_has_tag`.
#[test]
fn dictionary_has_tag_lookups() {
    assert!(lookup(Tag::new(0x0010, 0x0010)).is_some());
    assert!(lookup(Tag::new(0x1111, 0x0010)).is_none());
    assert!(lookup_keyword("PatientName").is_some());
    assert!(lookup_keyword("PatientMane").is_none());
}

fn repeater_has_tag(tag: Tag) -> bool {
    let raw = tag.as_u32();
    repeaters()
        .iter()
        .any(|repeater| raw & repeater.mask == repeater.value)
}

fn repeater_has_keyword(keyword: &str) -> bool {
    repeaters()
        .iter()
        .any(|repeater| repeater.entry.keyword.eq_ignore_ascii_case(keyword))
}

/// Mirrors pydicom's `test_repeater_has_tag` and `test_repeater_has_keyword`.
#[test]
fn repeater_predicates() {
    assert!(repeater_has_tag(Tag::new(0x6000, 0x0010)));
    assert!(repeater_has_tag(Tag::new(0x6002, 0x0010)));
    assert!(!repeater_has_tag(Tag::new(0x0010, 0x0010)));

    assert!(repeater_has_keyword("OverlayData"));
    // Pixel Data is a plain entry, not part of a repeating group.
    assert!(!repeater_has_keyword("PixelData"));
}

#[test]
fn dictionary_size_and_constants() {
    assert!(
        entries().len() > 5_000,
        "too few dictionary entries: {}",
        entries().len()
    );
    assert!(!repeaters().is_empty());

    assert_eq!(tags::PATIENT_NAME, Tag::new(0x0010, 0x0010));
    assert_eq!(tags::PIXEL_DATA, Tag::new(0x7FE0, 0x0010));
    assert_eq!(tags::SOP_INSTANCE_UID, Tag::new(0x0008, 0x0018));
    assert_eq!(tags::TRANSFER_SYNTAX_UID, Tag::new(0x0002, 0x0010));
    assert_eq!(
        tags::FILE_META_INFORMATION_VERSION,
        Tag::new(0x0002, 0x0001)
    );
}
