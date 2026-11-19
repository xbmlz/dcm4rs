use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::charset::Charset;
use crate::dictionary::tags;
use crate::element::Element;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::tag::Tag;
use crate::value::Value;
use crate::valuerep::{Date, DateTime, DecimalString, IntegerString, PersonName, Time};
use crate::vr::Vr;

/// An ordered collection of data elements, corresponding to one DICOM Data Set or one Item.
///
/// Elements are stored in ascending tag order, exactly as required when writing a data set.
/// When the same tag appears twice the later one replaces the earlier; consistency warnings are
/// reported by the parser layer.
#[derive(Clone, Debug, Default)]
pub struct DataSet {
    elements: BTreeMap<Tag, Element>,
    endian: Endian,
    /// Character set of this data set; a sequence item inherits the parent's value when it
    /// carries no `(0008,0005)` of its own.
    charset: Charset,
    /// Tags the encoded stream carried more than once, in the order they were seen.
    duplicates: Vec<Tag>,
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
            duplicates: Vec::new(),
        }
    }

    /// Tags the encoded stream carried more than once, in the order they were seen.
    ///
    /// The first occurrence is kept, as pydicom does — it warns about the later ones instead of
    /// reporting them. A parent data set lists the duplicates found inside its sequence items
    /// too, because the parser walks them with the same reader.
    pub fn duplicates(&self) -> &[Tag] {
        &self.duplicates
    }

    pub(crate) fn set_duplicates(&mut self, duplicates: Vec<Tag>) {
        self.duplicates = duplicates;
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

    /// Read the first date (`DA`).
    pub fn date(&self, tag: Tag) -> Result<Date> {
        let element = self.element(tag)?;
        require_vr(element, Vr::Da, "DA")?;
        let values = self.strings(tag)?;
        let text = values.first().ok_or(Error::NoValue(tag))?;
        Date::parse(text)
    }

    /// Read the first time (`TM`).
    pub fn time(&self, tag: Tag) -> Result<Time> {
        let element = self.element(tag)?;
        require_vr(element, Vr::Tm, "TM")?;
        let values = self.strings(tag)?;
        let text = values.first().ok_or(Error::NoValue(tag))?;
        Time::parse(text)
    }

    /// Read the first date-time (`DT`).
    pub fn date_time(&self, tag: Tag) -> Result<DateTime> {
        let element = self.element(tag)?;
        require_vr(element, Vr::Dt, "DT")?;
        let values = self.strings(tag)?;
        let text = values.first().ok_or(Error::NoValue(tag))?;
        DateTime::parse(text)
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

impl DataSet {
    /// Iterate every element, descending into the items of sequences.
    pub fn walk(&self) -> impl Iterator<Item = &Element> + '_ {
        self.walk_with_path().map(|(_, element)| element)
    }

    /// Iterate every element with the dotted path that finds it.
    ///
    /// The path names an element by its keyword where the dictionary has one and by its tag
    /// otherwise, and numbers sequence items, as in
    /// `ReferencedStudySequence[0].ReferencedSOPClassUID`. A sequence element comes before the
    /// elements of its items, so the order follows the nesting.
    pub fn walk_with_path(&self) -> impl Iterator<Item = (String, &Element)> + '_ {
        Walk {
            frames: vec![Frame {
                prefix: String::new(),
                elements: self.iter().collect::<Vec<_>>().into_iter(),
            }],
        }
    }

    /// Remove every private element, descending into the items of sequences.
    ///
    /// Private elements are those in an odd group other than the reserved ones, the set
    /// [`Tag::is_private`] reports, which is what pydicom's `remove_private_tags` does too.
    pub fn remove_private_tags(&mut self) {
        self.elements.retain(|tag, _| !tag.is_private());
        for element in self.elements.values_mut() {
            if let Value::Sequence(items) = &mut element.value {
                for item in items {
                    item.remove_private_tags();
                }
            }
        }
    }

    /// A data set holding the elements of one group.
    ///
    /// The copy keeps the byte order and the inherited character set, and shares nothing with
    /// the original. pydicom's `group_dataset` returns the same selection.
    pub fn group_dataset(&self, group: u16) -> Self {
        let mut set = Self::with_endian(self.endian);
        set.charset = self.charset;
        for element in self.iter().filter(|element| element.tag.group() == group) {
            set.elements.insert(element.tag, element.clone());
        }
        set
    }

    /// One line per element, in the flat order pydicom's `Dataset.formatted_lines` yields.
    ///
    /// A line is the tag, the standard name padded to 35 characters, the VR and a short
    /// representation of the value: text in quotes, numbers as written, a UID by its registry
    /// name, a sequence as `<Sequence, length 2>`, and a long or many-valued value as
    /// `Array of 12 elements`. Values are decoded with this data set's byte order and character
    /// set, and the items of a sequence come right after their sequence element.
    pub fn formatted_lines(&self) -> impl Iterator<Item = String> + '_ {
        let endian = self.endian;
        let charset = self.charset().unwrap_or_default();
        self.walk()
            .map(move |element| format_line(element, endian, charset))
    }
}

/// Longest value pydicom prints before summarizing it (`DataElement.maxBytesToDisplay`).
const MAX_DISPLAY: usize = 128;

/// One `formatted_lines` line.
fn format_line(element: &Element, endian: Endian, charset: Charset) -> String {
    let name = display_name(element.tag);
    let name: String = name.chars().take(35).collect();
    format!(
        "{} {name:<35} {}: {}",
        element.tag,
        element.vr,
        representation(element, endian, charset)
    )
}

