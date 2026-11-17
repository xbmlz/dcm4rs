//! Character set decoding, mirroring pydicom's `tests/test_charset.py`.
//!
//! Covered: the default repertoire (ISO_IR 6 / ISO_IR 100 as Latin-1), ISO_IR 192 (UTF-8),
//! the ISO-8859 family, GBK/GB18030, ISO_IR 13 (Shift_JIS), ISO_IR 166 (Thai) and the complete
//! ISO 2022 code extensions: single-byte G1 designations, JIS X 0208 (IR 87), JIS X 0212
//! (IR 159), GB2312 (IR 58) and EUC-KR (IR 149). Items inherit the parent character set and
//! may override it.
//!
//! Divergences from pydicom: pydicom converts text at read time and re-encodes on write; we keep
//! the raw bytes and decode on access, so a file written back is byte-identical (after padding).
//! An unknown or undeclared escape sequence is reported as `Error::InvalidText`; pydicom warns
//! and falls back to the default encoding, leaving the escape bytes in the text.

mod common;

use dcm4rs::{Charset, DataSet, Error, Tag, TransferSyntax};

use common::{element_le, item_le, tag};

const EXPLICIT_LE: TransferSyntax = TransferSyntax::ExplicitVrLittleEndian;

/// Latin-1 "Müller^Han"; even length, so the writer does not pad it.
const LATIN1_NAME: &[u8] = b"M\xFCller^Han";

fn read_set(charset: &str, vr: &str, element_tag: Tag, body: &[u8]) -> DataSet {
    let mut bytes = element_le("CS", tag(0x0008, 0x0005), charset.as_bytes());
    bytes.extend(element_le(vr, element_tag, body));
    DataSet::read(&bytes, EXPLICIT_LE).unwrap()
}

#[test]
fn charset_names_map_to_encodings() {
    assert_eq!(Charset::Utf8.name(), "ISO_IR 192");
    assert_eq!(Charset::Iso8859_2.to_string(), "ISO_IR 101");
    assert_eq!(
        Charset::from_specific_character_set("ISO 2022 IR 6\\ISO 2022 IR 87")
            .unwrap()
            .name(),
        "ISO 2022 IR 6\\ISO 2022 IR 87"
    );
    assert_eq!(
        Charset::from_specific_character_set(
            "ISO 2022 IR 6\\ISO 2022 IR 13\\ISO 2022 IR 87\\ISO 2022 IR 159"
        )
        .unwrap()
        .name(),
        "ISO 2022 IR 6\\ISO 2022 IR 13\\ISO 2022 IR 87\\ISO 2022 IR 159"
    );
    assert_eq!(
        Charset::from_specific_character_set("ISO_IR 192").unwrap(),
        Charset::Utf8
    );
    assert_eq!(
        Charset::from_specific_character_set("ISO_IR 100").unwrap(),
        Charset::Latin1
    );
    assert_eq!(
        Charset::from_specific_character_set("").unwrap(),
        Charset::Latin1
    );
    assert_eq!(
        Charset::from_specific_character_set("ISO 2022 GBK").unwrap(),
        Charset::Gbk
    );
    assert_eq!(
        Charset::from_specific_character_set("ISO_IR 999"),
        Err(Error::UnsupportedCharacterSet("ISO_IR 999".to_owned()))
    );
    // More than four sets is not a declaration PS3.5 allows us to keep.
    assert!(matches!(
        Charset::from_specific_character_set(
            "ISO 2022 IR 6\\ISO 2022 IR 100\\ISO 2022 IR 101\\ISO 2022 IR 109\\ISO 2022 IR 110"
        ),
        Err(Error::UnsupportedCharacterSet(_))
    ));
}

/// Mirrors pydicom's default: with no `(0008,0005)` the text is decoded as ISO-8859-1
/// (`default_encoding = "iso8859"`, pydicom/charset.py:20-30).
#[test]
fn absent_charset_decodes_as_latin1() {
    let bytes = element_le("PN", tag(0x0010, 0x0010), LATIN1_NAME);
    let set = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    assert_eq!(set.charset().unwrap(), Charset::Latin1);
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Müller^Han");
}

