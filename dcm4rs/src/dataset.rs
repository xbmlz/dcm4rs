use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::charset::Charset;
use crate::dictionary::tags;
use crate::element::Element;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::tag::Tag;
use crate::value::Value;
use crate::valuerep::{DecimalString, IntegerString, PersonName};
use crate::vr::Vr;

/// An ordered collection of data elements, corresponding to one DICOM Data Set or one Item.
///
/// Elements are stored in ascending tag order, exactly as required when writing a data set.
/// When the same tag appears twice the later one replaces the earlier; consistency warnings are
/// reported by the parser layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DataSet {
    elements: BTreeMap<Tag, Element>,
    endian: Endian,
    /// Character set of this data set; a sequence item inherits the parent's value when it
    /// carries no `(0008,0005)` of its own.
    charset: Charset,
}

impl DataSet {
    /// Create an empty data set decoding numbers as little endian.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty data set with the given byte order (used when parsing big-endian transfer
    /// syntaxes).
    pub fn with_endian(endian: Endian) -> Self {
        Self {
            elements: BTreeMap::new(),
            endian,
            charset: Charset::default(),
        }
    }

    /// The byte order used for numbers in this data set.
    pub fn endian(&self) -> Endian {
        self.endian
    }

    /// Set the byte order. Only a parser that knows the source transfer syntax should call this.
    pub fn set_endian(&mut self, endian: Endian) {
        self.endian = endian;
    }

    /// Return a copy whose numeric values are stored in `endian`.
    ///
    /// Text, `OB`/`UN` values and encapsulated fragments are copied unchanged; sequences are
    /// converted recursively. Numeric values whose byte length is not a multiple of the VR unit
    /// report [`Error::WrongLength`].
    pub fn to_endian(&self, endian: Endian) -> Result<Self> {
        let from = self.endian;
        let mut out = self.clone();
        out.endian = endian;
        for element in out.elements.values_mut() {
            *element = element.transcode(from, endian)?;
        }
        Ok(out)
    }

    /// Number of elements.
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Whether there are no elements.
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Whether the tag is present.
    pub fn contains(&self, tag: Tag) -> bool {
        self.elements.contains_key(&tag)
    }

    /// Insert an element, returning the previous element it replaced.
    ///
    /// Inserting `(0008,0005)` updates the data set's character set; an unknown name leaves the
    /// previous character set in place and reports through [`Self::charset`].
    pub fn insert(&mut self, element: Element) -> Option<Element> {
        if element.tag == tags::SPECIFIC_CHARACTER_SET {
            if let Ok(charset) = Charset::from_element(&element) {
                self.charset = charset;
            }
        }
        self.elements.insert(element.tag, element)
    }

    /// Insert an element by tag, VR and value.
    pub fn set(&mut self, tag: Tag, vr: Vr, value: Value) -> Option<Element> {
        self.insert(Element::new(tag, vr, value))
    }

    /// Read an element by tag.
    pub fn get(&self, tag: Tag) -> Option<&Element> {
        self.elements.get(&tag)
    }

    /// Read an element by tag mutably.
    pub fn get_mut(&mut self, tag: Tag) -> Option<&mut Element> {
        self.elements.get_mut(&tag)
    }

    /// Remove an element.
    pub fn remove(&mut self, tag: Tag) -> Option<Element> {
        self.elements.remove(&tag)
    }

