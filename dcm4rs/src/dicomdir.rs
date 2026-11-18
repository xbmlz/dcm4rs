//! Media storage directories: the DICOMDIR (PS3.10 8.5, PS3.3 F.5.1).
//!
//! A DICOMDIR is a Part 10 file whose data set carries the File-set attributes and, in
//! `(0004,1220)` Directory Record Sequence, the records that index the files of the File-set.
//! PS3.10 stores the records in a pre-order traversal, and each record says with `(0004,1420)`
//! whether its children follow and with `(0004,1400)` whether a sibling follows, so the tree is
//! rebuilt from those two flags: the byte offsets a writer computed are not needed to read it.

use std::path::Path;

use crate::dataset::DataSet;
use crate::dictionary::tags;
use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::file::DicomFile;
use crate::tag::Tag;

/// SOP Class UID of a media storage directory (`1.2.840.10008.1.3.10`).
pub const MEDIA_STORAGE_DIRECTORY_STORAGE: &str = "1.2.840.10008.1.3.10";

/// A directory record and the records below it.
#[derive(Clone, Debug)]
pub struct DirectoryRecord<'a> {
    /// The record's own attributes.
    pub dataset: &'a DataSet,
    /// The records its `(0004,1420)` points at, in order.
    pub children: Vec<DirectoryRecord<'a>>,
}

impl<'a> DirectoryRecord<'a> {
    /// `(0004,1430)` Directory Record Type, or `None` when the element is absent.
    pub fn record_type(&self) -> Result<Option<&'a str>> {
        match self.dataset.get(tags::DIRECTORY_RECORD_TYPE) {
            Some(element) => Ok(Some(element.string()?)),
            None => Ok(None),
        }
    }

    /// `(0004,1500)` Referenced File ID, its components joined by `\`.
    ///
    /// The attribute is multi-valued, one component per path segment, so it is read through
    /// [`Element::string_values`] rather than [`Element::string`], which would report the first
    /// component alone.
    pub fn referenced_file(&self) -> Result<Option<String>> {
        match self.dataset.get(tags::REFERENCED_FILE_ID) {
            Some(element) => Ok(Some(element.string_values()?.join("\\"))),
            None => Ok(None),
        }
    }

    /// The number of records at this level and below.
    pub fn count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(|child| child.count())
            .sum::<usize>()
    }
}

/// A media storage directory: the File-set attributes plus the directory records.
#[derive(Clone, Debug)]
pub struct DicomDir {
    file: DicomFile,
}

impl DicomDir {
    /// Read a DICOMDIR from the bytes of a Part 10 file.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        Self::from_file(DicomFile::read(bytes)?)
    }

    /// Read a DICOMDIR from disk.
    pub fn read_file(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_file(DicomFile::read_file(path)?)
    }

    /// Wrap a file whose SOP Class UID is Media Storage Directory Storage.
    pub fn from_file(file: DicomFile) -> Result<Self> {
        let sop_class = file.dataset.string(tags::SOP_CLASS_UID)?;
        if sop_class.trim() != MEDIA_STORAGE_DIRECTORY_STORAGE {
            return Err(Error::InvalidData {
                offset: 0,
                reason: "SOP Class UID is not a media storage directory",
            });
        }
        Ok(Self { file })
    }

    /// The underlying file.
    pub fn file(&self) -> &DicomFile {
        &self.file
    }

    /// `(0004,1130)` File-set ID, or `None` when the element is absent.
    pub fn file_set_id(&self) -> Result<Option<&str>> {
        match self.file.dataset.get(tags::FILE_SET_ID) {
            Some(element) => Ok(Some(element.string()?)),
            None => Ok(None),
        }
    }

    /// The records of `(0004,1220)`, in the order the file stores them.
    pub fn records(&self) -> Result<&[DataSet]> {
        self.file
            .dataset
            .element(tags::DIRECTORY_RECORD_SEQUENCE)?
            .as_sequence()
    }

    /// The root records, each with the records below it.
    pub fn roots(&self) -> Result<Vec<DirectoryRecord<'_>>> {
        let records = self.records()?;
        if records.is_empty() {
            return Ok(Vec::new());
        }
        let endian = self.file.dataset.endian();
        let mut index = 0;
        let roots = parse_chain(records, &mut index, endian)?;
        // PS3.10 stores the whole tree in one pre-order traversal, so a chain that stops early
        // leaves records the flags never reached.
        if index != records.len() {
            return Err(Error::InvalidData {
                offset: 0,
                reason: "the record chains do not cover every record",
            });
        }
        Ok(roots)
    }
}

/// Read one level of the tree: the records that are siblings of one another.
fn parse_chain<'a>(
    records: &'a [DataSet],
    index: &mut usize,
    endian: Endian,
) -> Result<Vec<DirectoryRecord<'a>>> {
    let mut chain = Vec::new();
    loop {
        let record = records.get(*index).ok_or(Error::InvalidData {
            offset: 0,
            reason: "a record chain ends past the last directory record",
        })?;
        *index += 1;
        let children = if has_offset(
            record,
            tags::OFFSET_OF_REFERENCED_LOWER_LEVEL_DIRECTORY_ENTITY,
            endian,
        )? {
            parse_chain(records, index, endian)?
        } else {
            Vec::new()
        };
        let has_next = has_offset(record, tags::OFFSET_OF_THE_NEXT_DIRECTORY_RECORD, endian)?;
        chain.push(DirectoryRecord {
            dataset: record,
            children,
        });
        if !has_next {
            return Ok(chain);
        }
    }
}