#[test]
fn iso_ir_100_decodes_latin1() {
    let set = read_set("ISO_IR 100", "PN", tag(0x0010, 0x0010), LATIN1_NAME);
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Müller^Han");
}

#[test]
fn iso_ir_192_decodes_utf8() {
    let body = "Müller^Han".as_bytes();
    let set = read_set("ISO_IR 192", "PN", tag(0x0010, 0x0010), body);
    assert_eq!(set.charset().unwrap(), Charset::Utf8);
    assert_eq!(set.string(tag(0x0010, 0x0010)).unwrap(), "Müller^Han");
}

/// GBK and GB18030 bytes for "中文": `D6 D0 CE C4`.
#[test]
fn gbk_and_gb18030_decode_chinese() {
    for charset in ["GBK", "GB18030"] {
        let set = read_set(
            charset,
            "PN",
            tag(0x0010, 0x0010),
            &[0xD6, 0xD0, 0xCE, 0xC4],
        );
        assert_eq!(
            set.string(tag(0x0010, 0x0010)).unwrap(),
            "中文",
            "{charset}"
        );
    }
}

/// Shift_JIS bytes `82 A0` are "あ".
#[test]
fn iso_ir_13_decodes_shift_jis() {
    let set = read_set("ISO_IR 13", "SH", tag(0x0008, 0x0100), &[0x82, 0xA0]);
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "あ");
}

/// TIS-620 byte `A1` is "ก" (U+0E01).
#[test]
fn iso_ir_166_decodes_thai() {
    let set = read_set("ISO_IR 166", "LO", tag(0x0008, 0x0100), &[0xA1]);
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "ก");
}

/// ISO-2022-JP: `ESC $ B 24 22 ESC ( B` is "あ" (JIS X 0208 row 4, cell 2).
#[test]
fn japanese_iso_2022_decodes() {
    let body = [0x1B, 0x24, 0x42, 0x24, 0x22, 0x1B, 0x28, 0x42];
    let set = read_set(
        "ISO 2022 IR 6\\ISO 2022 IR 87",
        "LO",
        tag(0x0008, 0x0100),
        &body,
    );
    assert!(matches!(set.charset().unwrap(), Charset::Iso2022(_)));
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "あ");
}

/// `ESC - A` designates ISO-8859-1 to G1, so `FC` reads as "ü".
#[test]
fn single_byte_code_extension_decodes() {
    let body = [0x1B, b'-', b'A', 0xFC];
    let set = read_set(
        "ISO 2022 IR 6\\ISO 2022 IR 100",
        "LO",
        tag(0x0008, 0x0100),
        &body,
    );
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "ü");
}

/// An escape sequence for a set the data set did not declare is rejected (PS3.5 6.1.2.4).
#[test]
fn undeclared_escape_sequence_is_reported() {
    let body = [0x1B, b'-', b'A', 0xFC];
    let set = read_set("ISO 2022 IR 6", "LO", tag(0x0008, 0x0100), &body);
    assert!(matches!(
        set.string(tag(0x0008, 0x0100)),
        Err(Error::InvalidText(_))
    ));
}

#[test]
fn unknown_escape_sequence_is_reported() {
    let body = [0x1B, b'-', b'Z', 0x40];
    let set = read_set("ISO 2022 IR 6", "LO", tag(0x0008, 0x0100), &body);
    assert!(matches!(
        set.string(tag(0x0008, 0x0100)),
        Err(Error::InvalidText(_))
    ));
}

/// `ESC $ ( D 6D 4E` is "齩" (JIS X 0212), pydicom's Japanese multi-byte example.
#[test]
fn jis_x_0212_decodes() {
    let body = [0x1B, b'$', b'(', b'D', 0x6D, 0x4E];
    let set = read_set(
        "ISO 2022 IR 6\\ISO 2022 IR 159",
        "LO",
        tag(0x0008, 0x0100),
        &body,
    );
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "齩");
}

/// A data set whose first set is JIS X 0212 decodes both escaped and bare bytes.
#[test]
fn jis_x_0212_alone_decodes() {
    let escaped = [0x1B, b'$', b'(', b'D', 0x6D, 0x4E];
    let set = read_set("ISO 2022 IR 159", "LO", tag(0x0008, 0x0100), &escaped);
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "齩");

    let bare = [0x6D, 0x4E];
    let set = read_set("ISO 2022 IR 159", "LO", tag(0x0008, 0x0100), &bare);
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "齩");
}

