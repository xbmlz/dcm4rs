use std::path::{Path, PathBuf};

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::element::Element;
use crate::error::{Error, Result};
use crate::private_dict::PrivateDictionary;
use crate::read_options::ReadOptions;
use crate::reader::{DeferredElement, Reader};
use crate::tag::Tag;
use crate::transfer_syntax::TransferSyntax;
use crate::value::Value;
use crate::vr::Vr;
use crate::writer::write_data_set;

/// 128-byte preamble that precedes the `DICM` prefix of a Part 10 file.
const PREAMBLE_LENGTH: usize = 128;

/// Offset of the `DICM` prefix, which carries no information of its own.
const DICM_OFFSET: usize = PREAMBLE_LENGTH;

/// Implementation Class UID: a UUID-derived root under the `2.25` arc, so no registration is
/// needed.
const IMPLEMENTATION_CLASS_UID: &str = "2.25.247279088714401268306282486322359480112";

/// Where a file's bytes came from, so a deferred element can be read again.
#[derive(Clone, Debug)]
enum Source {
    Bytes(Vec<u8>),
    Path(PathBuf),
}

/// A DICOM Part 10 file: the file meta information plus the main data set.
#[derive(Clone, Debug)]
pub struct DicomFile {
    /// File meta information (group 0002); always explicit VR little endian.
    pub meta: DataSet,
    /// The main data set, encoded with the transfer syntax declared in `meta`.
    pub dataset: DataSet,
    /// The 128-byte preamble, kept exactly as it was read.
    ///
    /// PS3.10 leaves it unconstrained, and some writers put a signature or a vendor string in it,
    /// so keeping it lets a file be read and written back byte for byte. A file built with
    /// [`DicomFile::new`] starts with zeros.
    pub preamble: [u8; PREAMBLE_LENGTH],
    /// Where the file's bytes came from, so a deferred value can be read again.
    source: Option<Source>,
    /// Elements whose values a read left out, in stream order.
    deferred: Vec<DeferredElement>,
}