/// The standard name of a tag, as pydicom's `DataElement.name` reports it.
fn display_name(tag: Tag) -> String {
    match crate::dictionary::lookup(tag) {
        Some(entry) => entry.name.to_owned(),
        None if tag.is_private() => "Private tag data".to_owned(),
        None => "Unknown Tag & Data".to_owned(),
    }
}

/// A short form of an element's value, as pydicom's `DataElement.repval` reports it.
fn representation(element: &Element, endian: Endian, charset: Charset) -> String {
    match &element.value {
        Value::Empty => String::new(),
        Value::Bytes(bytes) if bytes.is_empty() => String::new(),
        Value::Sequence(items) => format!("<Sequence, length {}>", items.len()),
        Value::Fragments(fragments) => {
            let bytes: usize = fragments.iter().map(Vec::len).sum();
            if bytes > MAX_DISPLAY {
                format!("Array of {bytes} elements")
            } else {
                format!("<{} fragments>", fragments.len())
            }
        }
        Value::Bytes(bytes) => {
            if element.vr == Vr::Ui {
                return element.string().map_or_else(
                    |_| String::new(),
                    |text| {
                        crate::uid::Uid::new(text)
                            .map_or_else(|_| text.to_owned(), |uid| uid.name().to_owned())
                    },
                );
            }
            if element.vr.is_string() {
                let values: Vec<String> = element
                    .string_values_with(charset)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|value| value.into_owned())
                    .collect();
                if values.len() > MAX_DISPLAY {
                    return format!("Array of {} elements", values.len());
                }
                let length: usize = values.iter().map(String::len).sum();
                if element.vr.uses_long_length() && length > MAX_DISPLAY {
                    return format!("Array of {length} elements");
                }
                return match element.vr {
                    // pydicom keeps `IS`/`DS` as numbers, so they print without quotes.
                    Vr::Is | Vr::Ds => bracketed(&values),
                    _ => quoted(&values),
                };
            }
            if let Some(values) = binary_numbers(element, endian) {
                if values.len() > MAX_DISPLAY {
                    return format!("Array of {} elements", values.len());
                }
                return bracketed(&values);
            }
            if bytes.len() > MAX_DISPLAY {
                format!("Array of {} elements", bytes.len())
            } else {
                format!("{bytes:?}")
            }
        }
    }
}

/// Decode the binary numeric VRs into their printed form.
fn binary_numbers(element: &Element, endian: Endian) -> Option<Vec<String>> {
    let values: Vec<String> = match element.vr {
        Vr::Us => element
            .u16s(endian)
            .ok()?
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
        Vr::Ss => element
            .i16s(endian)
            .ok()?
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
        Vr::Ul => element
            .u32s(endian)
            .ok()?
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
        Vr::Sl => element
            .i32s(endian)
            .ok()?
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
        Vr::Uv => element
            .u64s(endian)
            .ok()?
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
        Vr::Sv => element
            .i64s(endian)
            .ok()?
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
        Vr::Fl => element
            .f32s(endian)
            .ok()?
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
        Vr::Fd => element
            .f64s(endian)
            .ok()?
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
        Vr::At => element
            .tags(endian)
            .ok()?
            .into_iter()
            .map(|value| value.to_string())
            .collect(),
        _ => return None,
    };
    Some(values)
}

/// One value as it is, several as a Python-style list.
fn bracketed(values: &[String]) -> String {
    if let [one] = values {
        return one.clone();
    }
    format!("[{}]", values.join(", "))
}

/// Text values in single quotes, several as a list of quoted values.
fn quoted(values: &[String]) -> String {
    let quoted: Vec<String> = values.iter().map(|value| format!("'{value}'")).collect();
    bracketed(&quoted)
}

/// Depth-first walk over a data set and the items of its sequences.
struct Walk<'a> {
    frames: Vec<Frame<'a>>,
}

struct Frame<'a> {
    prefix: String,
    elements: std::vec::IntoIter<&'a Element>,
}

impl<'a> Iterator for Walk<'a> {
    type Item = (String, &'a Element);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let frame = self.frames.last_mut()?;
            let Some(element) = frame.elements.next() else {
                self.frames.pop();
                continue;
            };
            let name = element_name(element.tag);
            let path = if frame.prefix.is_empty() {
                name
            } else {
                format!("{}.{name}", frame.prefix)
            };
            if let Some(items) = element.value.as_sequence() {
                for (index, item) in items.iter().enumerate().rev() {
                    self.frames.push(Frame {
                        prefix: format!("{path}[{index}]"),
                        elements: item.iter().collect::<Vec<_>>().into_iter(),
                    });
                }
            }
            return Some((path, element));
        }
    }
}

/// The keyword of an element when the dictionary has one, its tag otherwise.
fn element_name(tag: Tag) -> String {
    match crate::dictionary::lookup(tag) {
        Some(entry) => entry.keyword.to_owned(),
        None => tag.to_string(),
    }
}

/// Two data sets are equal when they hold the same elements.
///
/// The byte order and the inherited character set are reading state rather than content, so they
/// do not take part: the same numbers in another byte order compare equal here, even though they
/// decode differently until [`DataSet::to_endian`] makes the two agree.
impl PartialEq for DataSet {
    fn eq(&self, other: &Self) -> bool {
        self.elements == other.elements
    }
}

impl Eq for DataSet {}

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
