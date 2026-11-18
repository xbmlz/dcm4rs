//! Ports of `pydicom/tests/test_valuerep.py` for the VRs with internal structure.
//!
//! Each case names the pydicom test it comes from.

use dcm4rs::dictionary::tags;
use dcm4rs::{DataSet, Date, DateTime, DecimalString, IntegerString, PersonName, Time, Vr};

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
    assert!(ds.date(tags::PATIENT_NAME).is_err());
    assert!(ds.time(tags::PATIENT_NAME).is_err());
    assert!(ds.date_time(tags::PATIENT_NAME).is_err());
    assert!(ds.person_name(tags::PATIENT_NAME).is_ok());
}

/// pydicom: `TestTM::test_new_str_conversion`
#[test]
fn time_reads_reduced_precision() {
    let hour = Time::parse("21").expect("time");
    assert_eq!((hour.hour(), hour.minute(), hour.second()), (21, 0, 0));

    let minute = Time::parse("2122").expect("time");
    assert_eq!(
        (minute.hour(), minute.minute(), minute.second()),
        (21, 22, 0)
    );

    let second = Time::parse("212223").expect("time");
    assert_eq!(second.second(), 23);
    assert_eq!(second.to_string(), "212223");
}

/// pydicom: `TestTM::test_str_and_repr` and `test_time_behavior`
#[test]
fn time_keeps_its_fraction() {
    let time = Time::parse("212223.1234").expect("time");
    assert_eq!(time.microsecond(), 123_400);
    assert_eq!(time.text(), "212223.1234");

    let six_digits = Time::parse("010203.123456").expect("time");
    assert_eq!(six_digits.microsecond(), 123_456);
    // PS3.5 allows padding spaces around the value.
    let padded = Time::parse("  212223  ").expect("time");
    assert_eq!(padded.text(), "212223");
}

/// pydicom: `TestTM::test_new_str_conversion` — non-conformant values
#[test]
fn time_rejects_non_conformant_values() {
    for invalid in ["24", "2360", "235", "", "abc", "23:59", "212223.1234567"] {
        assert!(Time::parse(invalid).is_err(), "{invalid:?} should fail");
    }
}

/// pydicom: `TestTM::test_new_str_conversion` — the leap second
///
/// pydicom lowers `235960` to `23:59:59` with a warning because Python's `time` cannot hold
/// 60; PS3.5 allows it, so it is kept here.
#[test]
fn time_keeps_a_leap_second() {
    let time = Time::parse("235960").expect("leap second");
    assert_eq!(time.second(), 60);
    assert_eq!(time.text(), "235960");
}

#[test]
fn time_orders_by_the_clock() {
    let earlier = Time::parse("010203").expect("time");
    let later = Time::parse("010203.123456").expect("time");
    assert!(earlier < later);
    let next_second = Time::parse("010204").expect("time");
    assert!(later < next_second);
}

/// pydicom: `TestDA::test_str_and_repr`
#[test]
fn date_parses_both_spellings() {
    let plain = Date::parse("19111213").expect("date");
    let dotted = Date::parse("1911.12.13").expect("date");
    assert_eq!((plain.year(), plain.month(), plain.day()), (1911, 12, 13));
    // Equality is by the date, not by the characters: pydicom compares against `date` too.
    assert_eq!(plain, dotted);
    assert_eq!(plain.text(), "19111213");
    assert_eq!(dotted.text(), "1911.12.13");
}

/// pydicom: `TestDA::test_date_behavior` — impossible dates are refused, as Python's `date`
/// refuses them
#[test]
fn date_rejects_impossible_dates() {
    for invalid in ["20010230", "20011301", "00000203", "20210132", "19111.213"] {
        assert!(Date::parse(invalid).is_err(), "{invalid:?} should fail");
    }
    assert!(Date::parse("20000229").is_ok());
    assert!(Date::parse("19000229").is_err());
}

/// pydicom: `TestDT::test_new_obj_conversion` — a `DT` may stop anywhere
#[test]
fn date_time_reads_partial_values() {
    let year_only = DateTime::parse("1001").expect("date-time");
    assert_eq!(
        (year_only.year(), year_only.month(), year_only.day()),
        (1001, 1, 1)
    );
    assert!(!year_only.has_time());

    let date_only = DateTime::parse("10010203").expect("date-time");
    assert_eq!(date_only.day(), 3);
    assert!(!date_only.has_time());

    let full = DateTime::parse("19111213212123").expect("date-time");
    assert_eq!((full.hour(), full.minute(), full.second()), (21, 21, 23));
    assert!(full.has_time());
}