impl DicomFile {
    /// Read a Part 10 file from bytes.
    ///
    /// The input must start with the 128-byte preamble and the `DICM` prefix; a bare data set
    /// is read with [`DataSet::read`] instead.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        Self::read_inner(bytes, None, ReadOptions::default())
    }

    /// Read a Part 10 file, applying [`ReadOptions`].
    pub fn read_with_options(bytes: &[u8], options: ReadOptions) -> Result<Self> {
        Self::read_inner(bytes, None, options)
    }

    /// Read a Part 10 file while resolving private element VRs from `private_dictionary`.
    pub fn read_with_private_dictionary(
        bytes: &[u8],
        private_dictionary: &PrivateDictionary,
    ) -> Result<Self> {
        Self::read_inner(bytes, Some(private_dictionary), ReadOptions::default())
    }

    fn read_inner(
        bytes: &[u8],
        private_dictionary: Option<&PrivateDictionary>,
        options: ReadOptions,
    ) -> Result<Self> {
        if bytes.len() < DICM_OFFSET + 4 || &bytes[DICM_OFFSET..DICM_OFFSET + 4] != b"DICM" {
            return Err(Error::InvalidData {
                offset: DICM_OFFSET,
                reason: "missing DICM prefix",
            });
        }
        let mut preamble = [0u8; PREAMBLE_LENGTH];
        preamble.copy_from_slice(&bytes[..PREAMBLE_LENGTH]);
        let meta_start = DICM_OFFSET + 4;
        let mut reader = Reader::new(&bytes[meta_start..], TransferSyntax::ExplicitVrLittleEndian);
        let mut meta = DataSet::new();
        while reader.remaining() >= 4 && reader.peek_tag()?.group() == 0x0002 {
            if let Some(element) = reader.read_element()? {
                meta.insert(element);
            }
        }
        let dataset_start = meta_start + reader.position();
        let syntax = transfer_syntax_from_meta(&meta)?;
        let (dataset, deferred) = DataSet::read_inner_with_deferred(
            &bytes[dataset_start..],
            syntax,
            private_dictionary,
            options,
        )?;
        let source = if deferred.is_empty() {
            None
        } else {
            Some(Source::Bytes(bytes.to_vec()))
        };
        Ok(Self {
            meta,
            dataset,
            preamble,
            source,
            deferred,
        })
    }

    /// Read a Part 10 file from disk.
    pub fn read_file(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|err| Error::Io(err.to_string()))?;
        Self::read(&bytes)
    }

    /// Read a Part 10 file from disk, applying [`ReadOptions`].
    pub fn read_file_with_options(path: impl AsRef<Path>, options: ReadOptions) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let bytes = std::fs::read(&path).map_err(|err| Error::Io(err.to_string()))?;
        let mut file = Self::read_with_options(&bytes, options)?;
        if !file.deferred.is_empty() {
            // Re-open the file on demand rather than hold a copy of it in memory.
            file.source = Some(Source::Path(path));
        }
        Ok(file)
    }

    /// Read a Part 10 file from disk while resolving private element VRs.
    pub fn read_file_with_private_dictionary(
        path: impl AsRef<Path>,
        private_dictionary: &PrivateDictionary,
    ) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|err| Error::Io(err.to_string()))?;
        Self::read_with_private_dictionary(&bytes, private_dictionary)
    }

    /// The transfer syntax declared by `(0002,0010)`.
    pub fn transfer_syntax(&self) -> Result<TransferSyntax> {
        transfer_syntax_from_meta(&self.meta)
    }

    /// Assemble a new file for `dataset`.
    ///
    /// The meta information required by PS3.10 is derived from the data set: its SOP Class UID
    /// and SOP Instance UID become the media storage UIDs, and the implementation identifiers
    /// describe this crate.
    pub fn new(dataset: DataSet, syntax: TransferSyntax) -> Result<Self> {
        let sop_class = dataset.string(tags::SOP_CLASS_UID)?.into_owned();
        let sop_instance = dataset.string(tags::SOP_INSTANCE_UID)?.into_owned();
        let mut meta = DataSet::new();
        meta.set(
            tags::FILE_META_INFORMATION_VERSION,
            Vr::Ob,
            Value::from_bytes(vec![0x00, 0x01]),
        );
        meta.set(
            tags::MEDIA_STORAGE_SOP_CLASS_UID,
            Vr::Ui,
            Value::from_str(&sop_class),
        );
        meta.set(
            tags::MEDIA_STORAGE_SOP_INSTANCE_UID,
            Vr::Ui,
            Value::from_str(&sop_instance),
        );
        meta.set(
            tags::TRANSFER_SYNTAX_UID,
            Vr::Ui,
            Value::from_str(syntax.uid()),
        );
        meta.set(
            tags::IMPLEMENTATION_CLASS_UID,
            Vr::Ui,
            Value::from_str(IMPLEMENTATION_CLASS_UID),
        );
        meta.set(
            tags::IMPLEMENTATION_VERSION_NAME,
            Vr::Sh,
            Value::from_str(&implementation_version_name()),
        );
        Ok(Self {
            meta,
            dataset,
            preamble: [0u8; PREAMBLE_LENGTH],
            source: None,
            deferred: Vec::new(),
        })
    }

    /// Assemble a file from parts that are already built.
    ///
    /// [`DicomFile::new`] derives the meta information from the data set; this takes it as it
    /// stands, which is what a caller that built the meta information elsewhere has.
    pub fn from_parts(meta: DataSet, dataset: DataSet, preamble: [u8; PREAMBLE_LENGTH]) -> Self {
        Self {
            meta,
            dataset,
            preamble,
            source: None,
            deferred: Vec::new(),
        }
    }

    /// The elements whose values were left out of the data set because they reached
    /// [`ReadOptions::defer_size`].
    pub fn deferred_elements(&self) -> &[DeferredElement] {
        &self.deferred
    }

    /// Whether `tag` was deferred.
    pub fn is_deferred(&self, tag: Tag) -> bool {
        self.deferred.iter().any(|element| element.tag == tag)
    }

    /// Read one deferred element back into the data set.
    ///
    /// A tag that was not deferred is returned as it stands, so this is safe to call for any
    /// tag. Reading again re-parses the file the element came from.
    pub fn load_element(&mut self, tag: Tag) -> Result<&Element> {
        if !self.is_deferred(tag) {
            return self.dataset.element(tag);
        }
        let bytes = self.source_bytes()?;
        let fresh = Self::read_inner(&bytes, None, ReadOptions::default())?;
        let element = fresh.dataset.element(tag)?.clone();
        self.dataset.insert(element);
        self.deferred.retain(|deferred| deferred.tag != tag);
        self.dataset.element(tag)
    }

    /// Read every deferred element back, reporting how many came back.
    pub fn load_deferred(&mut self) -> Result<usize> {
        if self.deferred.is_empty() {
            return Ok(0);
        }
        let bytes = self.source_bytes()?;
        let fresh = Self::read_inner(&bytes, None, ReadOptions::default())?;
        let tags: Vec<Tag> = self.deferred.iter().map(|deferred| deferred.tag).collect();
        let mut loaded = 0;
        for tag in &tags {
            if let Some(element) = fresh.dataset.get(*tag) {
                self.dataset.insert(element.clone());
                loaded += 1;
            }
        }
        self.deferred
            .retain(|deferred| !tags.contains(&deferred.tag));
        Ok(loaded)
    }

    fn source_bytes(&self) -> Result<Vec<u8>> {
        match &self.source {
            Some(Source::Bytes(bytes)) => Ok(bytes.clone()),
            Some(Source::Path(path)) => {
                std::fs::read(path).map_err(|err| Error::Io(err.to_string()))
            }
            None => Err(Error::InvalidData {
                offset: 0,
                reason: "the file of a deferred element is gone",
            }),
        }
    }

    /// Encode this file, preamble and prefix included.
    pub fn write(&self) -> Result<Vec<u8>> {
        let syntax = self.transfer_syntax()?;
        // PS3.10 restricts the file meta information to group 2; pydicom's
        // `write_file_meta_info` rejects anything else.
        if let Some(element) = self
            .meta
            .iter()
            .find(|element| element.tag.group() != 0x0002)
        {
            return Err(Error::InvalidMeta(element.tag));
        }
        let mut out = self.preamble.to_vec();
        out.extend_from_slice(b"DICM");
        write_data_set(&self.meta, TransferSyntax::ExplicitVrLittleEndian, &mut out)?;
        out.extend_from_slice(&self.dataset.write(syntax)?);
        Ok(out)
    }

    /// Write this file to `path`, creating or truncating it.
    ///
    /// The bytes are the same ones [`write`](Self::write) returns; this is the one-call form
    /// pydicom's `save_as` provides for a path.
    pub fn write_file(&self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::write(path, self.write()?).map_err(|err| Error::Io(err.to_string()))
    }
}

