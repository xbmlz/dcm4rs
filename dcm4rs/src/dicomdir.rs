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
use crate::transfer_syntax::TransferSyntax;
use crate::value::Value;
use crate::vr::Vr;

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

/// A record and the records below it, for building a directory.
#[derive(Clone, Debug)]
pub struct DirectoryEntry {
    /// The record's own attributes.
    pub record: DataSet,
    /// The records to store below this one.
    pub children: Vec<DirectoryEntry>,
}

impl DirectoryEntry {
    /// A record with no children.
    pub fn new(record: DataSet) -> Self {
        Self {
            record,
            children: Vec::new(),
        }
    }
}

/// One record in the pre-order layout the file will store.
struct FlatNode {
    dataset: DataSet,
    next: Option<usize>,
    child: Option<usize>,
}

struct Flat {
    nodes: Vec<FlatNode>,
    roots: Vec<usize>,
}

impl DicomDir {
    /// Build a directory whose records are stored in one pre-order traversal.
    ///
    /// The offsets are placeholders until [`DicomDir::write`] lays the records out; the flags
    /// that say whether a record has children or a sibling are set now.
    pub fn build(
        file_set_id: &str,
        sop_instance_uid: &str,
        records: Vec<DirectoryEntry>,
    ) -> Result<Self> {
        let mut set = DataSet::new();
        set.set_string(tags::SOP_CLASS_UID, Vr::Ui, MEDIA_STORAGE_DIRECTORY_STORAGE);
        set.set_string(tags::SOP_INSTANCE_UID, Vr::Ui, sop_instance_uid);
        set.set_string(tags::FILE_SET_ID, Vr::Cs, file_set_id);
        set.set(
            tags::FILE_SET_CONSISTENCY_FLAG,
            Vr::Us,
            Value::from_u16s(&[0]),
        );
        set.set(
            tags::OFFSET_OF_THE_FIRST_DIRECTORY_RECORD_OF_THE_ROOT_DIRECTORY_ENTITY,
            Vr::Ul,
            Value::from_u32s(&[0]),
        );
        set.set(
            tags::OFFSET_OF_THE_LAST_DIRECTORY_RECORD_OF_THE_ROOT_DIRECTORY_ENTITY,
            Vr::Ul,
            Value::from_u32s(&[0]),
        );
        let flat = flatten(&records);
        let stored = flat
            .nodes
            .iter()
            .map(|node| {
                let mut record = node.dataset.clone();
                record.set(
                    tags::OFFSET_OF_THE_NEXT_DIRECTORY_RECORD,
                    Vr::Ul,
                    Value::from_u32s(&[u32::from(node.next.is_some())]),
                );
                record.set(
                    tags::OFFSET_OF_REFERENCED_LOWER_LEVEL_DIRECTORY_ENTITY,
                    Vr::Ul,
                    Value::from_u32s(&[u32::from(node.child.is_some())]),
                );
                record
            })
            .collect();
        set.set(
            tags::DIRECTORY_RECORD_SEQUENCE,
            Vr::Sq,
            Value::sequence(stored),
        );
        let file = DicomFile::new(set, TransferSyntax::ExplicitVrLittleEndian)?;
        Self::from_file(file)
    }