/// pydicom: `TestDT::test_str_and_repr` — the UTC offset suffix
#[test]
fn date_time_reads_the_utc_offset() {
    let west = DateTime::parse("19111213212123-0630").expect("date-time");
    assert_eq!(west.offset_minutes(), Some(-390));

    let east = DateTime::parse("20220102080907.123456+0600").expect("date-time");
    assert_eq!(east.microsecond(), 123_456);
    assert_eq!(east.offset_minutes(), Some(360));
    assert_eq!(east.text(), "20220102080907.123456+0600");

    let none = DateTime::parse("20220101").expect("date-time");
    assert_eq!(none.offset_minutes(), None);
}

/// pydicom: `TestDT::test_new_str_conversion` — the leap second
#[test]
fn date_time_keeps_a_leap_second() {
    let value = DateTime::parse("20010101235960").expect("leap second");
    assert_eq!(value.second(), 60);
    assert_eq!(value.text(), "20010101235960");
}

/// pydicom: `TestDT::test_new_str_conversion` — non-conformant values, and the length limit
#[test]
fn date_time_rejects_non_conformant_values() {
    for invalid in [
        "a2000,00,00",
        "2001.5",
        "202213",
        "20221123+2400",
        "19111213+06300",
    ] {
        assert!(DateTime::parse(invalid).is_err(), "{invalid:?} should fail");
    }
    // PS3.5 caps a date-time at 26 characters, which is checked before parsing.
    assert!(DateTime::parse(&format!("{}99", "2".repeat(25))).is_err());
}

#[test]
fn date_time_equality_ignores_missing_zeros() {
    let partial = DateTime::parse("20220101").expect("date-time");
    let full = DateTime::parse("20220101000000").expect("date-time");
    assert_eq!(partial, full);
}

#[test]
fn dataset_reads_temporal_values() {
    let mut ds = DataSet::new();
    ds.set_string(tags::STUDY_DATE, Vr::Da, "20110305");
    ds.set_string(tags::STUDY_TIME, Vr::Tm, "134455.123");
    ds.set_string(
        tags::ACQUISITION_DATE_TIME,
        Vr::Dt,
        "20110305134455.123-0500",
    );

    let date = ds.date(tags::STUDY_DATE).expect("study date");
    assert_eq!((date.year(), date.month(), date.day()), (2011, 3, 5));

    let time = ds.time(tags::STUDY_TIME).expect("study time");
    assert_eq!(time.microsecond(), 123_000);

    let date_time = ds
        .date_time(tags::ACQUISITION_DATE_TIME)
        .expect("acquisition");
    assert_eq!(date_time.offset_minutes(), Some(-300));
    assert_eq!(date_time.second(), 55);
}

#[test]
fn temporal_values_build_from_parts() {
    let date = Date::from_ymd(2011, 3, 5).expect("date");
    assert_eq!(date.text(), "20110305");
    assert!(Date::from_ymd(2011, 2, 30).is_err());

    let time = Time::from_hms(13, 44, 55).expect("time");
    assert_eq!(time.text(), "134455");
    let time = Time::from_fields(13, 44, 55, 123_000).expect("time");
    assert_eq!(time.text(), "134455.123000");
    // A leap second is allowed, an hour of 24 is not.
    assert!(Time::from_hms(23, 59, 60).is_ok());
    assert!(Time::from_hms(24, 0, 0).is_err());

    let date_time = DateTime::from_ymd_hms(2011, 3, 5, 13, 44, 55).expect("date-time");
    assert_eq!(date_time.text(), "20110305134455");
    assert!(DateTime::from_ymd_hms(2011, 2, 30, 0, 0, 0).is_err());
}

#[test]
fn temporal_values_parse_through_from_str() {
    let date: Date = "20110305".parse().expect("date");
    assert_eq!(date.year(), 2011);

    let time: Time = "134455".parse().expect("time");
    assert_eq!(time.hour(), 13);

    let date_time: DateTime = "20110305134455".parse().expect("date-time");
    assert_eq!(date_time.year(), 2011);
}

#[test]
fn temporal_values_display_their_text() {
    let date = Date::parse("1911.12.13").expect("date");
    assert_eq!(date.to_string(), "1911.12.13");

    let time = Time::parse(" 212223 ").expect("time");
    assert_eq!(time.to_string(), "212223");

    let date_time = DateTime::parse("19111213212123").expect("date-time");
    assert_eq!(date_time.to_string(), "19111213212123");
}

#[test]
fn temporal_values_reject_an_empty_fraction() {
    assert!(Time::parse("212223.").is_err());
    assert!(DateTime::parse("20220101000000.").is_err());
}
