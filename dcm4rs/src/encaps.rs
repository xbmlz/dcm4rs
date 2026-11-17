//! Encapsulated pixel data: fragments, items and the offset tables.
//!
//! Compressed transfer syntaxes carry their frames as fragments. Every fragment is an item with
//! tag (FFFE,E000), and the items are preceded by the Basic Offset Table item, which records the
//! byte offset of each frame's first item. This module works on the raw Pixel Data value and
//! mirrors `pydicom.encaps`: [`encapsulate`] and [`encapsulate_extended`] build the value,
//! [`parse_basic_offsets`], [`parse_fragments`], [`generate_fragments`] and [`defragment`] take
//! it apart.
//!
//! Items are always written little endian (PS3.5, A.4); the parsers accept both byte orders
//! because a big-endian data set still carries big-endian item tags.

use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::tag::Tag;

/// The item tag that wraps every fragment, the Basic Offset Table included.
pub const ITEM: Tag = Tag::new(0xFFFE, 0xE000);

/// The sequence delimiter that ends the encapsulated stream.
pub const SEQUENCE_DELIMITER: Tag = Tag::new(0xFFFE, 0xE0DD);

/// Wrap one fragment in an item.
///
/// The item is eight bytes of tag and length followed by the fragment; the caller keeps the
/// fragment an even number of bytes long, as PS3.5 requires of item values.
pub fn itemize_fragment(fragment: &[u8]) -> Vec<u8> {
    let mut item = Vec::with_capacity(fragment.len() + 8);
    item.extend_from_slice(b"\xfe\xff\x00\xe0");
    item.extend_from_slice(&(fragment.len() as u32).to_le_bytes());
    item.extend_from_slice(fragment);
    item
}

/// Split one frame into `nr_fragments` fragments.
///
/// Each fragment is an even number of bytes; the last one is padded with a single zero byte when
/// the frame leaves an odd remainder, which is allowed after the end of image marker.
/// `nr_fragments` may not exceed half the frame length, the smallest fragment being two bytes.
pub fn fragment_frame(frame: &[u8], nr_fragments: usize) -> Result<Vec<Vec<u8>>> {
    if nr_fragments == 0 {
        return Err(Error::InvalidPixelData(
            "A frame cannot be split into zero fragments".to_owned(),
        ));
    }
    if nr_fragments * 2 > frame.len() + 1 {
        return Err(Error::InvalidPixelData(format!(
            "Too many fragments requested: {} fragments need at least {} bytes, the frame has {}",
            nr_fragments,
            nr_fragments * 2 - 1,
            frame.len()
        )));
    }
    let mut length = frame.len() / nr_fragments;
    if length % 2 != 0 {
        length += 1;
    }
    let mut fragments = Vec::with_capacity(nr_fragments);
    for index in 0..nr_fragments - 1 {
        let start = index * length;
        let end = (start + length).min(frame.len());
        fragments.push(frame[start..end].to_vec());
    }
    let mut last = frame[length * (nr_fragments - 1)..].to_vec();
    if last.len() % 2 != 0 {
        last.push(0);
    }
    fragments.push(last);
    Ok(fragments)
}

/// Itemise one frame, returning the items end to end.
///
/// Equivalent to [`fragment_frame`] followed by [`itemize_fragment`] on each fragment.
pub fn itemize_frame(frame: &[u8], nr_fragments: usize) -> Result<Vec<u8>> {
    let fragments = fragment_frame(frame, nr_fragments)?;
    let total: usize = fragments.iter().map(|fragment| fragment.len() + 8).sum();
    let mut items = Vec::with_capacity(total);
    for fragment in &fragments {
        items.extend_from_slice(&itemize_fragment(fragment));
    }
    Ok(items)
}