    /// Encode the directory with the byte offsets computed from the record order.
    ///
    /// PS3.10 measures the offsets from the beginning of the file, so the layout is worked out
    /// first and the records are then written with the offsets they landed at. The offset
    /// attributes are `UL`, so filling them in changes no length and the layout stays valid.
    pub fn write(&self) -> Result<Vec<u8>> {
        let syntax = TransferSyntax::ExplicitVrLittleEndian;
        let tree = self.owned_tree()?;
        let flat = flatten(&tree);

        let mut meta = Vec::new();
        crate::writer::write_data_set(&self.file.meta, syntax, &mut meta)?;

        let mut base = self.file.dataset.clone();
        base.remove(tags::DIRECTORY_RECORD_SEQUENCE);
        // Every offset attribute exists before the first pass, so filling it in cannot change
        // the layout.
        base.set(
            tags::OFFSET_OF_THE_FIRST_DIRECTORY_RECORD_OF_THE_ROOT_DIRECTORY_ENTITY,
            Vr::Ul,
            Value::from_u32s(&[0]),
        );
        base.set(
            tags::OFFSET_OF_THE_LAST_DIRECTORY_RECORD_OF_THE_ROOT_DIRECTORY_ENTITY,
            Vr::Ul,
            Value::from_u32s(&[0]),
        );

        let mut payloads = Vec::with_capacity(flat.nodes.len());
        for node in &flat.nodes {
            payloads.push(record_payload(&node.dataset, syntax)?);
        }

        let mut base_buf = Vec::new();
        crate::writer::write_data_set(&base, syntax, &mut base_buf)?;
        // preamble (128) + "DICM" (4) + meta + the data set before the sequence + the sequence
        // element's header (tag, VR, reserved, length) of 12 bytes.
        // `sequence_start` already points past the sequence element's header, at its first item.
        let sequence_start = PREAMBLE_LENGTH + 4 + meta.len() + base_buf.len() + 12;
        let mut starts = Vec::with_capacity(payloads.len());
        let mut cursor = sequence_start;
        for payload in &payloads {
            starts.push(cursor as u32);
            cursor += payload.len() + 8;
        }

        // Second pass: the offsets the records will be read back at.
        let mut payloads = Vec::with_capacity(flat.nodes.len());
        for node in &flat.nodes {
            let mut record = node.dataset.clone();
            let next = node.next.map_or(0, |index| starts[index]);
            let child = node.child.map_or(0, |index| starts[index]);
            record.set(
                tags::OFFSET_OF_THE_NEXT_DIRECTORY_RECORD,
                Vr::Ul,
                Value::from_u32s(&[next]),
            );
            record.set(
                tags::OFFSET_OF_REFERENCED_LOWER_LEVEL_DIRECTORY_ENTITY,
                Vr::Ul,
                Value::from_u32s(&[child]),
            );
            payloads.push(record_payload(&record, syntax)?);
        }

        let first = flat.roots.first().map_or(0, |index| starts[*index]);
        let last = flat.roots.last().map_or(0, |index| starts[*index]);
        base.set(
            tags::OFFSET_OF_THE_FIRST_DIRECTORY_RECORD_OF_THE_ROOT_DIRECTORY_ENTITY,
            Vr::Ul,
            Value::from_u32s(&[first]),
        );
        base.set(
            tags::OFFSET_OF_THE_LAST_DIRECTORY_RECORD_OF_THE_ROOT_DIRECTORY_ENTITY,
            Vr::Ul,
            Value::from_u32s(&[last]),
        );
        let mut base_buf = Vec::new();
        crate::writer::write_data_set(&base, syntax, &mut base_buf)?;

        let mut out = Vec::with_capacity(
            sequence_start + 12 + payloads.iter().map(Vec::len).sum::<usize>() + 8,
        );
        out.extend_from_slice(&[0u8; PREAMBLE_LENGTH]);
        out.extend_from_slice(b"DICM");
        out.extend_from_slice(&meta);
        out.extend_from_slice(&base_buf);
        // The Directory Record Sequence, written with an undefined length because its offsets
        // are computed from this very layout.
        out.extend_from_slice(&[0x04, 0x00, 0x20, 0x12]);
        out.extend_from_slice(b"SQ");
        out.extend_from_slice(&[0x00, 0x00]);
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        for payload in &payloads {
            out.extend_from_slice(&[0xFE, 0xFF, 0x00, 0xE0]);
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(payload);
        }
        out.extend_from_slice(&[0xFE, 0xFF, 0xDD, 0xE0, 0x00, 0x00, 0x00, 0x00]);
        Ok(out)
    }

    /// The records as owned entries, in the order the file stores them.
    fn owned_tree(&self) -> Result<Vec<DirectoryEntry>> {
        Ok(self.roots()?.iter().map(owned_entry).collect())
    }
}

fn owned_entry(node: &DirectoryRecord<'_>) -> DirectoryEntry {
    DirectoryEntry {
        record: node.dataset.clone(),
        children: node.children.iter().map(owned_entry).collect(),
    }
}

fn record_payload(record: &DataSet, syntax: TransferSyntax) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    crate::writer::write_data_set(record, syntax, &mut payload)?;
    Ok(payload)
}

fn flatten(entries: &[DirectoryEntry]) -> Flat {
    let mut nodes = Vec::new();
    let roots = place_chain(entries, &mut nodes);
    Flat { nodes, roots }
}

