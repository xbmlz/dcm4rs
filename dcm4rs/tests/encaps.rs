//! Ports of `pydicom/tests/test_encaps.py`.
//!
//! Each case names the pydicom test it comes from. The streaming and buffer-object tests are
//! absent because they exercise Python's `BufferedIOBase` plumbing rather than DICOM semantics.

use dcm4rs::encaps::{
    defragment, encapsulate, encapsulate_extended, fragment_frame, generate_fragments,
    itemize_fragment, itemize_frame, parse_basic_offsets, parse_fragments,
};
use dcm4rs::Endian;

#[test]
fn fragment_frame_single_fragment_even_data() {
    let bytestream = b"\xfe\xff\x00\xe1";
    let fragments = fragment_frame(bytestream, 1).expect("fragment");
    assert_eq!(fragments.len(), 1);
    assert_eq!(fragments[0], bytestream);
}

#[test]
fn fragment_frame_single_fragment_odd_data() {
    let bytestream = b"\xfe\xff\x00";
    let fragments = fragment_frame(bytestream, 1).expect("fragment");
    assert_eq!(fragments.len(), 1);
    assert_eq!(fragments[0], b"\xfe\xff\x00\x00");
}

#[test]
fn fragment_frame_even_fragments_even_data() {
    let bytestream = b"\xfe\xff\x00\xe1";
    let fragments = fragment_frame(bytestream, 2).expect("fragment");
    assert_eq!(fragments, vec![b"\xfe\xff".to_vec(), b"\x00\xe1".to_vec()]);
}

#[test]
fn fragment_frame_even_fragments_odd_data() {
    let bytestream = b"\xfe\xff\x00";
    let fragments = fragment_frame(bytestream, 2).expect("fragment");
    assert_eq!(fragments, vec![b"\xfe\xff".to_vec(), b"\x00\x00".to_vec()]);
}

#[test]
fn fragment_frame_odd_fragments_even_data() {
    // 124 bytes into seven fragments: six of 18 bytes, the last of 16.
    let bytestream = b"\xfe\xff\x00\xe1".repeat(31);
    let fragments = fragment_frame(&bytestream, 7).expect("fragment");
    assert_eq!(fragments.len(), 7);
    for fragment in &fragments[..6] {
        assert_eq!(fragment.len(), 18);
    }
    assert_eq!(fragments[6].len(), 16);
}

#[test]
fn fragment_frame_odd_fragments_odd_data() {
    // 93 bytes into seven fragments: six of 14 bytes, the last of 9 plus padding.
    let bytestream = b"\xfe\xff\x00".repeat(31);
    let fragments = fragment_frame(&bytestream, 7).expect("fragment");
    assert_eq!(fragments.len(), 7);
    for fragment in &fragments[..6] {
        assert_eq!(fragment.len(), 14);
    }
    assert_eq!(fragments[6].len(), 10);
}

#[test]
fn fragment_frame_too_many_fragments_raises() {
    let bytestream = b"\xfe\xff\x00".repeat(31);
    // 93 bytes can hold at most 47 fragments of two bytes.
    assert!(fragment_frame(&bytestream, 47).is_ok());
    assert!(fragment_frame(&bytestream, 48).is_err());
}

#[test]
fn itemize_frame_single_item() {
    let items = itemize_frame(b"\xfe\xff\x00\xe1", 1).expect("itemize");
    assert_eq!(items, b"\xfe\xff\x00\xe0\x04\x00\x00\x00\xfe\xff\x00\xe1");
}

#[test]
fn itemize_frame_two_items() {
    let items = itemize_frame(b"\xfe\xff\x00\xe1", 2).expect("itemize");
    assert_eq!(
        items,
        b"\xfe\xff\x00\xe0\x02\x00\x00\x00\xfe\xff\xfe\xff\x00\xe0\x02\x00\x00\x00\x00\xe1"
    );
}

#[test]
fn encapsulate_single_frame_with_bot() {
    let frames = vec![b"\xfe\xff\x00\xe1".to_vec()];
    let value = encapsulate(&frames, 1, true).expect("encapsulate");
    assert_eq!(
        value,
        b"\xfe\xff\x00\xe0\x04\x00\x00\x00\x00\x00\x00\x00\
          \xfe\xff\x00\xe0\x04\x00\x00\x00\xfe\xff\x00\xe1"
    );
    // The one offset points at the first fragment's item tag, directly after the table.
    let (offsets, start) = parse_basic_offsets(&value, Endian::Little).expect("offsets");
    assert_eq!(offsets, vec![0]);
    assert_eq!(start, 12);
}

