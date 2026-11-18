//! DICOM JSON model (PS3.18 F.2).
//!
//! A data set becomes a JSON object keyed by eight uppercase hex digits, each value carrying its
//! VR and exactly one of `Value`, `InlineBinary` or `BulkDataURI`. This mirrors pydicom's
//! `DataSet.to_json_dict` and `DataSet.from_json`; the values themselves are decoded to the type
//! the VR names, so a `US` element holds numbers rather than bytes.
//!
//! Two things are deliberately out of reach: `BulkDataURI` is not fetched (reading it would need
//! whatever hosts the payload), and encapsulated pixel data has no JSON form here, because its
//! fragments would be flattened into one opaque blob and never come back as fragments.

use std::fmt::Write as _;

use serde_json::{Map, Number, Value as Json};

use crate::dataset::DataSet;
use crate::element::Element;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::tag::Tag;
use crate::value::Value;
use crate::vr::Vr;

/// The keys a person name may carry, in the order PS3.18 gives them.
const NAME_GROUPS: [&str; 3] = ["Alphabetic", "Ideographic", "Phonetic"];

impl DataSet {
    /// Convert to the PS3.18 F.2 JSON model.
    ///
    /// Numeric VRs are decoded with the data set's own byte order, so the JSON does not depend on
    /// the transfer syntax the values came from.
    pub fn to_json(&self) -> Result<Json> {
        let endian = self.endian();
        let mut object = Map::new();
        for element in self.iter() {
            object.insert(tag_key(element.tag), element_to_json(element, endian)?);
        }
        Ok(Json::Object(object))
    }

    /// Convert from the PS3.18 F.2 JSON model.
    ///
    /// The result holds little-endian bytes: JSON has no byte order, and [`DataSet::new`] is the
    /// native one.
    pub fn from_json(json: &Json) -> Result<Self> {
        let object = json.as_object().ok_or_else(|| {
            Error::InvalidValue("a data set is a JSON object of tags".to_string())
        })?;
        let mut dataset = Self::new();
        for (key, value) in object {
            let tag: Tag = key.parse()?;
            dataset.insert(element_from_json(tag, value)?);
        }
        Ok(dataset)
    }
}

/// `00100010` from a tag.
fn tag_key(tag: Tag) -> String {
    format!("{:04X}{:04X}", tag.group(), tag.element())
}

fn element_to_json(element: &Element, endian: Endian) -> Result<Json> {
    let mut object = Map::new();
    object.insert(
        "vr".to_string(),
        Json::String(element.vr.as_str().to_string()),
    );

    match &element.value {
        Value::Empty => {}
        Value::Fragments(_) => {
            return Err(Error::UnsupportedPixelData(
                "encapsulated pixel data cannot be written as JSON".to_string(),
            ))
        }
        Value::Sequence(items) => {
            let values = items
                .iter()
                .map(DataSet::to_json)
                .collect::<Result<Vec<_>>>()?;
            object.insert("Value".to_string(), Json::Array(values));
        }
        Value::Bytes(_) if is_binary(element.vr) => {
            object.insert(
                "InlineBinary".to_string(),
                Json::String(base64_encode(element.raw_value())),
            );
        }
        Value::Bytes(_) if element.vr == Vr::Pn => {
            object.insert("Value".to_string(), Json::Array(person_names(element)?));
        }
        Value::Bytes(_) if element.vr == Vr::At => {
            let values = element
                .tags(endian)?
                .into_iter()
                .map(|tag| Json::String(tag_key(tag)))
                .collect();
            object.insert("Value".to_string(), Json::Array(values));
        }
        Value::Bytes(_) if element.vr.is_string() => {
            let values = element
                .string_values()?
                .into_iter()
                .map(|text| Json::String(text.to_string()))
                .collect();
            object.insert("Value".to_string(), Json::Array(values));
        }
        Value::Bytes(_) => {
            object.insert("Value".to_string(), Json::Array(numbers(element, endian)?));
        }
    }

    Ok(Json::Object(object))
}

/// Whether the VR carries raw bytes rather than a decoded value.
fn is_binary(vr: Vr) -> bool {
    matches!(
        vr,
        Vr::Ob | Vr::Ow | Vr::Of | Vr::Od | Vr::Ol | Vr::Ov | Vr::Un
    )
}

/// One object per value, holding the component groups the name actually has.
fn person_names(element: &Element) -> Result<Vec<Json>> {
    let mut names = Vec::new();
    for value in element.string_values()? {
        let groups: Vec<&str> = value.split('=').collect();
        let mut name = Map::new();
        for (group, key) in groups.iter().zip(NAME_GROUPS) {
            if !group.is_empty() {
                name.insert(key.to_string(), Json::String((*group).to_string()));
            }
        }
        names.push(Json::Object(name));
    }
    Ok(names)
}