/// The Japanese multi-byte example from pydicom: JIS X 0201 Roman and katakana, JIS X 0208 and
/// JIS X 0212 in one value.
#[test]
fn japanese_code_extensions_decode() {
    let body = [
        0x1B, b'$', b'B', 0x24, 0x22, // あ (JIS X 0208 row 4, cell 2)
        0x1B, b'(', b'J', b'a', // a (JIS X 0201 Roman)
        0x1B, b')', b'I', 0xB1, // ｱ (JIS X 0201 katakana)
        0x1B, b'$', b'B', 0x25, 0x22, // ア (JIS X 0208 row 5, cell 2)
        0x1B, b'$', b'(', b'D', 0x6D, 0x4E, // 齩 (JIS X 0212)
        0x1B, b'(', b'J', // back to JIS X 0201 Roman
    ];
    let charset = "ISO 2022 IR 6\\ISO 2022 IR 13\\ISO 2022 IR 87\\ISO 2022 IR 159";
    let set = read_set(charset, "LO", tag(0x0008, 0x0100), &body);
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "あaｱア齩");
}

/// PS3.5 Annex K.2-1 for "Zhang^XiaoDong=张^小东=": the designation escape is stripped and the
/// delimiter after "张" resets the fragment to the default repertoire.
#[test]
fn gb2312_code_extension_decodes() {
    let body = b"Zhang^XiaoDong=\x1b$)A\xd5\xc5^\x1b$)A\xd0\xa1\xb6\xab=";
    let set = read_set("\\ISO 2022 IR 58", "PN", tag(0x0010, 0x0010), body);
    assert_eq!(
        set.string(tag(0x0010, 0x0010)).unwrap(),
        "Zhang^XiaoDong=张^小东="
    );
}

#[test]
fn gb2312_alone_decodes() {
    let set = read_set("ISO 2022 IR 58", "LO", tag(0x0008, 0x0100), &[0xD5, 0xC5]);
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "张");
}

/// `ESC $ ) C C7 D1` is "한" (EUC-KR).
#[test]
fn euc_kr_code_extension_decodes() {
    let body = [0x1B, b'$', b')', b'C', 0xC7, 0xD1];
    let set = read_set(
        "ISO 2022 IR 6\\ISO 2022 IR 149",
        "LO",
        tag(0x0008, 0x0100),
        &body,
    );
    assert_eq!(set.string(tag(0x0008, 0x0100)).unwrap(), "한");
}

/// JIS X 0212 bytes outside the 94x94 grid are reported, not decoded as replacement characters.
#[test]
fn invalid_jis_x_0212_is_reported() {
    let body = [0x1B, b'$', b'(', b'D', 0xC4, 0xE9];
    let set = read_set(
        "ISO 2022 IR 6\\ISO 2022 IR 159",
        "LO",
        tag(0x0008, 0x0100),
        &body,
    );
    assert!(matches!(
        set.string(tag(0x0008, 0x0100)),
        Err(Error::InvalidText(_))
    ));
}

#[test]
fn unknown_charset_is_reported_on_access() {
    let set = read_set("ISO_IR 999", "PN", tag(0x0010, 0x0010), LATIN1_NAME);
    assert_eq!(
        set.charset(),
        Err(Error::UnsupportedCharacterSet("ISO_IR 999".to_owned()))
    );
    assert!(matches!(
        set.string(tag(0x0010, 0x0010)),
        Err(Error::UnsupportedCharacterSet(_))
    ));
}

#[test]
fn invalid_bytes_report_the_encoding() {
    let set = read_set("GB18030", "PN", tag(0x0010, 0x0010), &[0xFF]);
    assert!(matches!(
        set.string(tag(0x0010, 0x0010)),
        Err(Error::InvalidText(ref name)) if name == "GB18030"
    ));
}

