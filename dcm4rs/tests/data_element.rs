//! Data element and value behaviour, mirroring pydicom's `tests/test_dataelem.py`.
//!
//! Covered: the VR table and its encoding classifications, text decoding (padding, multi-value,
//! UTF-8), numeric decoding per VR family, `DS`/`IS` parsing, empty values, long values, and the
//! raw byte surface.
//!
//! Not covered because the API does not exist here: Python-side value wrappers (`PersonName`,
//! `DSfloat`, `IS`, `MultiValue`), `VM`/`repr`, and keyword-based construction — pydicom's
//! equivalents of the latter are `dictionary::lookup_keyword` (see `dictionary.rs`). Charset
//! decoding is not implemented yet, so non-UTF-8 bytes are expected to fail (see `README`).

mod common;

use std::collections::BTreeSet;

use dcm4rs::{DataSet, Element, Endian, Error, Tag, Value, Vr};

use common::tag;

#[test]
fn vr_table_is_complete_and_round_trips() {
    assert_eq!(Vr::ALL.len(), 34);
    let mut seen = BTreeSet::new();
    for vr in Vr::ALL {
        assert_eq!(vr.to_string(), vr.as_str());
        assert_eq!(vr.as_str().parse::<Vr>().unwrap(), vr);
        assert!(seen.insert(vr), "duplicate VR {vr}");
    }
}

#[test]
fn vr_length_form_classification() {
    let long = [
        Vr::Ob,
        Vr::Od,
        Vr::Of,
        Vr::Ol,
        Vr::Ov,
        Vr::Ow,
        Vr::Sq,
        Vr::Uc,
        Vr::Un,
        Vr::Ur,
        Vr::Ut,
    ];
    for vr in Vr::ALL {
        assert_eq!(
            vr.uses_long_length(),
            long.contains(&vr),
            "wrong length form for {vr}"
        );
    }
}

#[test]
fn vr_textual_classification() {
    let textual = [
        Vr::Ae,
        Vr::As,
        Vr::Cs,
        Vr::Da,
        Vr::Ds,
        Vr::Dt,
        Vr::Is,
        Vr::Lo,
        Vr::Lt,
        Vr::Pn,
        Vr::Sh,
        Vr::St,
        Vr::Tm,
        Vr::Uc,
        Vr::Ui,
        Vr::Ur,
        Vr::Ut,
    ];
    let charset = [
        Vr::Lo,
        Vr::Lt,
        Vr::Pn,
        Vr::Sh,
        Vr::St,
        Vr::Uc,
        Vr::Ut,
        Vr::Ur,
    ];
    for vr in Vr::ALL {
        assert_eq!(vr.is_string(), textual.contains(&vr), "is_string for {vr}");
        assert_eq!(
            vr.uses_specific_character_set(),
            charset.contains(&vr),
            "charset for {vr}"
        );
        let splits = vr.is_string() && !matches!(vr, Vr::Lt | Vr::St | Vr::Ut | Vr::Ur);
        assert_eq!(
            vr.splits_on_backslash(),
            splits,
            "backslash splitting for {vr}"
        );
    }
}

#[test]
fn vr_padding_rules() {
    for vr in Vr::ALL {
        let expected = if vr == Vr::Ui {
            0
        } else if vr.is_string() {
            b' '
        } else {
            0
        };
        assert_eq!(vr.padding(), expected, "padding for {vr}");
    }
    assert_eq!(Vr::Ui.padding(), 0);
    assert_eq!(Vr::Pn.padding(), b' ');
    assert_eq!(Vr::Ob.padding(), 0);
}

#[test]
fn vr_parse_accepts_case_and_whitespace() {
    assert_eq!(" ob ".parse::<Vr>().unwrap(), Vr::Ob);
    assert_eq!("pn".parse::<Vr>().unwrap(), Vr::Pn);
    assert_eq!("\tur\n".parse::<Vr>().unwrap(), Vr::Ur);
}

#[test]
fn vr_parse_rejects_unknown_codes() {
    for text in ["ZZ", "", "A", "OBB", "0B"] {
        assert!(text.parse::<Vr>().is_err(), "{text:?} must not parse");
    }
    assert!(matches!(
        "ZZ".parse::<Vr>(),
        Err(Error::UnknownVr(text)) if text == "ZZ"
    ));
}