/// Numeric VRs as JSON numbers; every other VR here is a program bug, not user data.
fn numbers(element: &Element, endian: Endian) -> Result<Vec<Json>> {
    let integer = |values: Vec<u64>| {
        values
            .into_iter()
            .map(|n| Json::Number(Number::from(n)))
            .collect()
    };
    let signed = |values: Vec<i64>| {
        values
            .into_iter()
            .map(|n| Json::Number(Number::from(n)))
            .collect()
    };
    let real = |values: Vec<f64>| -> Result<Vec<Json>> {
        values
            .into_iter()
            .map(|n| {
                Number::from_f64(n)
                    .map(Json::Number)
                    .ok_or_else(|| Error::InvalidValue(format!("{n} has no JSON form")))
            })
            .collect()
    };

    let values = match element.vr {
        Vr::Us => integer(element.u16s(endian)?.into_iter().map(u64::from).collect()),
        Vr::Ss => signed(element.i16s(endian)?.into_iter().map(i64::from).collect()),
        Vr::Ul => integer(element.u32s(endian)?.into_iter().map(u64::from).collect()),
        Vr::Sl => signed(element.i32s(endian)?.into_iter().map(i64::from).collect()),
        Vr::Uv => integer(element.u64s(endian)?),
        Vr::Sv => signed(element.i64s(endian)?),
        Vr::Fl => real(element.f32s(endian)?.into_iter().map(f64::from).collect())?,
        Vr::Fd => real(element.f64s(endian)?)?,
        _ => {
            return Err(Error::InvalidValue(format!(
                "VR {} has no JSON number form",
                element.vr.as_str()
            )))
        }
    };
    Ok(values)
}

fn element_from_json(tag: Tag, json: &Json) -> Result<Element> {
    let object = json
        .as_object()
        .ok_or_else(|| Error::InvalidValue(format!("element {tag} is not a JSON object")))?;
    let vr_text = object
        .get("vr")
        .and_then(Json::as_str)
        .ok_or_else(|| Error::InvalidValue(format!("element {tag} has no vr")))?;
    let vr: Vr = vr_text.parse()?;

    if object.contains_key("BulkDataURI") {
        return Err(Error::InvalidValue(format!(
            "element {tag} points at bulk data, which is not fetched"
        )));
    }
    if let Some(inline) = object.get("InlineBinary") {
        let text = inline.as_str().ok_or_else(|| {
            Error::InvalidValue(format!("element {tag} has a non-text InlineBinary"))
        })?;
        return Ok(Element::new(
            tag,
            vr,
            Value::from_bytes(base64_decode(text)?),
        ));
    }
    let Some(values) = object.get("Value") else {
        return Ok(Element::new(tag, vr, Value::Empty));
    };
    let values = values
        .as_array()
        .ok_or_else(|| Error::InvalidValue(format!("element {tag} has a non-array Value")))?;

    let value = if vr.is_sequence() {
        let mut items = Vec::new();
        for item in values {
            items.push(DataSet::from_json(item)?);
        }
        Value::sequence(items)
    } else if vr == Vr::Pn {
        let mut names = Vec::new();
        for item in values {
            names.push(name_from_json(tag, item)?);
        }
        Value::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>())
    } else if vr == Vr::At {
        let mut tags = Vec::new();
        for item in values {
            let text = item
                .as_str()
                .ok_or_else(|| Error::InvalidValue(format!("element {tag} has a non-text tag")))?;
            tags.push(text.parse()?);
        }
        Value::from_tags(&tags)
    } else if vr.is_string() {
        let mut texts = Vec::new();
        for item in values {
            texts.push(item.as_str().ok_or_else(|| {
                Error::InvalidValue(format!("element {tag} has a non-text value"))
            })?);
        }
        Value::from_strings(&texts)
    } else {
        numeric_value(tag, vr, values)?
    };

    Ok(Element::new(tag, vr, value))
}

/// Rebuild one person name from its component-group object.
fn name_from_json(tag: Tag, json: &Json) -> Result<String> {
    let object = json
        .as_object()
        .ok_or_else(|| Error::InvalidValue(format!("element {tag} has a non-object name")))?;
    let mut groups = Vec::new();
    for key in NAME_GROUPS {
        match object.get(key) {
            Some(group) => groups.push(group.as_str().ok_or_else(|| {
                Error::InvalidValue(format!("element {tag} has a non-text {key}"))
            })?),
            None => groups.push(""),
        }
    }
    while groups.last() == Some(&"") {
        groups.pop();
    }
    Ok(groups.join("="))
}