#[test]
fn encapsulate_single_frame_without_bot() {
    let frames = vec![b"\xfe\xff\x00\xe1".to_vec()];
    let value = encapsulate(&frames, 1, false).expect("encapsulate");
    assert_eq!(
        value,
        b"\xfe\xff\x00\xe0\x00\x00\x00\x00\xfe\xff\x00\xe0\x04\x00\x00\x00\xfe\xff\x00\xe1"
    );
}

#[test]
fn encapsulate_offsets_follow_itemised_lengths() {
    let frames = vec![b"\xfe\xff\x00\xe1".to_vec(), b"\x00\x01\x02".to_vec()];
    let value = encapsulate(&frames, 1, true).expect("encapsulate");
    // Each frame occupies one item: eight bytes of item plus the padded payload.
    let (offsets, _) = parse_basic_offsets(&value, Endian::Little).expect("offsets");
    assert_eq!(offsets, vec![0, 12]);

    // The second frame was odd length, so its payload gained a zero byte.
    let start = 8 + 4 * frames.len();
    let fragments = generate_fragments(&value[start..], Endian::Little).expect("fragments");
    assert_eq!(fragments[0], b"\xfe\xff\x00\xe1");
    assert_eq!(fragments[1], b"\x00\x01\x02\x00");
}

#[test]
fn encapsulate_empty_frame_list_writes_an_empty_table() {
    let value = encapsulate(&[], 1, true).expect("encapsulate");
    assert_eq!(value, b"\xfe\xff\x00\xe0\x00\x00\x00\x00");
}

#[test]
fn encapsulate_extended_odd_length_frames() {
    let frames = vec![b"\x00".to_vec(), b"\x01".to_vec(), b"\x02".to_vec()];
    let (value, offsets, lengths) = encapsulate_extended(&frames).expect("encapsulate");
    assert_eq!(value, encapsulate(&frames, 1, false).expect("encapsulate"));
    assert_eq!(
        offsets,
        [0u64, 10, 20]
            .iter()
            .flat_map(|offset| offset.to_le_bytes())
            .collect::<Vec<u8>>()
    );
    assert_eq!(
        lengths,
        [2u64, 2, 2]
            .iter()
            .flat_map(|length| length.to_le_bytes())
            .collect::<Vec<u8>>()
    );
}

#[test]
fn parse_basic_offsets_bad_tag() {
    let value = b"\xfe\xff\x00\xe1\x08\x00\x00\x00\x01\x02\x03\x04\x05\x06\x07\x08";
    let error = parse_basic_offsets(value, Endian::Little).unwrap_err();
    assert!(
        error.to_string().contains(
            "Found unexpected tag (FFFE,E100) instead of (FFFE,E000) when parsing the Basic \
             Offset Table item"
        ),
        "{error}"
    );

    // Read as big endian the same bytes spell (FEFF,00E1).
    let error = parse_basic_offsets(value, Endian::Big).unwrap_err();
    assert!(
        error.to_string().contains(
            "Found unexpected tag (FEFF,00E1) instead of (FFFE,E000) when parsing the Basic \
             Offset Table item"
        ),
        "{error}"
    );
}

#[test]
fn parse_basic_offsets_bad_length_multiple() {
    let value = b"\xfe\xff\x00\xe0\x0a\x00\x00\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a";
    let error = parse_basic_offsets(value, Endian::Little).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("The length of the Basic Offset Table item is not a multiple of 4"),
        "{error}"
    );
}

#[test]
fn parse_basic_offsets_zero_length() {
    let value = b"\xfe\xff\x00\xe0\x00\x00\x00\x00";
    let (offsets, start) = parse_basic_offsets(value, Endian::Little).expect("offsets");
    assert!(offsets.is_empty());
    assert_eq!(start, 8);

    let value = b"\xff\xfe\xe0\x00\x00\x00\x00\x00";
    let (offsets, start) = parse_basic_offsets(value, Endian::Big).expect("offsets");
    assert!(offsets.is_empty());
    assert_eq!(start, 8);
}

#[test]
fn parse_basic_offsets_multi_frame() {
    let value = b"\xfe\xff\x00\xe0\x10\x00\x00\x00\
                  \x00\x00\x00\x00\x66\x13\x00\x00\xf4\x25\x00\x00\xfe\x37\x00\x00\
                  \xfe\xff\x00\xe0";
    let (offsets, start) = parse_basic_offsets(value, Endian::Little).expect("offsets");
    assert_eq!(offsets, vec![0, 4966, 9716, 14334]);
    assert_eq!(start, 24);
}

