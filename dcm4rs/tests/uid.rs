//! Ports of `pydicom/tests/test_uid.py`.
//!
//! Each case names the pydicom test it comes from. `TestGenerateUID` covers the `2.25.` form
//! instead of pydicom's registered prefix, so the cases assert the shape rather than the digits.
//! `TestRegisterTransferSyntax` is absent: it needs a mutable registry, which does not exist
//! here.

use dcm4rs::uid::is_valid;
use dcm4rs::Uid;

const IMPLICIT: &str = "1.2.840.10008.1.2";
const EXPLICIT_LITTLE: &str = "1.2.840.10008.1.2.1";
const DEFLATED_LITTLE: &str = "1.2.840.10008.1.2.1.99";
const EXPLICIT_BIG: &str = "1.2.840.10008.1.2.2";
const JPEG_BASELINE: &str = "1.2.840.10008.1.2.4.50";
const CT_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.2";

fn uid(value: &str) -> Uid {
    Uid::new(value).expect("valid UID")
}

/// pydicom: `test_storage_sopclass_uids`
#[test]
fn storage_sop_class_uids_are_registered() {
    assert_eq!(uid(CT_IMAGE_STORAGE).name(), "CT Image Storage");
}

/// pydicom: `TestUID::test_is_implicit_vr`
#[test]
fn is_implicit_vr() {
    assert_eq!(uid(IMPLICIT).is_implicit_vr(), Some(true));
    assert_eq!(uid(EXPLICIT_LITTLE).is_implicit_vr(), Some(false));
    assert_eq!(uid(DEFLATED_LITTLE).is_implicit_vr(), Some(false));
    assert_eq!(uid(EXPLICIT_BIG).is_implicit_vr(), Some(false));
    assert_eq!(uid(JPEG_BASELINE).is_implicit_vr(), Some(false));
    // Not a transfer syntax: pydicom raises ValueError, `None` here.
    assert_eq!(uid(CT_IMAGE_STORAGE).is_implicit_vr(), None);
}

/// pydicom: `TestUID::test_is_little_endian`
#[test]
fn is_little_endian() {
    assert_eq!(uid(IMPLICIT).is_little_endian(), Some(true));
    assert_eq!(uid(EXPLICIT_LITTLE).is_little_endian(), Some(true));
    assert_eq!(uid(DEFLATED_LITTLE).is_little_endian(), Some(true));
    assert_eq!(uid(EXPLICIT_BIG).is_little_endian(), Some(false));
    assert_eq!(uid(JPEG_BASELINE).is_little_endian(), Some(true));
    assert_eq!(uid(CT_IMAGE_STORAGE).is_little_endian(), None);
}

/// pydicom: `TestUID::test_is_deflated`
#[test]
fn is_deflated() {
    assert_eq!(uid(IMPLICIT).is_deflated(), Some(false));
    assert_eq!(uid(EXPLICIT_LITTLE).is_deflated(), Some(false));
    assert_eq!(uid(DEFLATED_LITTLE).is_deflated(), Some(true));
    assert_eq!(uid(EXPLICIT_BIG).is_deflated(), Some(false));
    assert_eq!(uid(JPEG_BASELINE).is_deflated(), Some(false));

    // JPIP Referenced (not deflated) and the two JPIP deflated syntaxes (PS3.5 A.7 and A.12).
    assert_eq!(uid("1.2.840.10008.1.2.4.94").is_deflated(), Some(false));
    assert_eq!(uid("1.2.840.10008.1.2.4.95").is_deflated(), Some(true));
    assert_eq!(uid("1.2.840.10008.1.2.4.204").is_deflated(), Some(false));
    assert_eq!(uid("1.2.840.10008.1.2.4.205").is_deflated(), Some(true));

    assert_eq!(uid(CT_IMAGE_STORAGE).is_deflated(), None);
}

/// pydicom: `TestUID::test_is_transfer_syntax`
#[test]
fn is_transfer_syntax() {
    assert!(uid(IMPLICIT).is_transfer_syntax());
    assert!(uid(EXPLICIT_LITTLE).is_transfer_syntax());
    assert!(uid(DEFLATED_LITTLE).is_transfer_syntax());
    assert!(uid(EXPLICIT_BIG).is_transfer_syntax());
    assert!(uid(JPEG_BASELINE).is_transfer_syntax());
    assert!(!uid(CT_IMAGE_STORAGE).is_transfer_syntax());
}