/// Files are equal when their meta information and main data set are.
///
/// The preamble is a transport detail PS3.10 gives no meaning to, so two files that differ only
/// there hold the same DICOM content and compare equal.
impl PartialEq for DicomFile {
    fn eq(&self, other: &Self) -> bool {
        self.meta == other.meta && self.dataset == other.dataset
    }
}

impl Eq for DicomFile {}

fn transfer_syntax_from_meta(meta: &DataSet) -> Result<TransferSyntax> {
    let Some(element) = meta.get(tags::TRANSFER_SYNTAX_UID) else {
        // pydicom assumes the default transfer syntax when a file meta group carries no
        // Transfer Syntax UID (`test_no_transfer_syntax_in_meta`).
        return Ok(TransferSyntax::ImplicitVrLittleEndian);
    };
    let uid = element.string()?;
    TransferSyntax::from_uid(uid).ok_or_else(|| Error::UnsupportedTransferSyntax(uid.to_owned()))
}

fn implementation_version_name() -> String {
    format!("DCM4RS_{}", env!("CARGO_PKG_VERSION").replace('.', "_"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file whose Pixel Data holds 400 bytes, so a defer size can single it out.
    fn image() -> DicomFile {
        let mut set = DataSet::new();
        set.set_string(tags::SOP_CLASS_UID, Vr::Ui, "1.2.840.10008.5.1.4.1.1.2");
        set.set_string(tags::SOP_INSTANCE_UID, Vr::Ui, "1.2.3");
        set.set(Tag::new(0x0028, 0x0002), Vr::Us, Value::from_u16s(&[1]));
        set.set(
            Tag::new(0x0028, 0x0004),
            Vr::Cs,
            Value::from_str("MONOCHROME2"),
        );
        set.set(Tag::new(0x0028, 0x0010), Vr::Us, Value::from_u16s(&[2]));
        set.set(Tag::new(0x0028, 0x0011), Vr::Us, Value::from_u16s(&[2]));
        set.set(Tag::new(0x0028, 0x0100), Vr::Us, Value::from_u16s(&[8]));
        set.set(Tag::new(0x0028, 0x0101), Vr::Us, Value::from_u16s(&[8]));
        set.set(Tag::new(0x0028, 0x0102), Vr::Us, Value::from_u16s(&[7]));
        set.set(Tag::new(0x0028, 0x0103), Vr::Us, Value::from_u16s(&[0]));
        set.set(tags::PIXEL_DATA, Vr::Ob, Value::from_bytes(vec![0u8; 400]));
        DicomFile::new(set, TransferSyntax::ExplicitVrLittleEndian).expect("file")
    }

    fn deferred_options() -> ReadOptions {
        ReadOptions {
            defer_size: 200,
            ..ReadOptions::default()
        }
    }

    #[test]
    fn a_large_value_is_left_out_until_it_is_loaded() {
        let bytes = image().write().expect("bytes");
        let mut file = DicomFile::read_with_options(&bytes, deferred_options()).expect("read");

        assert!(file.is_deferred(tags::PIXEL_DATA));
        assert!(!file.dataset.contains(tags::PIXEL_DATA));
        assert_eq!(file.deferred_elements().len(), 1);
        assert_eq!(file.deferred_elements()[0].length, 400);
        // The small elements are all there.
        assert!(file.dataset.contains(Tag::new(0x0028, 0x0010)));

        let element = file.load_element(tags::PIXEL_DATA).expect("loaded");
        assert_eq!(element.raw_value().len(), 400);
        assert!(file.dataset.contains(tags::PIXEL_DATA));
        assert!(file.deferred_elements().is_empty());
    }

    #[test]
    fn load_deferred_brings_back_every_element() {
        let bytes = image().write().expect("bytes");
        let mut file = DicomFile::read_with_options(&bytes, deferred_options()).expect("read");

        assert_eq!(file.load_deferred().expect("loaded"), 1);
        assert!(file.deferred_elements().is_empty());
        assert!(file.dataset.contains(tags::PIXEL_DATA));
        // The deferred value is the one the file carried, not a placeholder.
        assert_eq!(
            file.dataset
                .element(tags::PIXEL_DATA)
                .expect("pixel data")
                .raw_value()
                .len(),
            400
        );
    }

    #[test]
    fn the_default_reads_every_value() {
        let bytes = image().write().expect("bytes");
        let file = DicomFile::read(&bytes).expect("read");
        assert!(file.dataset.contains(tags::PIXEL_DATA));
        assert!(file.deferred_elements().is_empty());
    }

    #[test]
    fn a_tag_that_was_never_deferred_loads_as_it_stands() {
        let bytes = image().write().expect("bytes");
        let mut file = DicomFile::read_with_options(&bytes, deferred_options()).expect("read");
        let rows = Tag::new(0x0028, 0x0010);
        assert_eq!(file.load_element(rows).expect("rows").byte_len(), 2);
    }
}