fn numeric_value(tag: Tag, vr: Vr, values: &[Json]) -> Result<Value> {
    let value = match vr {
        Vr::Us | Vr::Ul | Vr::Uv => {
            let mut numbers = Vec::new();
            for item in values {
                numbers.push(item.as_u64().ok_or_else(|| {
                    Error::InvalidValue(format!("element {tag} has a non-numeric value"))
                })?);
            }
            match vr {
                Vr::Us => Value::from_u16s(&narrow_unsigned::<u16>(tag, &numbers)?),
                Vr::Ul => Value::from_u32s(&narrow_unsigned::<u32>(tag, &numbers)?),
                _ => Value::from_u64s(&numbers),
            }
        }
        Vr::Ss | Vr::Sl | Vr::Sv => {
            let mut numbers = Vec::new();
            for item in values {
                numbers.push(integer_of(tag, item)?);
            }
            match vr {
                Vr::Ss => Value::from_i16s(&narrow_signed::<i16>(tag, &numbers)?),
                Vr::Sl => Value::from_i32s(&narrow_signed::<i32>(tag, &numbers)?),
                _ => Value::from_i64s(&narrow_signed::<i64>(tag, &numbers)?),
            }
        }
        Vr::Fl | Vr::Fd => {
            let mut reals = Vec::new();
            for item in values {
                reals.push(item.as_f64().ok_or_else(|| {
                    Error::InvalidValue(format!("element {tag} has a non-numeric value"))
                })?);
            }
            if vr == Vr::Fl {
                Value::from_f32s(&reals.into_iter().map(|n| n as f32).collect::<Vec<_>>())
            } else {
                Value::from_f64s(&reals)
            }
        }
        _ => {
            return Err(Error::InvalidValue(format!(
                "VR {} has no JSON number form",
                vr.as_str()
            )));
        }
    };
    Ok(value)
}

/// A JSON number as an exact integer, kept in the order it appeared.
fn integer_of(tag: Tag, item: &Json) -> Result<i128> {
    if let Some(n) = item.as_i64() {
        return Ok(i128::from(n));
    }
    if let Some(n) = item.as_u64() {
        return Ok(i128::from(n));
    }
    match item.as_f64() {
        Some(n) if n.fract() == 0.0 => Ok(n as i128),
        _ => Err(Error::InvalidValue(format!(
            "element {tag} has a non-integer value"
        ))),
    }
}

/// Narrow unsigned numbers, refusing one that does not fit.
fn narrow_unsigned<T>(tag: Tag, values: &[u64]) -> Result<Vec<T>>
where
    T: TryFrom<u64>,
{
    values
        .iter()
        .map(|n| {
            T::try_from(*n).map_err(|_| {
                Error::InvalidValue(format!("element {tag} value {n} is out of range"))
            })
        })
        .collect()
}

/// Narrow signed numbers, refusing one that does not fit.
fn narrow_signed<T>(tag: Tag, values: &[i128]) -> Result<Vec<T>>
where
    T: TryFrom<i128>,
{
    values
        .iter()
        .map(|n| {
            T::try_from(*n).map_err(|_| {
                Error::InvalidValue(format!("element {tag} value {n} is out of range"))
            })
        })
        .collect()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        let _ = write!(
            out,
            "{}{}{}{}",
            ALPHABET[(triple >> 18) as usize & 0x3F] as char,
            ALPHABET[(triple >> 12) as usize & 0x3F] as char,
            if chunk.len() > 1 {
                ALPHABET[(triple >> 6) as usize & 0x3F] as char
            } else {
                '='
            },
            if chunk.len() > 2 {
                ALPHABET[triple as usize & 0x3F] as char
            } else {
                '='
            },
        );
    }
    out
}