/// Whether an offset attribute of a record is present and non-zero.
fn has_offset(record: &DataSet, tag: Tag, endian: Endian) -> Result<bool> {
    match record.get(tag) {
        Some(element) => Ok(element.u32s(endian)?.first().copied().unwrap_or(0) != 0),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer_syntax::TransferSyntax;
    use crate::value::Value;
    use crate::vr::Vr;

    fn record(kind: &str, file: Option<&str>, child: bool, next: bool) -> DataSet {
        let mut record = DataSet::new();
        record.set(
            tags::OFFSET_OF_THE_NEXT_DIRECTORY_RECORD,
            Vr::Ul,
            Value::from_u32s(&[u32::from(next)]),
        );
        record.set(
            tags::RECORD_IN_USE_FLAG,
            Vr::Us,
            Value::from_u16s(&[0xFFFF]),
        );
        record.set(
            tags::OFFSET_OF_REFERENCED_LOWER_LEVEL_DIRECTORY_ENTITY,
            Vr::Ul,
            Value::from_u32s(&[u32::from(child)]),
        );
        record.set_string(tags::DIRECTORY_RECORD_TYPE, Vr::Cs, kind);
        if let Some(file) = file {
            record.set_string(tags::REFERENCED_FILE_ID, Vr::Cs, file);
        }
        record
    }

    /// PATIENT > STUDY > SERIES > (IMAGE, IMAGE).
    fn directory() -> DicomDir {
        let mut set = DataSet::new();
        set.set_string(tags::SOP_CLASS_UID, Vr::Ui, MEDIA_STORAGE_DIRECTORY_STORAGE);
        set.set_string(tags::SOP_INSTANCE_UID, Vr::Ui, "1.2.3");
        set.set_string(tags::FILE_SET_ID, Vr::Cs, "FILMSET");
        let records = vec![
            record("PATIENT", None, true, false),
            record("STUDY", None, true, false),
            record("SERIES", None, true, false),
            record("IMAGE", Some("IMAGES\\A"), false, true),
            record("IMAGE", Some("IMAGES\\B"), false, false),
        ];
        set.set(
            tags::DIRECTORY_RECORD_SEQUENCE,
            Vr::Sq,
            Value::sequence(records),
        );
        let file = DicomFile::new(set, TransferSyntax::ExplicitVrLittleEndian).expect("file");
        DicomDir::from_file(file).expect("directory")
    }

    #[test]
    fn the_records_form_a_tree_from_the_flags() {
        let directory = directory();
        let roots = directory.roots().expect("roots");

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].record_type().expect("type"), Some("PATIENT"));
        assert_eq!(roots[0].count(), 5);

        let study = &roots[0].children[0];
        assert_eq!(study.record_type().expect("type"), Some("STUDY"));
        let series = &study.children[0];
        assert_eq!(series.record_type().expect("type"), Some("SERIES"));
        assert_eq!(series.children.len(), 2);
        assert_eq!(
            series.children[1]
                .referenced_file()
                .expect("file")
                .as_deref(),
            Some("IMAGES\\B")
        );
    }

    #[test]
    fn the_file_set_id_is_kept() {
        let directory = directory();
        assert_eq!(directory.file_set_id().expect("id"), Some("FILMSET"));
    }

    #[test]
    fn a_file_that_is_not_a_directory_is_refused() {
        let mut set = DataSet::new();
        set.set_string(tags::SOP_CLASS_UID, Vr::Ui, "1.2.840.10008.5.1.4.1.1.2");
        set.set_string(tags::SOP_INSTANCE_UID, Vr::Ui, "1.2.3");
        let file = DicomFile::new(set, TransferSyntax::ExplicitVrLittleEndian).expect("file");
        assert!(DicomDir::from_file(file).is_err());
    }

    #[test]
    fn a_chain_that_runs_past_the_last_record_is_reported() {
        let mut set = DataSet::new();
        set.set_string(tags::SOP_CLASS_UID, Vr::Ui, MEDIA_STORAGE_DIRECTORY_STORAGE);
        set.set_string(tags::SOP_INSTANCE_UID, Vr::Ui, "1.2.3");
        // The only record claims a sibling that is not there.
        set.set(
            tags::DIRECTORY_RECORD_SEQUENCE,
            Vr::Sq,
            Value::sequence(vec![record("PATIENT", None, false, true)]),
        );
        let file = DicomFile::new(set, TransferSyntax::ExplicitVrLittleEndian).expect("file");
        let directory = DicomDir::from_file(file).expect("directory");
        assert!(directory.roots().is_err());
    }

    #[test]
    fn a_record_without_a_type_reports_none() {
        let mut record = DataSet::new();
        record.set(
            tags::OFFSET_OF_THE_NEXT_DIRECTORY_RECORD,
            Vr::Ul,
            Value::from_u32s(&[0]),
        );
        let mut set = DataSet::new();
        set.set_string(tags::SOP_CLASS_UID, Vr::Ui, MEDIA_STORAGE_DIRECTORY_STORAGE);
        set.set_string(tags::SOP_INSTANCE_UID, Vr::Ui, "1.2.3");
        set.set(
            tags::DIRECTORY_RECORD_SEQUENCE,
            Vr::Sq,
            Value::sequence(vec![record]),
        );
        let file = DicomFile::new(set, TransferSyntax::ExplicitVrLittleEndian).expect("file");
        let directory = DicomDir::from_file(file).expect("directory");
        let roots = directory.roots().expect("roots");
        assert_eq!(roots[0].record_type().expect("type"), None);
    }

    #[test]
    fn a_directory_round_trips_through_bytes() {
        let directory = directory();
        let bytes = directory.file().write().expect("bytes");
        let read = DicomDir::read(&bytes).expect("directory");
        let roots = read.roots().expect("roots");
        assert_eq!(roots[0].count(), 5);
    }
}
