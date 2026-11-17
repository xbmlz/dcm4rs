//! Byte fixtures shared by the encoding tests.
//!
//! pydicom's tests build sample files with `dcmwrite` and hand-written byte arrays; this module
//! plays that role. Every helper emits canonical bytes (ascending tags, even lengths, no group
//! length elements) so byte-level assertions match what [`dcm4rs::DataSet::write`] produces.

#![allow(dead_code)]

use dcm4rs::{DataSet, Tag, TransferSyntax, Vr};

pub const UNDEFINED: u32 = 0xFFFF_FFFF;
pub const PREAMBLE_LENGTH: usize = 128;
pub const ITEM: Tag = Tag::new(0xFFFE, 0xE000);
pub const ITEM_DELIMITER: Tag = Tag::new(0xFFFE, 0xE00D);
pub const SEQUENCE_DELIMITER: Tag = Tag::new(0xFFFE, 0xE0DD);

pub fn tag(group: u16, element: u16) -> Tag {
    Tag::new(group, element)
}

fn vr(text: &str) -> Vr {
    text.parse()
        .unwrap_or_else(|_| panic!("fixture uses an unknown VR: {text:?}"))
}

pub fn tag_le(tag: Tag) -> [u8; 4] {
    let mut bytes = [0u8; 4];
    bytes[0..2].copy_from_slice(&tag.group().to_le_bytes());
    bytes[2..4].copy_from_slice(&tag.element().to_le_bytes());
    bytes
}

pub fn tag_be(tag: Tag) -> [u8; 4] {
    let mut bytes = [0u8; 4];
    bytes[0..2].copy_from_slice(&tag.group().to_be_bytes());
    bytes[2..4].copy_from_slice(&tag.element().to_be_bytes());
    bytes
}

/// Explicit VR little endian element with a defined length.
pub fn element_le(vr_text: &str, tag: Tag, body: &[u8]) -> Vec<u8> {
    let vr = vr(vr_text);
    let mut out = tag_le(tag).to_vec();
    out.extend_from_slice(vr.as_str().as_bytes());
    if vr.uses_long_length() {
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    } else {
        out.extend_from_slice(&(body.len() as u16).to_le_bytes());
    }
    out.extend_from_slice(body);
    out
}

/// Explicit VR little endian element with an undefined length; `body` carries the item stream and
/// its delimiter.
pub fn element_le_undefined(vr_text: &str, tag: Tag, body: &[u8]) -> Vec<u8> {
    let vr = vr(vr_text);
    let mut out = tag_le(tag).to_vec();
    out.extend_from_slice(vr.as_str().as_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&UNDEFINED.to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// Explicit VR big endian element with a defined length.
pub fn element_be(vr_text: &str, tag: Tag, body: &[u8]) -> Vec<u8> {
    let vr = vr(vr_text);
    let mut out = tag_be(tag).to_vec();
    out.extend_from_slice(vr.as_str().as_bytes());
    if vr.uses_long_length() {
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    } else {
        out.extend_from_slice(&(body.len() as u16).to_be_bytes());
    }
    out.extend_from_slice(body);
    out
}

/// Implicit VR little endian element.
pub fn element_implicit(tag: Tag, body: &[u8]) -> Vec<u8> {
    let mut out = tag_le(tag).to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out
}

pub fn item_le(length: u32) -> Vec<u8> {
    let mut out = tag_le(ITEM).to_vec();
    out.extend_from_slice(&length.to_le_bytes());
    out
}

pub fn item_be(length: u32) -> Vec<u8> {
    let mut out = tag_be(ITEM).to_vec();
    out.extend_from_slice(&length.to_be_bytes());
    out
}

pub fn item_undefined_le() -> Vec<u8> {
    item_le(UNDEFINED)
}

pub fn delimiter_le(tag: Tag) -> Vec<u8> {
    let mut out = tag_le(tag).to_vec();
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

pub fn delimiter_be(tag: Tag) -> Vec<u8> {
    let mut out = tag_be(tag).to_vec();
    out.extend_from_slice(&0u32.to_be_bytes());
    out
}

/// A transfer syntax UID value padded to an even length with NUL, as `UI` requires.
pub fn uid_body(uid: &str) -> Vec<u8> {
    let mut bytes = uid.as_bytes().to_vec();
    if bytes.len() % 2 == 1 {
        bytes.push(0);
    }
    bytes
}

/// A file meta element declaring `syntax` as `(0002,0010)`.
pub fn meta_transfer_syntax(syntax: TransferSyntax) -> Vec<u8> {
    element_le("UI", tag(0x0002, 0x0010), &uid_body(syntax.uid()))
}

/// `(0002,0001)` File Meta Information Version.
pub fn meta_version() -> Vec<u8> {
    element_le("OB", tag(0x0002, 0x0001), &[0, 1])
}

/// A Part 10 file: preamble, `DICM`, the meta bytes, then the data set bytes.
pub fn part10(meta: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; PREAMBLE_LENGTH];
    out.extend_from_slice(b"DICM");
    out.extend_from_slice(meta);
    out.extend_from_slice(body);
    out
}

/// Write, read back and compare. Only valid for data sets whose values already have the padding
/// the writer would add: the writer pads odd-length values, so those come back one byte longer.
pub fn assert_round_trip(set: &DataSet, syntax: TransferSyntax) {
    let bytes = set.write(syntax).unwrap();
    let decoded = DataSet::read(&bytes, syntax).unwrap();
    assert_eq!(&decoded, set, "round trip changed the data set");
    assert_eq!(
        decoded.write(syntax).unwrap(),
        bytes,
        "canonical output must be a fixed point"
    );
}
