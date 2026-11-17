//! Transfer syntax registry behaviour.
//!
//! pydicom keeps the same information in `UID` dictionaries (`pydicom.uid`); there is no dedicated
//! test module upstream, so the checks below pin the properties the reader and writer depend on.

use dcm4rs::{Endian, TransferSyntax};

#[test]
fn native_syntaxes_round_trip_through_their_uid() {
    for syntax in [
        TransferSyntax::ImplicitVrLittleEndian,
        TransferSyntax::ExplicitVrLittleEndian,
        TransferSyntax::ExplicitVrBigEndian,
        TransferSyntax::DeflatedExplicitVrLittleEndian,
    ] {
        let uid = syntax.uid();
        assert_eq!(TransferSyntax::from_uid(uid), Some(syntax));
        assert_eq!(syntax.to_string(), uid);
    }
}

#[test]
fn encapsulated_syntaxes_round_trip_through_their_uid() {
    for uid in [
        "1.2.840.10008.1.2.4.50",
        "1.2.840.10008.1.2.4.70",
        "1.2.840.10008.1.2.4.80",
        "1.2.840.10008.1.2.4.90",
        "1.2.840.10008.1.2.4.110",
        "1.2.840.10008.1.2.5",
    ] {
        let syntax = TransferSyntax::from_uid(uid).unwrap();
        assert!(syntax.is_encapsulated(), "{uid}");
        assert_eq!(syntax.uid(), uid);
        assert_eq!(syntax.to_string(), uid);
        // Encapsulated data is carried in an explicit VR little endian data set.
        assert!(syntax.is_explicit_vr(), "{uid}");
        assert_eq!(syntax.endian(), Endian::Little, "{uid}");
    }
}

#[test]
fn native_syntax_properties() {
    assert_eq!(
        TransferSyntax::ImplicitVrLittleEndian.endian(),
        Endian::Little
    );
    assert_eq!(
        TransferSyntax::ExplicitVrLittleEndian.endian(),
        Endian::Little
    );
    assert_eq!(TransferSyntax::ExplicitVrBigEndian.endian(), Endian::Big);

    assert!(!TransferSyntax::ImplicitVrLittleEndian.is_explicit_vr());
    assert!(TransferSyntax::ExplicitVrLittleEndian.is_explicit_vr());
    assert!(TransferSyntax::ExplicitVrBigEndian.is_explicit_vr());

    assert!(!TransferSyntax::ImplicitVrLittleEndian.is_encapsulated());
    assert!(!TransferSyntax::ExplicitVrLittleEndian.is_encapsulated());
    assert!(!TransferSyntax::ExplicitVrBigEndian.is_encapsulated());
}

#[test]
fn deflated_is_recognised_but_not_encapsulated() {
    let syntax = TransferSyntax::DeflatedExplicitVrLittleEndian;
    assert!(syntax.is_explicit_vr());
    assert!(!syntax.is_encapsulated());
    assert_eq!(syntax.endian(), Endian::Little);
}

#[test]
fn unknown_uids_do_not_resolve() {
    for uid in ["", "1.2.3.4", "1.2.840.10008.1.2.3", "not-a-uid"] {
        assert_eq!(TransferSyntax::from_uid(uid), None, "{uid:?}");
    }
}

#[test]
fn padded_uid_values_resolve() {
    // `UI` values are padded to even length, so meta elements often carry a trailing NUL.
    assert_eq!(
        TransferSyntax::from_uid("1.2.840.10008.1.2.1\0"),
        Some(TransferSyntax::ExplicitVrLittleEndian)
    );
    assert_eq!(
        TransferSyntax::from_uid(" 1.2.840.10008.1.2.2 "),
        Some(TransferSyntax::ExplicitVrBigEndian)
    );
    assert_eq!(
        TransferSyntax::from_uid("1.2.840.10008.1.2.4.50\0"),
        Some(TransferSyntax::Encapsulated("1.2.840.10008.1.2.4.50"))
    );
}