fn base64_decode(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut bits = 0u32;
    let mut count = 0u32;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => {
                return Err(Error::InvalidValue(format!(
                    "{:?} is not a base64 digit",
                    other as char
                )))
            }
        };
        bits = (bits << 6) | u32::from(digit);
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DataSet {
        let mut set = DataSet::new();
        set.set_string(Tag::new(0x0010, 0x0010), Vr::Pn, "Doe^John");
        set.set_string(Tag::new(0x0008, 0x0060), Vr::Cs, "CT");
        set.set(Tag::new(0x0028, 0x0010), Vr::Us, Value::from_u16s(&[512]));
        set.set(Tag::new(0x0018, 0x1310), Vr::Ss, Value::from_i16s(&[-1, 2]));
        set.set(Tag::new(0x0028, 0x0006), Vr::Us, Value::from_u16s(&[]));
        let mut item = DataSet::new();
        item.set_string(Tag::new(0x0008, 0x0100), Vr::Sh, "ORIGINAL");
        set.set(
            Tag::new(0x0008, 0x1115),
            Vr::Sq,
            Value::sequence(vec![item]),
        );
        set.set(
            Tag::new(0x7FE0, 0x0010),
            Vr::Ob,
            Value::from_bytes([1u8, 2, 3, 4]),
        );
        set.set(
            Tag::new(0x0020, 0x5000),
            Vr::At,
            Value::from_tags(&[Tag::new(0x0028, 0x0010)]),
        );
        set
    }

    #[test]
    fn a_data_set_survives_a_round_trip() {
        let set = sample();
        let json = set.to_json().expect("json");
        let back = DataSet::from_json(&json).expect("data set");
        assert_eq!(back, set);
    }

    #[test]
    fn values_are_leaves_of_the_right_kind() {
        let set = sample();
        let json = set.to_json().expect("json");
        let object = json.as_object().expect("object");

        let name = &object["00100010"];
        assert_eq!(name["vr"], "PN");
        assert_eq!(name["Value"][0]["Alphabetic"], "Doe^John");

        let rows = &object["00280010"];
        assert_eq!(rows["Value"][0], 512);

        let pixel = &object["7FE00010"];
        assert_eq!(pixel["vr"], "OB");
        assert_eq!(pixel["InlineBinary"], "AQIDBA==");

        let item = &object["00081115"]["Value"][0];
        assert_eq!(item["00080100"]["Value"][0], "ORIGINAL");

        assert_eq!(object["00205000"]["Value"][0], "00280010");
    }

    #[test]
    fn an_empty_value_keeps_its_vr() {
        let mut set = DataSet::new();
        set.set(Tag::new(0x0010, 0x0010), Vr::Pn, Value::Empty);
        let json = set.to_json().expect("json");
        assert_eq!(json["00100010"]["vr"], "PN");
        assert!(json["00100010"].get("Value").is_none());
    }

    #[test]
    fn the_three_name_groups_survive() {
        let mut set = DataSet::new();
        set.set_string(Tag::new(0x0010, 0x0010), Vr::Pn, "Doe^John=山田^太郎");
        let json = set.to_json().expect("json");
        let name = &json["00100010"]["Value"][0];
        assert_eq!(name["Alphabetic"], "Doe^John");
        assert_eq!(name["Ideographic"], "山田^太郎");
        assert!(name.get("Phonetic").is_none());
        let back = DataSet::from_json(&json).expect("data set");
        assert_eq!(back, set);
    }

    #[test]
    fn encapsulated_pixel_data_is_refused() {
        let mut set = DataSet::new();
        set.set(
            Tag::new(0x7FE0, 0x0010),
            Vr::Ob,
            Value::Fragments(vec![vec![], vec![1, 2]]),
        );
        assert!(matches!(set.to_json(), Err(Error::UnsupportedPixelData(_))));
    }

    #[test]
    fn a_bulk_data_uri_is_refused() {
        let json = serde_json::json!({
            "7FE00010": {"vr": "OB", "BulkDataURI": "https://example.test/pixels"}
        });
        assert!(DataSet::from_json(&json).is_err());
    }

    #[test]
    fn numbers_of_every_signedness_round_trip() {
        let mut set = DataSet::new();
        set.set(Tag::new(0x0018, 0x1310), Vr::Sl, Value::from_i32s(&[-7, 7]));
        set.set(
            Tag::new(0x0066, 0x0040),
            Vr::Uv,
            Value::from_u64s(&[u64::MAX]),
        );
        set.set(Tag::new(0x0072, 0x0050), Vr::Fd, Value::from_f64s(&[1.5]));
        let json = set.to_json().expect("json");
        let back = DataSet::from_json(&json).expect("data set");
        assert_eq!(back, set);
    }

    #[test]
    fn base64_keeps_every_byte() {
        for length in 0..12 {
            let bytes: Vec<u8> = (0..length as u8).collect();
            let text = base64_encode(&bytes);
            assert_eq!(base64_decode(&text).expect("decoded"), bytes);
        }
        assert_eq!(base64_encode(&[0]), "AA==");
        assert_eq!(base64_encode(&[0, 0]), "AAA=");
        assert_eq!(base64_encode(&[0, 0, 0]), "AAAA");
    }
}