#[test]
fn text_values_split_on_backslash() {
    let names = Element::new(
        tag(0x0010, 0x0010),
        Vr::Pn,
        Value::from_str("Doe^John\\Roe^Jane"),
    );
    assert_eq!(names.string_values().unwrap(), vec!["Doe^John", "Roe^Jane"]);
    assert_eq!(names.string().unwrap(), "Doe^John");
}

#[test]
fn text_values_keep_empty_components() {
    let embedded = Element::new(tag(0x0040, 0x0006), Vr::Ae, Value::from_str("A\\\\B"));
    assert_eq!(embedded.string_values().unwrap(), vec!["A", "", "B"]);

    let leading = Element::new(tag(0x0040, 0x0006), Vr::Ae, Value::from_str("\\B"));
    assert_eq!(leading.string_values().unwrap(), vec!["", "B"]);

    let trailing = Element::new(tag(0x0040, 0x0006), Vr::Ae, Value::from_str("A\\"));
    assert_eq!(trailing.string_values().unwrap(), vec!["A", ""]);
}

#[test]
fn text_padding_is_trimmed_at_the_end_only() {
    let padded = Element::new(
        tag(0x0010, 0x0010),
        Vr::Pn,
        Value::from_bytes(b"Doe^John ".to_vec()),
    );
    assert_eq!(padded.string().unwrap(), "Doe^John");

    let interior = Element::new(tag(0x0010, 0x0010), Vr::Pn, Value::from_str("Doe John"));
    assert_eq!(interior.string().unwrap(), "Doe John");

    // Leading spaces are content, not padding (pydicom behaves the same for PN/LO).
    let leading = Element::new(tag(0x0008, 0x0081), Vr::Lo, Value::from_str(" Doe^John"));
    assert_eq!(leading.string().unwrap(), " Doe^John");
}

#[test]
fn ui_padding_is_a_nul_byte() {
    let uid = Element::new(
        tag(0x0002, 0x0002),
        Vr::Ui,
        Value::from_bytes(b"1.2.840.10008\0".to_vec()),
    );
    assert_eq!(uid.string().unwrap(), "1.2.840.10008");

    // A trailing space on a UI is padding too.
    let spaced = Element::new(
        tag(0x0020, 0x000D),
        Vr::Ui,
        Value::from_bytes(b"1.2.3 ".to_vec()),
    );
    assert_eq!(spaced.string().unwrap(), "1.2.3");
}

#[test]
fn text_decoding_rejects_non_utf8() {
    // Latin-1 "Müller": 0xFC is not a UTF-8 continuation start.
    let latin1 = Element::new(
        tag(0x0010, 0x0010),
        Vr::Pn,
        Value::from_bytes(vec![b'M', 0xFC, b'l', b'l', b'e', b'r']),
    );
    assert!(matches!(latin1.string(), Err(Error::InvalidUtf8(_))));
}

#[test]
fn text_decoding_accepts_utf8() {
    let utf8 = Element::new(tag(0x0010, 0x0010), Vr::Pn, Value::from_str("Müller^Hans"));
    assert_eq!(utf8.string().unwrap(), "Müller^Hans");
}

#[test]
fn text_access_on_binary_vr_is_rejected() {
    let rows = Element::new(tag(0x0028, 0x0010), Vr::Us, Value::from_u16s(&[512]));
    assert!(matches!(
        rows.string_values(),
        Err(Error::WrongVr {
            actual: Vr::Us,
            expected: "textual VR"
        })
    ));
}

#[test]
fn empty_text_value_reads_as_empty_string() {
    let empty = Element::new(tag(0x0010, 0x0010), Vr::Pn, Value::Empty);
    assert_eq!(empty.string().unwrap(), "");
    assert!(empty.string_values().unwrap().is_empty());
    assert!(empty.is_empty());
}

#[test]
fn u16_family_accepts_us_ss_and_un() {
    for vr in [Vr::Us, Vr::Ss, Vr::Un] {
        let element = Element::new(tag(0x0028, 0x0010), vr, Value::from_bytes(vec![0xFF, 0xFF]));
        assert_eq!(element.u16s(Endian::Little).unwrap(), vec![65535], "{vr}");
        assert_eq!(element.i16s(Endian::Little).unwrap(), vec![-1], "{vr}");
    }
}