    /// Iterate elements in ascending tag order.
    pub fn iter(&self) -> impl Iterator<Item = &Element> + '_ {
        self.elements.values()
    }

    /// Iterate elements mutably in ascending tag order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Element> + '_ {
        self.elements.values_mut()
    }

    /// Iterate all tags in ascending order.
    pub fn tags(&self) -> impl Iterator<Item = Tag> + '_ {
        self.elements.keys().copied()
    }

    /// Read an element by tag, returning [`Error::MissingTag`] when absent.
    pub fn element(&self, tag: Tag) -> Result<&Element> {
        self.get(tag).ok_or(Error::MissingTag(tag))
    }

    /// The character set declared by `(0008,0005)`, or the value inherited from the parent data
    /// set when the element is absent.
    pub fn charset(&self) -> Result<Charset> {
        match self.get(tags::SPECIFIC_CHARACTER_SET) {
            Some(element) => Charset::from_element(element),
            None => Ok(self.charset),
        }
    }

    /// Set the character set used when `(0008,0005)` is absent. Parsers use this to hand a
    /// sequence item the character set of its parent.
    pub fn set_charset(&mut self, charset: Charset) {
        self.charset = charset;
    }

    /// Read the first string value, decoded with the data set's character set.
    pub fn string(&self, tag: Tag) -> Result<Cow<'_, str>> {
        let charset = self.charset()?;
        self.element(tag)?.string_with(charset)
    }

    /// Read all string values, decoded with the data set's character set.
    pub fn strings(&self, tag: Tag) -> Result<Vec<Cow<'_, str>>> {
        let charset = self.charset()?;
        self.element(tag)?.string_values_with(charset)
    }

    /// Read a single 16-bit unsigned integer.
    pub fn u16(&self, tag: Tag) -> Result<u16> {
        self.u16s(tag)?.first().copied().ok_or(Error::NoValue(tag))
    }

    /// Read all 16-bit unsigned integer values.
    pub fn u16s(&self, tag: Tag) -> Result<Vec<u16>> {
        self.element(tag)?.u16s(self.endian)
    }

    /// Read all 32-bit signed integer values.
    pub fn i32s(&self, tag: Tag) -> Result<Vec<i32>> {
        self.element(tag)?.i32s(self.endian)
    }

    /// Read all double-precision float values.
    pub fn f64s(&self, tag: Tag) -> Result<Vec<f64>> {
        self.element(tag)?.f64s(self.endian)
    }

    /// Read all numeric values of `DS` text.
    pub fn decimal_values(&self, tag: Tag) -> Result<Vec<f64>> {
        self.element(tag)?.decimal_values()
    }

    /// Read the first person name (`PN`), decoded with the data set's character set.
    pub fn person_name(&self, tag: Tag) -> Result<PersonName> {
        let element = self.element(tag)?;
        require_vr(element, Vr::Pn, "PN")?;
        let values = self.strings(tag)?;
        let text = values.first().ok_or(Error::NoValue(tag))?;
        PersonName::parse(text)
    }

    /// Read every decimal string (`DS`), keeping the text each value was written as.
    pub fn decimal_strings(&self, tag: Tag) -> Result<Vec<DecimalString>> {
        let element = self.element(tag)?;
        require_vr(element, Vr::Ds, "DS")?;
        let mut values = Vec::new();
        for part in self.strings(tag)? {
            if part.trim().is_empty() {
                continue;
            }
            values.push(DecimalString::parse(&part)?);
        }
        Ok(values)
    }

    /// Read every integer string (`IS`), keeping the text each value was written as.
    pub fn integer_strings(&self, tag: Tag) -> Result<Vec<IntegerString>> {
        let element = self.element(tag)?;
        require_vr(element, Vr::Is, "IS")?;
        let mut values = Vec::new();
        for part in self.strings(tag)? {
            if part.trim().is_empty() {
                continue;
            }
            values.push(IntegerString::parse(&part)?);
        }
        Ok(values)
    }

    /// Read sequence items.
    pub fn sequence(&self, tag: Tag) -> Result<&[DataSet]> {
        self.element(tag)?.as_sequence()
    }

    /// Write a string value.
    pub fn set_string(&mut self, tag: Tag, vr: Vr, text: &str) {
        self.set(tag, vr, Value::from_str(text));
    }

    /// Write a single 16-bit unsigned integer (`US`).
    pub fn set_u16(&mut self, tag: Tag, value: u16) {
        self.set(tag, Vr::Us, Value::from_u16s(&[value]));
    }

    /// Write a sequence.
    pub fn set_sequence(&mut self, tag: Tag, items: Vec<DataSet>) {
        self.set(tag, Vr::Sq, Value::sequence(items));
    }
}

/// Reject an element whose VR is not the one the accessor expects.
fn require_vr(element: &Element, vr: Vr, name: &'static str) -> Result<()> {
    if element.vr != vr {
        return Err(Error::WrongVr {
            actual: element.vr,
            expected: name,
        });
    }
    Ok(())
}