/// pydicom: `TestUID::test_is_compressed`
#[test]
fn is_compressed() {
    assert_eq!(uid(IMPLICIT).is_compressed(), Some(false));
    assert_eq!(uid(EXPLICIT_LITTLE).is_compressed(), Some(false));
    assert_eq!(uid(DEFLATED_LITTLE).is_compressed(), Some(false));
    assert_eq!(uid(EXPLICIT_BIG).is_compressed(), Some(false));
    assert_eq!(uid(JPEG_BASELINE).is_compressed(), Some(true));
    assert_eq!(uid(CT_IMAGE_STORAGE).is_compressed(), None);
}

/// pydicom: `TestUID::test_is_encapsulated`
#[test]
fn is_encapsulated_matches_compressed() {
    assert_eq!(uid(IMPLICIT).is_encapsulated(), Some(false));
    assert_eq!(uid(EXPLICIT_LITTLE).is_encapsulated(), Some(false));
    assert_eq!(uid(DEFLATED_LITTLE).is_encapsulated(), Some(false));
    assert_eq!(uid(EXPLICIT_BIG).is_encapsulated(), Some(false));
    assert_eq!(uid(JPEG_BASELINE).is_encapsulated(), Some(true));
    assert_eq!(uid(CT_IMAGE_STORAGE).is_encapsulated(), None);
}

/// pydicom: `TestUID::test_name`
#[test]
fn name() {
    assert_eq!(uid(IMPLICIT).name(), "Implicit VR Little Endian");
    assert_eq!(uid(CT_IMAGE_STORAGE).name(), "CT Image Storage");
    // An unregistered UID names itself.
    assert_eq!(uid("1.2.3").name(), "1.2.3");
}

/// pydicom: `TestUID::test_type`
#[test]
fn kind() {
    assert_eq!(uid(IMPLICIT).kind(), "Transfer Syntax");
    assert_eq!(uid(CT_IMAGE_STORAGE).kind(), "SOP Class");
}

/// pydicom: `TestUID::test_info`
#[test]
fn info() {
    assert_eq!(uid(IMPLICIT).info(), "Default Transfer Syntax for DICOM");
    assert_eq!(uid(CT_IMAGE_STORAGE).info(), "");
}

/// pydicom: `TestUID::test_is_retired`
#[test]
fn is_retired() {
    assert!(!uid(IMPLICIT).is_retired());
    assert!(uid(EXPLICIT_BIG).is_retired());
}

/// pydicom: `TestUID::test_keyword`
#[test]
fn keyword() {
    assert_eq!(uid(IMPLICIT).keyword(), "ImplicitVRLittleEndian");
    assert_eq!(uid(CT_IMAGE_STORAGE).keyword(), "CTImageStorage");
}

/// pydicom: `TestUID::test_is_valid`
#[test]
fn is_valid_rejects_malformed_values() {
    for invalid in [
        "1".repeat(65),
        format!("1.{}", "2".repeat(63)),
        String::new(),
        ".".to_owned(),
        "1.".to_owned(),
        "1.01".to_owned(),
        "1.a.2".to_owned(),
    ] {
        assert!(!is_valid(&invalid), "{invalid} should be invalid");
        assert!(Uid::new(invalid).is_err());
    }

    for valid in [
        "0",
        "1",
        "0.1",
        &"1".repeat(64),
        &format!("1.{}", "2".repeat(62)),
        "1.0.23",
    ] {
        assert!(is_valid(valid), "{valid} should be valid");
    }
}

/// pydicom: `TestUID::test_is_private`
#[test]
fn is_private() {
    assert!(uid("1.2.840.10009.1.2").is_private());
    assert!(!uid(IMPLICIT).is_private());
}

/// pydicom: `TestUID::test_transitive`
#[test]
fn unregistered_lookups_stay_empty() {
    let uid = uid("1.2.840.10008.1.1");
    assert_eq!(uid.name(), "Verification SOP Class");
    assert_eq!(uid.as_str(), "1.2.840.10008.1.1");
}