#[test]
fn u16_rejects_odd_length_and_wrong_vr() {
    let odd = Element::new(
        tag(0x0028, 0x0010),
        Vr::Us,
        Value::from_bytes(vec![0, 0, 0]),
    );
    assert!(matches!(
        odd.u16s(Endian::Little),
        Err(Error::WrongLength {
            len: 3,
            unit: 2,
            ..
        })
    ));

    let text = Element::new(tag(0x0010, 0x0010), Vr::Pn, Value::from_str("Doe^John"));
    assert!(matches!(
        text.u16s(Endian::Little),
        Err(Error::WrongVr {
            actual: Vr::Pn,
            expected: "US/SS"
        })
    ));
}

#[test]
fn i32_family_and_units() {
    for vr in [Vr::Ul, Vr::Sl, Vr::Un] {
        let element = Element::new(tag(0x0008, 0x0000), vr, Value::from_bytes(vec![0xFF; 4]));
        assert_eq!(
            element.u32s(Endian::Little).unwrap(),
            vec![u32::MAX],
            "{vr}"
        );
        assert_eq!(element.i32s(Endian::Little).unwrap(), vec![-1], "{vr}");
    }

    let six = Element::new(tag(0x0008, 0x0000), Vr::Ul, Value::from_bytes(vec![0; 6]));
    assert!(matches!(
        six.i32s(Endian::Little),
        Err(Error::WrongLength {
            len: 6,
            unit: 4,
            ..
        })
    ));
}

#[test]
fn i64_family_and_units() {
    for vr in [Vr::Uv, Vr::Sv, Vr::Un] {
        let element = Element::new(tag(0x0008, 0x0000), vr, Value::from_bytes(vec![0xFF; 8]));
        assert_eq!(
            element.u64s(Endian::Little).unwrap(),
            vec![u64::MAX],
            "{vr}"
        );
        assert_eq!(element.i64s(Endian::Little).unwrap(), vec![-1], "{vr}");
    }

    let twelve = Element::new(tag(0x0008, 0x0000), Vr::Uv, Value::from_bytes(vec![0; 12]));
    assert!(matches!(
        twelve.u64s(Endian::Little),
        Err(Error::WrongLength {
            len: 12,
            unit: 8,
            ..
        })
    ));
}

#[test]
fn float_vrs_keep_their_precision() {
    let single = Element::new(tag(0x0028, 0x3006), Vr::Fl, Value::from_f32s(&[1.5]));
    assert_eq!(single.f32s(Endian::Little).unwrap(), vec![1.5]);
    assert!(matches!(
        single.f64s(Endian::Little),
        Err(Error::WrongVr {
            actual: Vr::Fl,
            expected: "FD"
        })
    ));

    let double = Element::new(tag(0x0018, 0x9087), Vr::Fd, Value::from_f64s(&[2.25, -0.5]));
    assert_eq!(double.f64s(Endian::Little).unwrap(), vec![2.25, -0.5]);
    assert_eq!(double.f64s(Endian::Big).unwrap().len(), 2);
}

#[test]
fn attribute_tags_decode_in_both_byte_orders() {
    let tags = Element::new(
        tag(0x0028, 0x0009),
        Vr::At,
        Value::from_tags(&[Tag::new(0x0010, 0x0010), Tag::new(0x0028, 0x0010)]),
    );
    assert_eq!(
        tags.tags(Endian::Little).unwrap(),
        vec![Tag::new(0x0010, 0x0010), Tag::new(0x0028, 0x0010)]
    );

    let big = Element::new(
        tag(0x0028, 0x0009),
        Vr::At,
        Value::from_bytes(vec![0x00, 0x10, 0x00, 0x10]),
    );
    assert_eq!(
        big.tags(Endian::Big).unwrap(),
        vec![Tag::new(0x0010, 0x0010)]
    );

    let short = Element::new(tag(0x0028, 0x0009), Vr::At, Value::from_bytes(vec![0; 2]));
    assert!(matches!(
        short.tags(Endian::Little),
        Err(Error::WrongLength {
            len: 2,
            unit: 4,
            ..
        })
    ));
}

