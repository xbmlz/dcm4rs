//! Ports of `pydicom/tests/test_valuerep.py` for the VRs with internal structure.
//!
//! Each case names the pydicom test it comes from. The date/time VRs (`DA`, `DT`, `TM`) are not
//! covered yet: their types do not exist here.

use dcm4rs::dictionary::tags;
use dcm4rs::{DataSet, DecimalString, IntegerString, PersonName, Vr};

/// pydicom: `TestPersonName::test_last_first`
#[test]
fn person_name_family_and_given() {
    let name = PersonName::parse("Family^Given").expect("person name");
    assert_eq!(name.alphabetic(), "Family^Given");
    assert_eq!(name.family_name(), "Family");
    assert_eq!(name.given_name(), "Given");
    assert_eq!(name.name_suffix(), "");
    assert_eq!(name.phonetic(), "");
}

/// pydicom: `TestPersonName::test_no_components`
#[test]
fn person_name_empty_value() {
    let name = PersonName::parse("").expect("person name");
    assert_eq!(name.alphabetic(), "");
    assert_eq!(name.ideographic(), "");
    assert_eq!(name.phonetic(), "");
    assert!(name.is_empty());
}

/// pydicom: `TestPersonName::test_three_component`
///
/// pydicom keeps the ISO 2022 escape sequences because it is handed encoded bytes; this crate
/// hands the view decoded text, so the ideographic and phonetic groups are characters here.
#[test]
fn person_name_three_groups() {
    let name = PersonName::parse("Hong^Gildong^Andrews=洪^吉洞=홍^길동").expect("person name");
    assert_eq!(name.family_name(), "Hong");
    assert_eq!(name.given_name(), "Gildong");
    assert_eq!(name.middle_name(), "Andrews");
    assert_eq!(name.alphabetic(), "Hong^Gildong^Andrews");
    assert_eq!(name.ideographic(), "洪^吉洞");
    assert_eq!(name.phonetic(), "홍^길동");
}

/// pydicom: `TestPersonName::test_three_component_shift_jis`
///
/// The alphabetic group may be empty while the ideographic one is not, which is what the
/// leading `=` marks.
#[test]
fn person_name_empty_alphabetic_group() {
    let name = PersonName::parse("=山田^太郎=やまだ^たろう").expect("person name");
    assert_eq!(name.alphabetic(), "");
    assert_eq!(name.family_name(), "");
    assert_eq!(name.ideographic(), "山田^太郎");
    assert_eq!(name.phonetic(), "やまだ^たろう");
}

/// pydicom: `TestPersonName::test_trailing_equals_char`
#[test]
fn person_name_drops_trailing_empty_groups() {
    let name = PersonName::parse("Doe^John===").expect("person name");
    assert_eq!(name.groups().len(), 1);
    assert_eq!(name.alphabetic(), "Doe^John");
    // ... and writing it back does not resurrect them.
    assert_eq!(name.to_value(), "Doe^John");
}

/// pydicom: `TestPersonName::test_from_named_components`
#[test]
fn person_name_from_named_components() {
    let name = PersonName::from_named_components("Family", "Given", "", "", "").expect("name");
    assert_eq!(name.alphabetic(), "Family^Given");
    assert_eq!(name.family_name(), "Family");
    assert_eq!(name.given_name(), "Given");

    let with_groups = name
        .with_group(&["洪", "吉洞"])
        .and_then(|name| name.with_group(&["홍", "길동"]))
        .expect("ideographic and phonetic groups");
    assert_eq!(with_groups.to_value(), "Family^Given=洪^吉洞=홍^길동");
}

/// pydicom: `TestPersonName::test_from_named_components` — disallowed delimiters
#[test]
fn person_name_components_reject_delimiters() {
    assert!(PersonName::from_named_components("Fam^ily", "", "", "", "").is_err());
    assert!(PersonName::from_named_components("Fam=ily", "", "", "", "").is_err());
    assert!(PersonName::from_named_components("Fam\\ily", "", "", "", "").is_err());
    // A 65-character component exceeds what PS3.5 allows.
    let long = "x".repeat(65);
    assert!(PersonName::from_named_components(&long, "", "", "", "").is_err());
    let longest = "x".repeat(64);
    assert!(PersonName::from_named_components(&longest, "", "", "", "").is_ok());
}

/// pydicom: `TestDSfloat::test_original_string`
#[test]
fn decimal_string_keeps_the_text() {
    let value = DecimalString::parse("1.00").expect("decimal string");
    assert_eq!(value.text(), "1.00");
    assert_eq!(value.value(), 1.0);
    assert_eq!(value.to_string(), "1.00");

    // Padding is trimmed, as pydicom stores the stripped value.
    let padded = DecimalString::parse("  1.0  ").expect("decimal string");
    assert_eq!(padded.text(), "1.0");

    // Scientific notation is a decimal string too.
    let exponent = DecimalString::parse("1e3").expect("decimal string");
    assert_eq!(exponent.value(), 1000.0);
}

