use std::collections::HashMap;
use std::io::Read;

use crate::charset::Charset;
use crate::dataset::DataSet;
use crate::dictionary::{self, tags, VrDef};
use crate::element::Element;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::private_dict::PrivateDictionary;
use crate::read_options::ReadOptions;
use crate::tag::Tag;
use crate::transfer_syntax::TransferSyntax;
use crate::value::Value;
use crate::vr::Vr;

const ITEM: Tag = Tag::new(0xFFFE, 0xE000);
const ITEM_DELIMITER: Tag = Tag::new(0xFFFE, 0xE00D);
const SEQUENCE_DELIMITER: Tag = Tag::new(0xFFFE, 0xE0DD);
const UNDEFINED_LENGTH: u32 = 0xFFFF_FFFF;

/// An element whose value a read skipped because it reached `ReadOptions::defer_size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeferredElement {
    /// The tag of the element.
    pub tag: Tag,
    /// The byte length of the value that was left out.
    pub length: usize,
    /// Where the value starts, counted from the start of the bytes it was read from.
    pub offset: usize,
}

/// Cursor over encoded bytes together with the syntax that governs their layout.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    position: usize,
    syntax: TransferSyntax,
    /// Character set in force for this data set; a sequence item starts from the parent's value
    /// and is restored afterwards.
    charset: Charset,
    /// Optional registry for private element VRs.
    private_dictionary: Option<&'a PrivateDictionary>,
    /// Private creators in force, keyed by `(group, block)`; sequence items inherit and restore.
    private_creators: HashMap<(u16, u16), String>,
    /// Relaxations asked for by the caller; the default reads strictly.
    options: ReadOptions,
    /// Duplicate tags seen so far, shared by the data set and the items it contains.
    duplicates: Vec<Tag>,
    /// Elements whose values were skipped for reaching [`ReadOptions::defer_size`].
    deferred: Vec<DeferredElement>,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8], syntax: TransferSyntax) -> Self {
        Self::new_with(data, syntax, None, ReadOptions::default())
    }

    pub(crate) fn new_with(
        data: &'a [u8],
        syntax: TransferSyntax,
        private_dictionary: Option<&'a PrivateDictionary>,
        options: ReadOptions,
    ) -> Self {
        Self {
            data,
            position: 0,
            syntax,
            charset: Charset::default(),
            private_dictionary,
            private_creators: HashMap::new(),
            options,
            duplicates: Vec::new(),
            deferred: Vec::new(),
        }
    }

    pub(crate) const fn position(&self) -> usize {
        self.position
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    fn endian(&self) -> Endian {
        self.syntax.endian()
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        let start = self.position;
        let end = start
            .checked_add(length)
            .ok_or(Error::UnexpectedEof { offset: start })?;
        if end > self.data.len() {
            return Err(Error::UnexpectedEof { offset: start });
        }
        self.position = end;
        Ok(&self.data[start..end])
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes = self.read_bytes(2)?;
        Ok(self.endian().u16([bytes[0], bytes[1]]))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_bytes(4)?;
        Ok(self.endian().u32([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn peek_tag(&self) -> Result<Tag> {
        if self.remaining() < 4 {
            return Err(Error::UnexpectedEof {
                offset: self.position,
            });
        }
        let bytes = &self.data[self.position..self.position + 4];
        Ok(Tag::new(
            self.endian().u16([bytes[0], bytes[1]]),
            self.endian().u16([bytes[2], bytes[3]]),
        ))
    }

    fn read_tag(&mut self) -> Result<Tag> {
        let group = self.read_u16()?;
        let element = self.read_u16()?;
        Ok(Tag::new(group, element))
    }

    /// Read elements until `end`. With `stop_at_delimiter`, a delimiter tag ends the loop
    /// without being consumed — that is how an undefined-length item finds its delimiter.
    pub(crate) fn read_data_set(&mut self, end: usize, stop_at_delimiter: bool) -> Result<DataSet> {
        let mut set = DataSet::with_endian(self.endian());
        set.set_charset(self.charset);
        let first_duplicate = self.duplicates.len();
        while self.position < end {
            if stop_at_delimiter && self.peek_tag()?.group() == 0xFFFE {
                break;
            }
            let element_offset = self.position;
            let Some(element) = self.read_element()? else {
                // A deferred value still has to end inside its container.
                if self.position > end {
                    return Err(Error::InvalidData {
                        offset: element_offset,
                        reason: "value runs past the end of its container",
                    });
                }
                continue;
            };
            self.remember_private_creator(&element);
            // The container's length bounds its elements, so a value that runs past `end` means
            // the stream is malformed. Without this check the value would silently consume the
            // bytes after the item.
            if self.position > end {
                return Err(Error::InvalidData {
                    offset: element_offset,
                    reason: "value runs past the end of its container",
                });
            }
            // A `(0008,0005)` element switches the character set for the rest of this data set
            // (and for its sequence items). An unknown name leaves the previous value in place;
            // `DataSet::charset` reports the unknown name when it is read.
            if let Some(charset) = charset_of(&element) {
                self.charset = charset;
            }
            // pydicom warns about duplicates and ignores the later one (pydicom/filereader.py:
            // 441-444, 517-532); we keep the first occurrence and list the tag instead.
            if set.contains(element.tag) {
                self.duplicates.push(element.tag);
            } else {
                set.insert(element);
            }
            set.set_charset(self.charset);
        }
        set.set_duplicates(self.duplicates[first_duplicate..].to_vec());
        Ok(set)
    }

    pub(crate) fn read_element(&mut self) -> Result<Option<Element>> {
        let tag = self.read_tag()?;
        if tag.group() == 0xFFFE {
            return Err(Error::InvalidData {
                offset: self.position - 4,
                reason: "unexpected item or delimiter tag",
            });
        }
        let mut vr = if self.syntax.is_explicit_vr() {
            let offset = self.position;
            let code = self.read_bytes(2)?;
            let text = std::str::from_utf8(code).map_err(|_| Error::InvalidData {
                offset,
                reason: "VR code is not valid UTF-8",
            })?;
            text.parse::<Vr>().map_err(|_| Error::InvalidData {
                offset,
                reason: "unknown VR code",
            })?
        } else {
            self.implicit_vr(tag)
        };
        let length = if !self.syntax.is_explicit_vr() {
            self.read_u32()?
        } else if vr.uses_long_length() {
            let _reserved = self.read_bytes(2)?;
            self.read_u32()?
        } else {
            u32::from(self.read_u16()?)
        };

        let value = if length == UNDEFINED_LENGTH {
            if vr.is_sequence() {
                Value::sequence(self.read_items_undefined()?)
            } else if matches!(vr, Vr::Ob | Vr::Ow) {
                Value::Fragments(self.read_fragments()?)
            } else if vr == Vr::Un {
                // PS3.5 6.2.2, applied by pydicom's filereader (filereader.py:284-287): an
                // undefined length on UN means a sequence, because no byte value can run to a
                // delimiter.
                vr = Vr::Sq;
                Value::sequence(self.read_items_undefined()?)
            } else if self.options.lenient_undefined_length {
                // pydicom's `read_undefined_length_value`: the bytes before the item delimiter
                // are the value, whatever VR the element claims.
                Value::from_bytes(self.read_undefined_value()?)
            } else {
                return Err(Error::InvalidData {
                    offset: self.position,
                    reason: "undefined length on a non-sequence element",
                });
            }
        } else if vr.is_sequence() {
            let offset = self.position;
            let end = offset
                .checked_add(length as usize)
                .ok_or(Error::UnexpectedEof { offset })?;
            if end > self.data.len() {
                return Err(Error::UnexpectedEof { offset });
            }
            Value::sequence(self.read_items_defined(end)?)
        } else if self.should_defer(length as usize) {
            // The value stays out of the data set; the file remembers how to read it back.
            let offset = self.position();
            self.read_bytes(length as usize)?;
            self.deferred.push(DeferredElement {
                tag,
                length: length as usize,
                offset,
            });
            return Ok(None);
        } else {
            let bytes = self.read_bytes(length as usize)?.to_vec();
            if vr == Vr::Un {
                vr = self.resolve_un_vr(tag, bytes.len());
            }
            Value::from_bytes(bytes)
        };
        Ok(Some(Element::new(tag, vr, value)))
    }

    /// Whether a value of `length` bytes is left out of the data set.
    fn should_defer(&self, length: usize) -> bool {
        self.options.defer_size > 0 && length >= self.options.defer_size
    }

    /// Elements whose values were skipped, in the order they were met.
    pub(crate) fn deferred(&self) -> &[DeferredElement] {
        &self.deferred
    }

    /// Body of one item whose length field has already been read. The item starts with the
    /// parent's character set and must not change it for its siblings.
    fn read_item_body(&mut self, length: u32) -> Result<DataSet> {
        let saved_charset = self.charset;
        let saved_creators = self.private_creators.clone();
        let item = self.read_item_body_inner(length);
        self.charset = saved_charset;
        self.private_creators = saved_creators;
        item
    }

    fn read_item_body_inner(&mut self, length: u32) -> Result<DataSet> {
        if length == UNDEFINED_LENGTH {
            let item = self.read_data_set(self.data.len(), true)?;
            self.expect_delimiter(ITEM_DELIMITER, "missing item delimiter")?;
            Ok(item)
        } else {
            let offset = self.position;
            let end = offset
                .checked_add(length as usize)
                .ok_or(Error::UnexpectedEof { offset })?;
            if end > self.data.len() {
                return Err(Error::UnexpectedEof { offset });
            }
            self.read_data_set(end, false)
        }
    }

    fn read_items_defined(&mut self, end: usize) -> Result<Vec<DataSet>> {
        let mut items = Vec::new();
        while self.position < end {
            let offset = self.position;
            let tag = self.read_tag()?;
            if tag != ITEM {
                return Err(Error::InvalidData {
                    offset,
                    reason: "expected an item tag inside a sequence",
                });
            }
            let length = self.read_u32()?;
            if length != UNDEFINED_LENGTH {
                let item_end = self
                    .position
                    .checked_add(length as usize)
                    .ok_or(Error::UnexpectedEof { offset })?;
                if item_end > end {
                    return Err(Error::InvalidData {
                        offset,
                        reason: "item runs past the end of the sequence",
                    });
                }
            }
            items.push(self.read_item_body(length)?);
        }
        Ok(items)
    }

    fn read_items_undefined(&mut self) -> Result<Vec<DataSet>> {
        let mut items = Vec::new();
        loop {
            let offset = self.position;
            let tag = self.read_tag()?;
            if tag == SEQUENCE_DELIMITER {
                let _delimiter_length = self.read_u32()?;
                return Ok(items);
            }
            if tag != ITEM {
                return Err(Error::InvalidData {
                    offset,
                    reason: "expected an item tag or the sequence delimiter",
                });
            }
            let length = self.read_u32()?;
            items.push(self.read_item_body(length)?);
        }
    }

    fn read_fragments(&mut self) -> Result<Vec<Vec<u8>>> {
        let mut fragments = Vec::new();
        loop {
            let offset = self.position;
            let tag = self.read_tag()?;
            if tag == SEQUENCE_DELIMITER {
                let _delimiter_length = self.read_u32()?;
                return Ok(fragments);
            }
            if tag != ITEM {
                return Err(Error::InvalidData {
                    offset,
                    reason: "expected a fragment item or the sequence delimiter",
                });
            }
            let length = self.read_u32()?;
            if length == UNDEFINED_LENGTH {
                return Err(Error::InvalidData {
                    offset,
                    reason: "fragment items must have a defined length",
                });
            }
            fragments.push(self.read_bytes(length as usize)?.to_vec());
        }
    }

    /// Read a value whose length is undefined but whose VR is not a sequence or a byte value by
    /// scanning to the item delimiter that ends it.
    ///
    /// This mirrors pydicom's `read_undefined_length_value`: the delimiter is searched for byte
    /// by byte, so a value that happens to contain the delimiter's four bytes would end early.
    /// The same limitation applies to pydicom.
    fn read_undefined_value(&mut self) -> Result<Vec<u8>> {
        let start = self.position;
        let delimiter = self.delimiter_bytes();
        let mut position = start;
        while position + 4 <= self.data.len() {
            if self.data[position..position + 4] == delimiter {
                let end = position;
                if end + 8 > self.data.len() {
                    return Err(Error::UnexpectedEof { offset: end });
                }
                let value = self.data[start..end].to_vec();
                // The delimiter is `(FFFE,E00D)` plus its four-byte length, which reads as zero.
                self.position = end + 8;
                return Ok(value);
            }
            position += 1;
        }
        Err(Error::UnexpectedEof { offset: start })
    }

    /// The four bytes of the item delimiter in this reader's byte order.
    fn delimiter_bytes(&self) -> [u8; 4] {
        let (group, element) = match self.endian() {
            Endian::Little => (
                ITEM_DELIMITER.group().to_le_bytes(),
                ITEM_DELIMITER.element().to_le_bytes(),
            ),
            Endian::Big => (
                ITEM_DELIMITER.group().to_be_bytes(),
                ITEM_DELIMITER.element().to_be_bytes(),
            ),
        };
        [group[0], group[1], element[0], element[1]]
    }

    fn expect_delimiter(&mut self, expected: Tag, reason: &'static str) -> Result<()> {
        let offset = self.position;
        let tag = self.read_tag()?;
        if tag != expected {
            return Err(Error::InvalidData { offset, reason });
        }
        let _length = self.read_u32()?;
        Ok(())
    }
}

/// Character set declared by a `(0008,0005)` element, when the name is one this crate handles.
fn charset_of(element: &Element) -> Option<Charset> {
    if element.tag != tags::SPECIFIC_CHARACTER_SET {
        return None;
    }
    Charset::from_element(element).ok()
}

impl<'a> Reader<'a> {
    /// VR of an element in implicit VR encoding, taken from the public or private dictionary.
    ///
    /// The fallbacks follow pydicom's `hooks.raw_element_vr` (`pydicom/hooks.py:181-212`): private
    /// creators are `LO`, other private elements are `UN` unless a [`PrivateDictionary`] resolves
    /// them, and group length elements are `UL` because the registry has no entry for them.
    fn implicit_vr(&self, tag: Tag) -> Vr {
        if let Some(def) = dictionary::vr(tag) {
            return match def {
                VrDef::Fixed(vr) => vr,
                VrDef::Ambiguous(set) => implicit_default(set),
            };
        }
        if tag.is_private() {
            if tag.is_private_creator() {
                return Vr::Lo;
            }
            return self.private_vr(tag).unwrap_or(Vr::Un);
        }
        if tag.is_group_length() {
            Vr::Ul
        } else {
            Vr::Un
        }
    }

    /// VR for a `UN` value read with explicit VR encoding.
    ///
    /// PS3.5 6.2.2 lets a known element carry `UN`; pydicom treats that as an encoding error and
    /// restores the dictionary VR when the value fits a 16-bit length (`config.replace_un_with_known_vr`
    /// is on by default, see `pydicom/hooks.py:201-210` and `pydicom/dataelem.py:229-241`).
    fn resolve_un_vr(&self, tag: Tag, length: usize) -> Vr {
        if tag.is_private() {
            if tag.is_private_creator() {
                return Vr::Lo;
            }
            return self.private_vr(tag).unwrap_or(Vr::Un);
        }
        if length >= 0xFFFF {
            return Vr::Un;
        }
        match dictionary::vr(tag) {
            Some(VrDef::Fixed(vr)) => vr,
            Some(VrDef::Ambiguous(set)) => implicit_default(set),
            None => Vr::Un,
        }
    }

    /// VR registered for a private element, using the creator in force for its block.
    fn private_vr(&self, tag: Tag) -> Option<Vr> {
        let dictionary = self.private_dictionary?;
        let block = tag.element() >> 8;
        let creator = self.private_creators.get(&(tag.group(), block))?;
        dictionary.get(creator, tag.group(), tag.element() & 0x00FF)
    }

    /// Remember the creator name declared by a private creator element.
    fn remember_private_creator(&mut self, element: &Element) {
        if !element.tag.is_private_creator() {
            return;
        }
        let Ok(creator) = element.string() else {
            return;
        };
        if creator.is_empty() {
            return;
        }
        let block = element.tag.element() & 0x00FF;
        self.private_creators
            .insert((element.tag.group(), block), creator.to_owned());
    }
}

/// Default VR for an ambiguous dictionary entry under implicit VR encoding (PS3.5 6.2.2). The
/// standard resolves a few entries from context; the typed decoders accept whole VR families
/// (`u16s` takes `US`, `SS` and `UN`), so the default keeps such values readable.
fn implicit_default(set: &[Vr]) -> Vr {
    if set.contains(&Vr::Us) || set.contains(&Vr::Ss) {
        Vr::Us
    } else if set.contains(&Vr::Ow) {
        Vr::Ow
    } else if set.contains(&Vr::Ob) {
        Vr::Ob
    } else {
        set.first().copied().unwrap_or(Vr::Un)
    }
}

impl DataSet {
    /// Read a data set encoded with `syntax`.
    ///
    /// The returned data set carries the byte order of the syntax, see [`DataSet::endian`].
    pub fn read(bytes: &[u8], syntax: TransferSyntax) -> Result<Self> {
        Self::read_inner(bytes, syntax, None, ReadOptions::default())
    }

    /// Read a data set encoded with `syntax`, applying [`ReadOptions`].
    pub fn read_with_options(
        bytes: &[u8],
        syntax: TransferSyntax,
        options: ReadOptions,
    ) -> Result<Self> {
        Self::read_inner(bytes, syntax, None, options)
    }

    /// Read a data set while resolving private element VRs from `private_dictionary`.
    ///
    /// Private creators are read from the stream; elements without a registered VR stay `UN`.
    pub fn read_with_private_dictionary(
        bytes: &[u8],
        syntax: TransferSyntax,
        private_dictionary: &PrivateDictionary,
    ) -> Result<Self> {
        Self::read_inner(
            bytes,
            syntax,
            Some(private_dictionary),
            ReadOptions::default(),
        )
    }

    pub(crate) fn read_inner(
        bytes: &[u8],
        syntax: TransferSyntax,
        private_dictionary: Option<&PrivateDictionary>,
        options: ReadOptions,
    ) -> Result<Self> {
        Ok(Self::read_inner_with_deferred(bytes, syntax, private_dictionary, options)?.0)
    }

    pub(crate) fn read_inner_with_deferred(
        bytes: &[u8],
        syntax: TransferSyntax,
        private_dictionary: Option<&PrivateDictionary>,
        options: ReadOptions,
    ) -> Result<(Self, Vec<crate::reader::DeferredElement>)> {
        if syntax == TransferSyntax::DeflatedExplicitVrLittleEndian {
            let mut decoder = flate2::read::DeflateDecoder::new(bytes);
            let mut inflated = Vec::new();
            decoder
                .read_to_end(&mut inflated)
                .map_err(|_| Error::InvalidData {
                    offset: 0,
                    reason: "deflate: invalid compressed data",
                })?;
            let mut reader = Reader::new_with(
                &inflated,
                TransferSyntax::ExplicitVrLittleEndian,
                private_dictionary,
                options,
            );
            let set = reader.read_data_set(inflated.len(), false)?;
            let deferred = reader.deferred().to_vec();
            return Ok((set, deferred));
        }
        let mut reader = Reader::new_with(bytes, syntax, private_dictionary, options);
        let set = reader.read_data_set(bytes.len(), false)?;
        let deferred = reader.deferred().to_vec();
        Ok((set, deferred))
    }
}