#[test]
fn decimal_string_parsing() {
    let multiple = Element::new(tag(0x0018, 0x0050), Vr::Ds, Value::from_str("1.5\\2.25"));
    assert_eq!(multiple.decimal_values().unwrap(), vec![1.5, 2.25]);

    let padded = Element::new(
        tag(0x0018, 0x0050),
        Vr::Ds,
        Value::from_bytes(b" 1.5 ".to_vec()),
    );
    assert_eq!(padded.decimal_values().unwrap(), vec![1.5]);

    let exponent = Element::new(tag(0x0018, 0x0050), Vr::Ds, Value::from_str("1.5e3"));
    assert_eq!(exponent.decimal_values().unwrap(), vec![1500.0]);

    // Empty components mean "no value" and are skipped.
    let holes = Element::new(tag(0x0018, 0x0050), Vr::Ds, Value::from_str("1.5\\\\2.25"));
    assert_eq!(holes.decimal_values().unwrap(), vec![1.5, 2.25]);

    let invalid = Element::new(tag(0x0018, 0x0050), Vr::Ds, Value::from_str("abc"));
    assert!(matches!(
        invalid.decimal_values(),
        Err(Error::InvalidNumber(text)) if text == "abc"
    ));

    let wrong = Element::new(tag(0x0018, 0x0050), Vr::Us, Value::from_u16s(&[1]));
    assert!(matches!(
        wrong.decimal_values(),
        Err(Error::WrongVr {
            actual: Vr::Us,
            expected: "DS"
        })
    ));
}

#[test]
fn integer_string_parsing() {
    let values = Element::new(tag(0x0020, 0x0013), Vr::Is, Value::from_str("3\\-2"));
    assert_eq!(values.integer_values().unwrap(), vec![3, -2]);

    let padded = Element::new(
        tag(0x0020, 0x0013),
        Vr::Is,
        Value::from_bytes(b" 3 ".to_vec()),
    );
    assert_eq!(padded.integer_values().unwrap(), vec![3]);

    let invalid = Element::new(tag(0x0020, 0x0013), Vr::Is, Value::from_str("3.5"));
    assert!(matches!(
        invalid.integer_values(),
        Err(Error::InvalidNumber(text)) if text == "3.5"
    ));

    let wrong = Element::new(tag(0x0020, 0x0013), Vr::Us, Value::from_u16s(&[3]));
    assert!(matches!(
        wrong.integer_values(),
        Err(Error::WrongVr {
            actual: Vr::Us,
            expected: "IS"
        })
    ));
}

#[test]
fn empty_values_across_the_vr_table() {
    for vr in Vr::ALL {
        let element = Element::new(tag(0x0011, 0x0100), vr, Value::Empty);
        assert!(element.is_empty(), "{vr}");
        assert_eq!(element.byte_len(), 0, "{vr}");
        assert_eq!(element.raw_value(), &[] as &[u8], "{vr}");
    }
}

#[test]
fn long_text_values_are_not_truncated() {
    let text = "A".repeat(70_000);
    let element = Element::new(tag(0x0040, 0xA160), Vr::Ut, Value::from_str(&text));
    assert_eq!(element.string().unwrap().len(), 70_000);
    assert_eq!(element.byte_len(), 70_000);
}

#[test]
fn raw_value_is_empty_for_structured_values() {
    let bytes_element = Element::new(
        tag(0x7FE0, 0x0010),
        Vr::Ob,
        Value::from_bytes(vec![1, 2, 3]),
    );
    assert_eq!(bytes_element.byte_len(), 3);
    assert_eq!(bytes_element.raw_value(), &[1, 2, 3]);

    let sequence = Element::new(
        tag(0x0008, 0x1110),
        Vr::Sq,
        Value::sequence(vec![DataSet::new()]),
    );
    assert_eq!(sequence.byte_len(), 0);
    assert_eq!(sequence.raw_value(), &[] as &[u8]);
    assert!(sequence.is_sequence());
    assert!(!sequence.is_empty());

    let fragments = Element::new(
        tag(0x7FE0, 0x0010),
        Vr::Ob,
        Value::Fragments(vec![vec![1, 2], vec![3]]),
    );
    assert_eq!(fragments.byte_len(), 0);
    assert_eq!(fragments.raw_value(), &[] as &[u8]);
    assert!(fragments.value.is_fragments());
}

