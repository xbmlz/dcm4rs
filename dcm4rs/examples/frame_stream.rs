//! Time reading a multi-frame image whole against streaming its frames.
//!
//! Run with `cargo run --release --example frame_stream`.

use std::time::Instant;

use dcm4rs::dictionary::tags;
use dcm4rs::{DataSet, DicomFile, ReadOptions, Value, Vr};

const ROWS: u16 = 256;
const COLUMNS: u16 = 256;
const FRAMES: u32 = 200;

fn main() -> dcm4rs::Result<()> {
    let frame_size = ROWS as usize * COLUMNS as usize * 2;
    let total = frame_size * FRAMES as usize;
    let mut set = DataSet::new();
    let _ = set.set(tags::ROWS, Vr::Us, Value::from_u16s(&[ROWS]));
    let _ = set.set(tags::COLUMNS, Vr::Us, Value::from_u16s(&[COLUMNS]));
    let _ = set.set(tags::SAMPLES_PER_PIXEL, Vr::Us, Value::from_u16s(&[1]));
    let _ = set.set(tags::BITS_ALLOCATED, Vr::Us, Value::from_u16s(&[16]));
    let _ = set.set(tags::BITS_STORED, Vr::Us, Value::from_u16s(&[16]));
    let _ = set.set(tags::HIGH_BIT, Vr::Us, Value::from_u16s(&[15]));
    let _ = set.set(tags::PIXEL_REPRESENTATION, Vr::Us, Value::from_u16s(&[0]));
    let _ = set.set(
        tags::PHOTOMETRIC_INTERPRETATION,
        Vr::Cs,
        Value::from_str("MONOCHROME2"),
    );
    let _ = set.set(
        tags::NUMBER_OF_FRAMES,
        Vr::Is,
        Value::from_str(&FRAMES.to_string()),
    );
    let samples: Vec<u8> = (0..total).map(|value| (value % 251) as u8).collect();
    let _ = set.set(tags::PIXEL_DATA, Vr::Ob, Value::from_bytes(samples));

    let mut meta = DataSet::new();
    let _ = meta.set(
        tags::TRANSFER_SYNTAX_UID,
        Vr::Ui,
        Value::from_str("1.2.840.10008.1.2.1"),
    );
    let mut path = std::env::temp_dir();
    path.push(format!("dcm4rs-stream-{}.dcm", std::process::id()));
    DicomFile::from_parts(meta, set, [0u8; 128]).write_file(&path)?;

    let size = std::fs::metadata(&path).expect("the written file").len();
    println!(
        "{FRAMES} frames of {ROWS}x{COLUMNS} 16-bit, {} stored bytes, {} bytes on disk",
        total, size
    );

    let start = Instant::now();
    let file = DicomFile::read_file(&path)?;
    let read = start.elapsed();
    let start = Instant::now();
    for index in 0..FRAMES as usize {
        let _ = file.decode_frame(index)?;
    }
    let whole = start.elapsed();

    let start = Instant::now();
    let options = ReadOptions {
        defer_size: 1 << 20,
        ..ReadOptions::default()
    };
    let deferred = DicomFile::read_file_with_options(&path, options)?;
    let deferred_read = start.elapsed();
    let start = Instant::now();
    let mut count = 0;
    for frame in deferred.iter_frames()? {
        let _ = frame?;
        count += 1;
    }
    let streamed = start.elapsed();

    println!("read whole file:        {read:>10?}");
    println!(
        "decode all frames:      {whole:>10?} ({:?} per frame)",
        whole / FRAMES
    );
    println!("read deferred file:     {deferred_read:>10?}");
    println!(
        "stream {count} frames:      {streamed:>10?} ({:?} per frame)",
        streamed / count
    );

    std::fs::remove_file(&path).expect("remove the file");
    Ok(())
}