/// Encapsulate encoded frames into one Pixel Data value.
///
/// The value is the Basic Offset Table item followed by each frame's items. Offsets are measured
/// from the first fragment's item tag; an empty table leaves the frame boundaries implicit.
/// The sequence delimiter is not part of the value pydicom returns either: the writer adds it
/// when the element has undefined length.
pub fn encapsulate(
    frames: &[Vec<u8>],
    fragments_per_frame: usize,
    has_bot: bool,
) -> Result<Vec<u8>> {
    let nr_frames = frames.len();
    let mut output = Vec::new();
    output.extend_from_slice(b"\xfe\xff\x00\xe0");

    if has_bot {
        // The table's own length field is 32 bits, so every offset must fit; the last frame's
        // offset is the only one that can be omitted, hence the sum over `frames[..nr-1]`.
        let total = if nr_frames <= 1 {
            0u64
        } else {
            (nr_frames as u64 - 1) * 8
                + frames[..nr_frames - 1]
                    .iter()
                    .map(|frame| frame.len() as u64)
                    .sum::<u64>()
        };
        if total > u32::MAX as u64 {
            return Err(Error::InvalidPixelData(format!(
                "The total length of the encapsulated frame data ({total} bytes) will be \
                 greater than the maximum allowed by the Basic Offset Table ({}) bytes; use \
                 `encapsulate_extended` and the Extended Offset Table instead",
                u32::MAX
            )));
        }
        output.extend_from_slice(&(4 * nr_frames as u32).to_le_bytes());
        output.extend_from_slice(&vec![0xFFu8; 4 * nr_frames]);
    } else {
        output.extend_from_slice(&0u32.to_le_bytes());
    }

    let mut offsets = Vec::with_capacity(nr_frames);
    let mut offset = 0u64;
    for frame in frames {
        offsets.push(offset);
        let items = itemize_frame(frame, fragments_per_frame)?;
        offset += items.len() as u64;
        output.extend_from_slice(&items);
    }

    if has_bot {
        for (index, offset) in offsets.iter().enumerate() {
            let position = 8 + 4 * index;
            output[position..position + 4].copy_from_slice(&(*offset as u32).to_le_bytes());
        }
    }

    Ok(output)
}

/// Encapsulate frames without a Basic Offset Table and return the Extended Offset Table values.
///
/// The three values are the Pixel Data, the Extended Offset Table (7FE0,0001) and the Extended
/// Offset Table Lengths (7FE0,0002), both encoded as 64-bit little-endian integers. Unlike the
/// Basic Offset Table this one records every frame, so it is the answer when the encapsulated
/// data outgrows 32-bit offsets.
pub fn encapsulate_extended(frames: &[Vec<u8>]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    // Odd-length frames are padded to even length by `encapsulate`; the table records the
    // padded length, not the raw one.
    let lengths: Vec<u64> = frames
        .iter()
        .map(|frame| {
            let length = frame.len() as u64;
            length + length % 2
        })
        .collect();

    let mut offsets = Vec::with_capacity(lengths.len());
    let mut position = 0u64;
    for length in &lengths {
        offsets.push(position);
        position += length + 8;
    }

    let mut offset_table = Vec::with_capacity(offsets.len() * 8);
    for offset in &offsets {
        offset_table.extend_from_slice(&offset.to_le_bytes());
    }
    let mut length_table = Vec::with_capacity(lengths.len() * 8);
    for length in &lengths {
        length_table.extend_from_slice(&length.to_le_bytes());
    }

    Ok((encapsulate(frames, 1, false)?, offset_table, length_table))
}