/// A sequence item without `(0008,0005)` uses the parent's character set.
#[test]
fn items_inherit_the_parent_charset() {
    let item_body = element_le("PN", tag(0x0010, 0x0010), LATIN1_NAME);
    let mut items = item_le(item_body.len() as u32);
    items.extend_from_slice(&item_body);
    let mut bytes = element_le("CS", tag(0x0008, 0x0005), b"ISO_IR 100");
    bytes.extend(element_le("SQ", tag(0x0040, 0x0275), &items));

    let set = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    let item = &set.sequence(tag(0x0040, 0x0275)).unwrap()[0];
    assert_eq!(item.charset().unwrap(), Charset::Latin1);
    assert_eq!(item.string(tag(0x0010, 0x0010)).unwrap(), "Müller^Han");
}

/// An item that carries its own `(0008,0005)` overrides the parent for itself only.
#[test]
fn items_may_override_the_charset() {
    let mut item_body = element_le("CS", tag(0x0008, 0x0005), b"ISO_IR 192");
    item_body.extend(element_le(
        "PN",
        tag(0x0010, 0x0010),
        "Müller^Han".as_bytes(),
    ));
    let mut items = item_le(item_body.len() as u32);
    items.extend_from_slice(&item_body);

    let mut bytes = element_le("CS", tag(0x0008, 0x0005), b"ISO_IR 100");
    bytes.extend(element_le("SQ", tag(0x0040, 0x0275), &items));
    let set = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    assert_eq!(set.charset().unwrap(), Charset::Latin1);

    let item = &set.sequence(tag(0x0040, 0x0275)).unwrap()[0];
    assert_eq!(item.charset().unwrap(), Charset::Utf8);
    assert_eq!(item.string(tag(0x0010, 0x0010)).unwrap(), "Müller^Han");
}

/// Both values of a multi-valued element are decoded with the data set's character set.
#[test]
fn multi_values_are_decoded_with_the_charset() {
    let body = b"M\xFCller^Han\\Doe^John";
    let set = read_set("ISO_IR 100", "PN", tag(0x0010, 0x0010), body);
    let values = set.strings(tag(0x0010, 0x0010)).unwrap();
    let values: Vec<&str> = values.iter().map(|value| value.as_ref()).collect();
    assert_eq!(values, vec!["Müller^Han", "Doe^John"]);
}

/// The character set affects decoding only; the stored and written bytes stay untouched.
#[test]
fn writing_keeps_the_bytes_and_the_charset() {
    let set = read_set("ISO_IR 100", "PN", tag(0x0010, 0x0010), LATIN1_NAME);
    let bytes = set.write(EXPLICIT_LE).unwrap();
    assert!(bytes
        .windows(LATIN1_NAME.len())
        .any(|window| window == LATIN1_NAME));

    let decoded = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    assert_eq!(decoded, set);
    assert_eq!(decoded.string(tag(0x0010, 0x0010)).unwrap(), "Müller^Han");
}

/// A defined-length empty character set element is the default repertoire.
#[test]
fn empty_charset_element_is_latin1() {
    let mut bytes = element_le("CS", tag(0x0008, 0x0005), &[]);
    bytes.extend(element_le("PN", tag(0x0010, 0x0010), LATIN1_NAME));
    let set = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    assert_eq!(set.charset().unwrap(), Charset::Latin1);
}

/// Round-trips a sequence whose item inherits the parent charset.
#[test]
fn inherited_charset_survives_a_round_trip() {
    let item_body = element_le("PN", tag(0x0010, 0x0010), LATIN1_NAME);
    let mut items = item_le(item_body.len() as u32);
    items.extend_from_slice(&item_body);
    let mut bytes = element_le("CS", tag(0x0008, 0x0005), b"ISO_IR 100");
    bytes.extend(element_le("SQ", tag(0x0040, 0x0275), &items));

    let set = DataSet::read(&bytes, EXPLICIT_LE).unwrap();
    let written = set.write(EXPLICIT_LE).unwrap();
    assert_eq!(DataSet::read(&written, EXPLICIT_LE).unwrap(), set);
    assert_eq!(
        set.sequence(tag(0x0040, 0x0275)).unwrap()[0]
            .string(tag(0x0010, 0x0010))
            .unwrap(),
        "Müller^Han"
    );
}