#[test]
fn value_lengths_and_emptiness() {
    assert_eq!(Value::from_str("AB").len(), 2);
    assert_eq!(Value::sequence(vec![DataSet::new()]).len(), 1);
    assert_eq!(Value::sequence(Vec::new()).len(), 0);
    assert_eq!(Value::Fragments(vec![vec![1, 2], vec![3, 4, 5]]).len(), 5);
    assert!(!Value::Fragments(vec![vec![1]]).is_empty());
    assert!(Value::Fragments(Vec::new()).is_empty());
    assert!(Value::from_bytes(Vec::new()).is_empty());
    assert!(Value::empty().is_empty());
}

#[test]
fn value_constructors_encode_little_endian() {
    assert_eq!(Value::from_u16s(&[1, 2]).as_bytes().unwrap(), &[1, 0, 2, 0]);
    assert_eq!(
        Value::from_i32s(&[-1]).as_bytes().unwrap(),
        &[0xFF, 0xFF, 0xFF, 0xFF]
    );
    assert_eq!(
        Value::from_strings(&["ORIGINAL", "PRIMARY"])
            .as_bytes()
            .unwrap(),
        b"ORIGINAL\\PRIMARY"
    );
    assert_eq!(
        Value::from_tags(&[Tag::new(0x0010, 0x0010)])
            .as_bytes()
            .unwrap(),
        &[0x10, 0x00, 0x10, 0x00]
    );
    assert_eq!(Value::from_f64s(&[1.0]).as_bytes().unwrap().len(), 8);
}

/// Mirrors pydicom's `test_empty_binary_values`: an empty value decodes to no components.
#[test]
fn empty_binary_values_have_no_components() {
    let ul = Element::new(tag(0x0008, 0x0000), Vr::Ul, Value::Empty);
    assert_eq!(ul.u32s(Endian::Little).unwrap(), Vec::<u32>::new());

    let us = Element::new(tag(0x0028, 0x0010), Vr::Us, Value::Empty);
    assert_eq!(us.u16s(Endian::Little).unwrap(), Vec::<u16>::new());
    assert_eq!(us.i16s(Endian::Little).unwrap(), Vec::<i16>::new());

    let at = Element::new(tag(0x0028, 0x0009), Vr::At, Value::Empty);
    assert_eq!(at.tags(Endian::Little).unwrap(), Vec::<Tag>::new());

    let fd = Element::new(tag(0x0028, 0x0119), Vr::Fd, Value::Empty);
    assert_eq!(fd.f64s(Endian::Little).unwrap(), Vec::<f64>::new());
}

/// Mirrors pydicom's `test_is_empty_sequence` and `test_empty_sequence_is_handled_as_array`.
#[test]
fn empty_sequence_element_is_empty_until_it_gets_items() {
    let mut sequence = Element::new(tag(0x0040, 0x0275), Vr::Sq, Value::sequence(Vec::new()));
    assert_eq!(sequence.vr, Vr::Sq);
    assert!(sequence.is_empty());
    assert!(sequence.as_sequence().unwrap().is_empty());

    sequence.as_sequence_mut().unwrap().push(DataSet::new());
    assert!(!sequence.is_empty());
    assert_eq!(sequence.as_sequence().unwrap().len(), 1);
}

#[test]
fn sequence_accessors_enforce_the_vr() {
    let mut item = DataSet::new();
    item.set_string(tag(0x0008, 0x0100), Vr::Sh, "ABC");
    let sequence = Element::new(tag(0x0008, 0x1110), Vr::Sq, Value::sequence(vec![item]));
    assert_eq!(sequence.as_sequence().unwrap().len(), 1);
    assert_eq!(
        sequence.as_sequence().unwrap()[0]
            .string(tag(0x0008, 0x0100))
            .unwrap(),
        "ABC"
    );

    let text = Element::new(tag(0x0010, 0x0010), Vr::Pn, Value::from_str("Doe^John"));
    assert!(matches!(
        text.as_sequence(),
        Err(Error::NotASequence(Vr::Pn))
    ));

    // An empty sequence value is promoted on mutable access, like pydicom's empty `Sequence`.
    let mut empty_sq = Element::new(tag(0x0008, 0x1110), Vr::Sq, Value::Empty);
    let items = empty_sq.as_sequence_mut().unwrap();
    items.push(DataSet::new());
    assert_eq!(empty_sq.as_sequence().unwrap().len(), 1);
}