/// Read the Basic Offset Table.
///
/// Returns the frame offsets and the position of the first fragment's item tag in `value`.
/// Offsets are measured from that position; an empty table yields no offsets.
pub fn parse_basic_offsets(value: &[u8], endian: Endian) -> Result<(Vec<u32>, usize)> {
    let tag = read_tag_at(value, 0, endian).ok_or_else(|| {
        Error::InvalidPixelData(
            "The encapsulated pixel data ends before the Basic Offset Table item tag".to_owned(),
        )
    })?;
    if tag != ITEM {
        return Err(Error::InvalidPixelData(format!(
            "Found unexpected tag {tag} instead of (FFFE,E000) when parsing the Basic Offset Table \
             item"
        )));
    }
    let length = read_u32_at(value, 4, endian).ok_or_else(|| {
        Error::InvalidPixelData(
            "The encapsulated pixel data ends before the Basic Offset Table length".to_owned(),
        )
    })? as usize;
    if length % 4 != 0 {
        return Err(Error::InvalidPixelData(
            "The length of the Basic Offset Table item is not a multiple of 4".to_owned(),
        ));
    }
    let end = 8 + length;
    let data = value.get(8..end).ok_or_else(|| {
        Error::InvalidPixelData(format!(
            "The Basic Offset Table declares {length} bytes but the data holds {}",
            value.len().saturating_sub(8)
        ))
    })?;
    let offsets = data
        .chunks_exact(4)
        .map(|chunk| endian.u32([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    Ok((offsets, end))
}

/// Count the fragments and locate each fragment's item tag.
///
/// `value` starts at the first fragment's item tag, usually just after the Basic Offset Table.
/// The returned positions are absolute byte offsets into `value`; iteration stops at the
/// sequence delimiter.
pub fn parse_fragments(value: &[u8], endian: Endian) -> Result<(usize, Vec<usize>)> {
    let mut offsets = Vec::new();
    let mut position = 0usize;
    while let Some(tag) = read_tag_at(value, position, endian) {
        match tag {
            ITEM => {
                let length = item_length(value, position, endian)?;
                offsets.push(position);
                position += 8 + length;
            }
            SEQUENCE_DELIMITER => break,
            other => {
                return Err(Error::InvalidPixelData(format!(
                    "Unexpected tag '{other}' at offset {position} when parsing the encapsulated \
                     pixel data fragment items"
                )));
            }
        }
    }
    Ok((offsets.len(), offsets))
}

/// Return each fragment's payload.
///
/// `value` starts at the first fragment's item tag, and iteration stops at the sequence
/// delimiter. A fragment that runs past the end of the data is an error rather than a short
/// payload, which is where this departs from pydicom's silent truncation.
pub fn generate_fragments(value: &[u8], endian: Endian) -> Result<Vec<&[u8]>> {
    let mut fragments = Vec::new();
    let mut position = 0usize;
    while let Some(tag) = read_tag_at(value, position, endian) {
        match tag {
            ITEM => {
                let length = item_length(value, position, endian)?;
                let start = position + 8;
                let end = start + length;
                let fragment = value.get(start..end).ok_or_else(|| {
                    Error::InvalidPixelData(format!(
                        "The fragment at offset {position} declares {length} bytes but the data \
                         holds {}",
                        value.len().saturating_sub(start)
                    ))
                })?;
                fragments.push(fragment);
                position = end;
            }
            SEQUENCE_DELIMITER => break,
            other => {
                return Err(Error::InvalidPixelData(format!(
                    "Unexpected tag '{other}' at offset {position} when parsing the encapsulated \
                     pixel data fragment items"
                )));
            }
        }
    }
    Ok(fragments)
}

/// Concatenate every fragment, skipping the Basic Offset Table.
///
/// This is the undone encapsulation: what a single-fragment-per-frame frame's codestream looks
/// like before the items were wrapped around it, and what pydicom's `defragment_data` returned.
pub fn defragment(value: &[u8]) -> Result<Vec<u8>> {
    let (_, start) = parse_basic_offsets(value, Endian::Little)?;
    let fragments = generate_fragments(value.get(start..).unwrap_or_default(), Endian::Little)?;
    let total: usize = fragments.iter().map(|fragment| fragment.len()).sum();
    let mut stream = Vec::with_capacity(total);
    for fragment in fragments {
        stream.extend_from_slice(fragment);
    }
    Ok(stream)
}

/// Read the item length at `position + 4`, rejecting undefined lengths.
fn item_length(value: &[u8], position: usize, endian: Endian) -> Result<usize> {
    let length = read_u32_at(value, position + 4, endian).ok_or_else(|| {
        Error::InvalidPixelData(format!(
            "Unable to determine the length of the item at offset {} as the end of the data has \
             been reached - the encapsulated pixel data may be invalid",
            position + 4
        ))
    })?;
    if length == u32::MAX {
        return Err(Error::InvalidPixelData(format!(
            "Undefined item length at offset {} when parsing the encapsulated pixel data \
             fragments",
            position + 4
        )));
    }
    Ok(length as usize)
}

/// Read a tag at `offset`, `None` when the data ends first.
fn read_tag_at(value: &[u8], offset: usize, endian: Endian) -> Option<Tag> {
    let chunk = value.get(offset..offset.checked_add(4)?)?;
    Some(Tag::new(
        endian.u16([chunk[0], chunk[1]]),
        endian.u16([chunk[2], chunk[3]]),
    ))
}

/// Read a 32-bit length at `offset`, `None` when the data ends first.
fn read_u32_at(value: &[u8], offset: usize, endian: Endian) -> Option<u32> {
    let chunk = value.get(offset..offset.checked_add(4)?)?;
    Some(endian.u32([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn itemize_wraps_the_fragment() {
        let item = itemize_fragment(b"\xfe\xff\x00\xe1");
        assert_eq!(item, b"\xfe\xff\x00\xe0\x04\x00\x00\x00\xfe\xff\x00\xe1");
    }

    #[test]
    fn round_trip_encapsulate_then_parse() {
        let frames = vec![b"\xfe\xff\x00\xe1".to_vec(), b"\xfe\xff\x00\xe2".to_vec()];
        let value = encapsulate(&frames, 1, true).expect("encapsulate");
        let (offsets, start) = parse_basic_offsets(&value, Endian::Little).expect("offsets");
        assert_eq!(offsets, vec![0, 12]);
        let fragments = generate_fragments(&value[start..], Endian::Little).expect("fragments");
        assert_eq!(fragments.len(), 2);
        assert_eq!(fragments[0], b"\xfe\xff\x00\xe1");
        assert_eq!(fragments[1], b"\xfe\xff\x00\xe2");
    }
}