#[test]
fn parse_basic_offsets_single_frame() {
    let value = b"\xfe\xff\x00\xe0\x04\x00\x00\x00\x00\x00\x00\x00\xfe\xff\x00\xe0";
    let (offsets, start) = parse_basic_offsets(value, Endian::Little).expect("offsets");
    assert_eq!(offsets, vec![0]);
    assert_eq!(start, 12);

    let value = b"\xff\xfe\xe0\x00\x00\x00\x00\x04\x00\x00\x00\x00\xff\xfe\xe0\x00";
    let (offsets, start) = parse_basic_offsets(value, Endian::Big).expect("offsets");
    assert_eq!(offsets, vec![0]);
    assert_eq!(start, 12);
}

#[test]
fn parse_fragments_item_undefined_length() {
    let value = b"\xfe\xff\x00\xe0\xff\xff\xff\xff\x00\x00\x00\x01";
    let error = parse_fragments(value, Endian::Little).unwrap_err();
    assert!(
        error.to_string().contains(
            "Undefined item length at offset 4 when parsing the encapsulated pixel data fragments"
        ),
        "{error}"
    );
}

#[test]
fn parse_fragments_stops_at_sequence_delimiter() {
    let value = b"\xfe\xff\x00\xe0\x04\x00\x00\x00\x01\x00\x00\x00\
                  \xfe\xff\xdd\xe0\x00\x00\x00\x00\
                  \xfe\xff\x00\xe0\x04\x00\x00\x00\x02\x00\x00\x00";
    let (count, offsets) = parse_fragments(value, Endian::Little).expect("fragments");
    assert_eq!(count, 1);
    assert_eq!(offsets, vec![0]);
}

#[test]
fn parse_fragments_item_bad_tag() {
    let value = b"\xfe\xff\x00\xe0\x04\x00\x00\x00\x01\x00\x00\x00\
                  \x10\x00\x10\x00\x00\x00\x00\x00\
                  \xfe\xff\x00\xe0\x04\x00\x00\x00\x02\x00\x00\x00";
    let error = parse_fragments(value, Endian::Little).unwrap_err();
    assert!(
        error.to_string().contains(
            "Unexpected tag '(0010,0010)' at offset 12 when parsing the encapsulated pixel data \
             fragment items"
        ),
        "{error}"
    );
}

#[test]
fn parse_fragments_counts_and_offsets() {
    let value = b"\xfe\xff\x00\xe0\x04\x00\x00\x00\x01\x00\x00\x00\
                  \xfe\xff\x00\xe0\x02\x00\x00\x00\x02\x00\
                  \xfe\xff\xdd\xe0\x00\x00\x00\x00";
    let (count, offsets) = parse_fragments(value, Endian::Little).expect("fragments");
    assert_eq!(count, 2);
    assert_eq!(offsets, vec![0, 12]);
}

#[test]
fn generate_fragments_returns_payloads() {
    let value = b"\xfe\xff\x00\xe0\x04\x00\x00\x00\x01\x00\x00\x00\
                  \xfe\xff\x00\xe0\x02\x00\x00\x00\x02\x00\
                  \xfe\xff\xdd\xe0\x00\x00\x00\x00";
    let fragments = generate_fragments(value, Endian::Little).expect("fragments");
    assert_eq!(fragments.len(), 2);
    assert_eq!(fragments[0], b"\x01\x00\x00\x00");
    assert_eq!(fragments[1], b"\x02\x00");
}

#[test]
fn generate_fragments_rejects_a_fragment_past_the_end() {
    let value = b"\xfe\xff\x00\xe0\x08\x00\x00\x00\x01\x02";
    let error = generate_fragments(value, Endian::Little).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("The fragment at offset 0 declares 8 bytes"),
        "{error}"
    );
}

#[test]
fn defragment_joins_fragments_after_the_table() {
    let frames = vec![b"\x01".to_vec(), b"\x02\x03".to_vec()];
    let value = encapsulate(&frames, 1, true).expect("encapsulate");
    let stream = defragment(&value).expect("defragment");
    // The odd-length first frame carries its padding byte into the stream.
    assert_eq!(stream, b"\x01\x00\x02\x03");
}

#[test]
fn defragment_skips_an_empty_table() {
    let value = encapsulate(&[b"\xfe\xff\x00\xe1".to_vec()], 1, false).expect("encapsulate");
    assert_eq!(defragment(&value).expect("defragment"), b"\xfe\xff\x00\xe1");
}

#[test]
fn itemize_fragment_is_always_little_endian() {
    // The item tag and length are little endian even inside a big-endian data set.
    let item = itemize_fragment(b"\x00\x01");
    assert_eq!(item[..4], [0xFE, 0xFF, 0x00, 0xE0]);
    assert_eq!(item[4..8], [2, 0, 0, 0]);
}