/// Assign the pre-order index of every record, linking each to its sibling and its first child.
fn place_chain(entries: &[DirectoryEntry], nodes: &mut Vec<FlatNode>) -> Vec<usize> {
    let mut indices = Vec::new();
    for entry in entries {
        let index = nodes.len();
        nodes.push(FlatNode {
            dataset: entry.record.clone(),
            next: None,
            child: None,
        });
        let children = place_chain(&entry.children, nodes);
        if let Some(first) = children.first() {
            nodes[index].child = Some(*first);
        }
        indices.push(index);
    }
    for pair in indices.windows(2) {
        nodes[pair[0]].next = Some(pair[1]);
    }
    indices
}

const PREAMBLE_LENGTH: usize = 128;

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

    fn entry(kind: &str, children: Vec<DirectoryEntry>) -> DirectoryEntry {
        let mut record = DataSet::new();
        record.set_string(tags::DIRECTORY_RECORD_TYPE, Vr::Cs, kind);
        DirectoryEntry { record, children }
    }

    /// PATIENT > STUDY > SERIES > (IMAGE, IMAGE).
    fn tree() -> Vec<DirectoryEntry> {
        vec![entry(
            "PATIENT",
            vec![entry(
                "STUDY",
                vec![entry(
                    "SERIES",
                    vec![entry("IMAGE", Vec::new()), entry("IMAGE", Vec::new())],
                )],
            )],
        )]
    }

    fn offset(record: &DataSet, tag: Tag) -> u32 {
        record
            .element(tag)
            .unwrap_or_else(|_| panic!("{tag} is missing"))
            .u32s(Endian::Little)
            .expect("offset")[0]
    }

    fn item_positions(bytes: &[u8]) -> Vec<usize> {
        bytes
            .windows(4)
            .enumerate()
            .filter(|(_, window)| window == &[0xFE, 0xFF, 0x00, 0xE0])
            .map(|(index, _)| index)
            .collect()
    }

    #[test]
    fn a_built_directory_round_trips_with_its_tree() {
        let directory = DicomDir::build("FILMSET", "1.2.3", tree()).expect("build");
        let bytes = directory.write().expect("write");

        let read = DicomDir::read(&bytes).expect("read");
        assert_eq!(read.file_set_id().expect("id"), Some("FILMSET"));
        let roots = read.roots().expect("roots");
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].record_type().expect("type"), Some("PATIENT"));
        assert_eq!(roots[0].count(), 5);
        assert_eq!(
            roots[0].children[0].children[0].children.len(),
            2,
            "the two IMAGE records sit under the SERIES"
        );
    }

    #[test]
    fn the_offsets_point_at_the_items_they_name() {
        let directory = DicomDir::build("FILMSET", "1.2.3", tree()).expect("build");
        let bytes = directory.write().expect("write");
        let read = DicomDir::read(&bytes).expect("read");
        let items = item_positions(&bytes);
        assert_eq!(items.len(), 5, "one item per record");

        let first = offset(
            &read.file().dataset,
            tags::OFFSET_OF_THE_FIRST_DIRECTORY_RECORD_OF_THE_ROOT_DIRECTORY_ENTITY,
        ) as usize;
        assert_eq!(first, items[0], "the root record's offset is its item");

        let records = read.records().expect("records");
        // PATIENT has a child: the STUDY item right after it.
        assert_eq!(
            offset(
                &records[0],
                tags::OFFSET_OF_REFERENCED_LOWER_LEVEL_DIRECTORY_ENTITY
            ) as usize,
            items[1]
        );
        // STUDY has no sibling.
        assert_eq!(
            offset(&records[1], tags::OFFSET_OF_THE_NEXT_DIRECTORY_RECORD),
            0
        );
        // SERIES has a child, and its first IMAGE has a sibling: the second IMAGE.
        assert_eq!(
            offset(
                &records[2],
                tags::OFFSET_OF_REFERENCED_LOWER_LEVEL_DIRECTORY_ENTITY
            ) as usize,
            items[3]
        );
        assert_eq!(
            offset(&records[3], tags::OFFSET_OF_THE_NEXT_DIRECTORY_RECORD) as usize,
            items[4]
        );
    }

    #[test]
    fn an_empty_directory_writes_offsets_of_zero() {
        let directory = DicomDir::build("FILMSET", "1.2.3", Vec::new()).expect("build");
        let bytes = directory.write().expect("write");
        let read = DicomDir::read(&bytes).expect("read");
        assert!(read.roots().expect("roots").is_empty());
        assert_eq!(
            offset(
                &read.file().dataset,
                tags::OFFSET_OF_THE_FIRST_DIRECTORY_RECORD_OF_THE_ROOT_DIRECTORY_ENTITY
            ),
            0
        );
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
