use std::path::Path;

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::error::{Error, Result};
use crate::private_dict::PrivateDictionary;
use crate::read_options::ReadOptions;
use crate::reader::Reader;
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
            meta.insert(reader.read_element()?);
        }
        let dataset_start = meta_start + reader.position();
        let syntax = transfer_syntax_from_meta(&meta)?;
        let dataset =
            DataSet::read_inner(&bytes[dataset_start..], syntax, private_dictionary, options)?;
        Ok(Self {
            meta,
            dataset,
            preamble,
        })
    }

    /// Read a Part 10 file from disk.
    pub fn read_file(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|err| Error::Io(err.to_string()))?;
        Self::read(&bytes)
    }

    /// Read a Part 10 file from disk, applying [`ReadOptions`].
    pub fn read_file_with_options(path: impl AsRef<Path>, options: ReadOptions) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|err| Error::Io(err.to_string()))?;
        Self::read_with_options(&bytes, options)
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
        })
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