/// pydicom: `TestUIDPrivate` (the class's whole set of lookups)
#[test]
fn private_uid_lookups_are_empty() {
    let private = uid("9.9.999.90009.1.2");
    assert_eq!(private.is_implicit_vr(), None);
    assert_eq!(private.is_little_endian(), None);
    assert_eq!(private.is_deflated(), None);
    assert!(!private.is_transfer_syntax());
    assert_eq!(private.is_compressed(), None);
    assert_eq!(private.is_encapsulated(), None);
    assert_eq!(private.name(), "9.9.999.90009.1.2");
    assert_eq!(private.kind(), "");
    assert_eq!(private.info(), "");
    assert!(!private.is_retired());
    assert!(private.is_valid());
    assert!(private.is_private());
    assert_eq!(private.keyword(), "");
}

/// pydicom: `TestUIDPrivate::test_private_encoding`
#[test]
fn private_encoding_makes_a_private_uid_a_transfer_syntax() {
    let private = uid("9.9.999.90009.1.2").with_private_encoding(true, true);
    assert!(private.is_transfer_syntax());
    assert_eq!(private.is_implicit_vr(), Some(true));
    assert_eq!(private.is_little_endian(), Some(true));

    let explicit_big = private.clone().with_private_encoding(false, false);
    assert_eq!(explicit_big.is_implicit_vr(), Some(false));
    assert_eq!(explicit_big.is_little_endian(), Some(false));

    // The declared encoding travels with a copy, as pydicom copies it on `UID(uid)`.
    let copied = explicit_big.clone();
    assert!(copied.is_transfer_syntax());
    assert_eq!(copied.is_implicit_vr(), Some(false));
    assert_eq!(copied.is_little_endian(), Some(false));
}

#[test]
fn construction_trims_whitespace() {
    // pydicom strips the value before storing it; the padding of a VR UI value ends up here.
    let padded = Uid::new("  1.2.840.10008.1.2  ").expect("valid UID");
    assert_eq!(padded.as_str(), IMPLICIT);
    assert_eq!(padded, IMPLICIT);
}

#[test]
fn equality_and_ordering_follow_the_text() {
    let first = uid(IMPLICIT);
    let second = uid(IMPLICIT);
    assert_eq!(first, second);
    assert_eq!(first, "1.2.840.10008.1.2");
    assert_ne!(first, uid(EXPLICIT_LITTLE));

    // UIDs sort lexicographically, which is what the registry lookup relies on.
    let mut uids = [uid(EXPLICIT_LITTLE), uid(IMPLICIT), uid(JPEG_BASELINE)];
    uids.sort();
    assert_eq!(uids[0], IMPLICIT);
}

/// pydicom: `test_generate_uid`, with the `2.25.` UUID root instead of pydicom's prefix.
#[test]
fn generated_uids_are_valid_and_unique() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..1000 {
        let generated = Uid::generate().expect("valid generated UID");
        assert!(generated.as_str().starts_with("2.25."));
        assert!(generated.as_str().len() <= 64);
        assert!(generated.is_private());
        assert!(
            seen.insert(generated.as_str().to_owned()),
            "{generated} repeated"
        );
    }
    assert!(dcm4rs::uid::generate_uid().unwrap().is_valid());
}

/// pydicom: `test_generate_uid_use_prefix`, including a prefix that leaves no room for digits.
#[test]
fn a_registered_prefix_can_be_used() {
    let generated = Uid::generate_with_root("1.2.840.10008.9.1").unwrap();
    assert!(generated.as_str().starts_with("1.2.840.10008.9.1."));
    assert!(generated.as_str().len() <= 64);

    // Sixty-two characters of prefix leave one digit, which is a valid UID.
    let long_root = format!("1.{}", "2".repeat(56));
    let generated = Uid::generate_with_root(&long_root).unwrap();
    assert!(generated.as_str().len() <= 64);
    assert!(generated.as_str().starts_with(&long_root));

    assert!(Uid::generate_with_root("not a root").is_err());
    assert!(Uid::generate_with_root("1..2").is_err());
}