/// pydicom: `TestIsValidDS::test_is_valid_ds*` and `TestDSfloat::test_invalid`
#[test]
fn decimal_string_rejects_malformed_values() {
    for invalid in [
        "", ".", "+", "-", "abc", "1.2.3", "1,5", "1.0a", "0x10", "--1", "1..0",
    ] {
        assert!(
            DecimalString::parse(invalid).is_err(),
            "{invalid:?} should fail"
        );
    }

    for valid in ["0", "1", "-1", "+1", "1.", ".5", "1.0", "-1.0e-3", "1.5E+2"] {
        assert!(
            DecimalString::parse(valid).is_ok(),
            "{valid:?} should parse"
        );
    }

    // PS3.5 caps a decimal string at sixteen characters.
    assert!(DecimalString::parse(&"1".repeat(16)).is_ok());
    assert!(DecimalString::parse(&"1".repeat(17)).is_err());
}

/// pydicom: `TestTruncateFloatForDS`
#[test]
fn decimal_string_from_float_fits_sixteen_characters() {
    for value in [
        0.0,
        1.0,
        -1.5,
        0.125,
        1e-5,
        1e13,
        1.2345678901234567,
        -3.15159265358979,
    ] {
        let formatted = DecimalString::from_f64(value).expect("finite value");
        assert!(
            formatted.text().len() <= 16,
            "{value} formatted to {} characters: {}",
            formatted.text().len(),
            formatted.text()
        );
    }

    assert_eq!(DecimalString::from_f64(1.5).expect("value").text(), "1.5");
    assert_eq!(
        DecimalString::from_f64(-2.25).expect("value").text(),
        "-2.25"
    );
    assert!(DecimalString::from_f64(f64::NAN).is_err());
    assert!(DecimalString::from_f64(f64::INFINITY).is_err());
}

/// pydicom: `TestIS::test_original_string`
#[test]
fn integer_string_keeps_the_text() {
    let value = IntegerString::parse("007").expect("integer string");
    assert_eq!(value.text(), "007");
    assert_eq!(value.value(), 7);

    let signed = IntegerString::parse("+5").expect("integer string");
    assert_eq!(signed.value(), 5);

    let padded = IntegerString::parse("  12  ").expect("integer string");
    assert_eq!(padded.text(), "12");
    assert_eq!(padded.value(), 12);
}

/// pydicom: `TestIS::test_invalid`
#[test]
fn integer_string_rejects_malformed_values() {
    for invalid in ["", "+", "-", "abc", "1.2", "1.0", "1e3", "1 2", "0x10"] {
        assert!(
            IntegerString::parse(invalid).is_err(),
            "{invalid:?} should fail"
        );
    }

    for valid in ["0", "-0", "42", "-42", "+42"] {
        assert!(
            IntegerString::parse(valid).is_ok(),
            "{valid:?} should parse"
        );
    }

    // PS3.5 caps an integer string at twelve characters and the value at 32 bits. Twelve
    // characters only fit inside the range when they are leading zeros.
    assert!(IntegerString::parse("000000000001").is_ok());
    assert!(IntegerString::parse("0000000000001").is_err());
    // Twelve significant digits overflow 32 bits even though the text is short enough.
    assert!(IntegerString::parse("999999999999").is_err());
    assert!(IntegerString::parse("2147483647").is_ok());
    assert!(IntegerString::parse("2147483648").is_err());
    assert!(IntegerString::parse("-2147483648").is_ok());
    assert!(IntegerString::parse("-2147483649").is_err());
}

#[test]
fn integer_string_round_trips_a_number() {
    let value = IntegerString::from_i32(-17);
    assert_eq!(value.text(), "-17");
    let parsed = IntegerString::parse(&value.to_string()).expect("integer string");
    assert_eq!(parsed, value);
}

/// pydicom: `TestDSfloat` / `TestIS` through a `Dataset`
#[test]
fn dataset_reads_typed_values() {
    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_NAME, Vr::Pn, "Doe^John");
    ds.set_string(tags::SLICE_THICKNESS, Vr::Ds, "1.0\\2.50");
    ds.set_string(tags::ECHO_NUMBERS, Vr::Is, "1\\02");

    let name = ds.person_name(tags::PATIENT_NAME).expect("person name");
    assert_eq!(name.family_name(), "Doe");
    assert_eq!(name.given_name(), "John");

    let decimals = ds
        .decimal_strings(tags::SLICE_THICKNESS)
        .expect("decimal strings");
    assert_eq!(decimals.len(), 2);
    assert_eq!(decimals[0].text(), "1.0");
    assert_eq!(decimals[1].text(), "2.50");
    assert_eq!(decimals[1].value(), 2.5);

    let integers = ds
        .integer_strings(tags::ECHO_NUMBERS)
        .expect("integer strings");
    assert_eq!(integers.len(), 2);
    assert_eq!(integers[0].text(), "1");
    assert_eq!(integers[1].text(), "02");
    assert_eq!(integers[1].value(), 2);
}

#[test]
fn dataset_accessors_reject_the_wrong_vr() {
    let mut ds = DataSet::new();
    ds.set_string(tags::PATIENT_NAME, Vr::Pn, "Doe^John");
    assert!(ds.decimal_strings(tags::PATIENT_NAME).is_err());
    assert!(ds.integer_strings(tags::PATIENT_NAME).is_err());
    assert!(ds.person_name(tags::PATIENT_NAME).is_ok());
}
