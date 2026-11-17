//! Optional conformance run over the [pylibjpeg-data] corpus.
//!
//! The corpus is not vendored. Fetch it with `scripts/fetch-pylibjpeg-data.sh`; the test skips
//! when no checkout is present. Lossless categories are compared byte-for-byte with the
//! uncompressed dataset named by pylibjpeg-data's `_matching_dataset` index.
//!
//! [pylibjpeg-data]: https://github.com/pydicom/pylibjpeg-data

use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use dcm4rs::dictionary::tags;
use dcm4rs::{DicomFile, PixelRepresentation};

struct Entry {
    rel: PathBuf,
    rows: Option<u16>,
    columns: Option<u16>,
    samples: Option<u16>,
    bits_allocated: Option<u16>,
    bits_stored: Option<u16>,
    pixel_representation: Option<u16>,
    frames: Option<u32>,
    status: u32,
    matching: Option<PathBuf>,
}

#[test]
fn pylibjpeg_data_decodes() {
    let Some((root, ds_dir)) = locate() else {
        eprintln!("skipping pylibjpeg-data; run `sh scripts/fetch-pylibjpeg-data.sh` to enable it");
        return;
    };
    let entries = entries(&root, &ds_dir);
    assert!(
        !entries.is_empty(),
        "no DICOM files under {}",
        ds_dir.display()
    );

    let mut decoded = 0usize;
    let mut skipped = 0usize;
    let mut failures = Vec::new();

    for entry in entries {
        if entry.status != 0 {
            skipped += 1;
            continue;
        }
        if skip_category(&entry.rel) {
            skipped += 1;
            continue;
        }
        let path = ds_dir.join(&entry.rel);
        let file = match DicomFile::read_file(&path) {
            Ok(file) => file,
            Err(error) => {
                failures.push(format!("{}: read: {error}", path.display()));
                continue;
            }
        };
        if !file.dataset.contains(tags::PIXEL_DATA) {
            skipped += 1;
            continue;
        }
        let syntax = match file.transfer_syntax() {
            Ok(syntax) => syntax,
            Err(error) => {
                failures.push(format!("{}: transfer syntax: {error}", path.display()));
                continue;
            }
        };
        if !decoder_available(syntax.uid()) {
            skipped += 1;
            continue;
        }
        let descriptor = match file.pixel_descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => {
                failures.push(format!("{}: descriptor: {error}", path.display()));
                continue;
            }
        };

        // `zune-jpeg` decodes 8-bit samples only; 12-bit JPEG needs a libjpeg-turbo build
        // with 12-bit support, which has no Rust binding here.
        if is_jpeg_uid(syntax.uid()) && descriptor.bits_allocated > 8 {
            skipped += 1;
            continue;
        }

        if let Some(rows) = entry.rows {
            if descriptor.rows != rows {
                failures.push(format!(
                    "{}: Rows {} != index {rows}",
                    path.display(),
                    descriptor.rows
                ));
                continue;
            }
        }
        if let Some(columns) = entry.columns {
            if descriptor.columns != columns {
                failures.push(format!(
                    "{}: Columns {} != index {columns}",
                    path.display(),
                    descriptor.columns
                ));
                continue;
            }
        }
        if let Some(samples) = entry.samples {
            if descriptor.samples_per_pixel != samples {
                failures.push(format!(
                    "{}: SamplesPerPixel {} != index {samples}",
                    path.display(),
                    descriptor.samples_per_pixel
                ));
                continue;
            }
        }
        if let Some(bits) = entry.bits_allocated {
            if descriptor.bits_allocated != bits {
                failures.push(format!(
                    "{}: BitsAllocated {} != index {bits}",
                    path.display(),
                    descriptor.bits_allocated
                ));
                continue;
            }
        }
        if let Some(bits) = entry.bits_stored {
            if descriptor.bits_stored != bits {
                failures.push(format!(
                    "{}: BitsStored {} != index {bits}",
                    path.display(),
                    descriptor.bits_stored
                ));
                continue;
            }
        }
        if let Some(value) = entry.pixel_representation {
            let expected = if value == 0 {
                PixelRepresentation::Unsigned
            } else {
                PixelRepresentation::Signed
            };
            if descriptor.pixel_representation != expected {
                failures.push(format!(
                    "{}: PixelRepresentation {:?} != index {value}",
                    path.display(),
                    descriptor.pixel_representation
                ));
                continue;
            }
        }
        if let Some(frames) = entry.frames {
            if descriptor.number_of_frames != frames {
                failures.push(format!(
                    "{}: NumberOfFrames {} != index {frames}",
                    path.display(),
                    descriptor.number_of_frames
                ));
                continue;
            }
        }

        let frames = match file.decode_pixel_data() {
            Ok(frames) => frames,
            Err(error) => {
                failures.push(format!("{}: decode: {error}", path.display()));
                continue;
            }
        };
        if frames.len() != descriptor.number_of_frames as usize {
            failures.push(format!(
                "{}: decoded {} frames, expected {}",
                path.display(),
                frames.len(),
                descriptor.number_of_frames
            ));
            continue;
        }
        let frame_size = descriptor.frame_size();
        if let Some(frame) = frames.iter().find(|frame| frame.len() != frame_size) {
            failures.push(format!(
                "{}: frame length {} != {frame_size}",
                path.display(),
                frame.len()
            ));
            continue;
        }

        if let Some(reference) = &entry.matching {
            if is_lossless_category(&entry.rel) {
                let reference_path = ds_dir.join(reference);
                match DicomFile::read_file(&reference_path)
                    .and_then(|reference| reference.decode_pixel_data())
                {
                    Ok(reference) if reference == frames => {}
                    Ok(_) => failures.push(format!(
                        "{}: decoded pixels differ from {}",
                        path.display(),
                        reference_path.display()
                    )),
                    Err(error) => failures.push(format!(
                        "{}: reference {}: {error}",
                        path.display(),
                        reference_path.display()
                    )),
                }
            }
        }

        decoded += 1;
    }

    eprintln!(
        "pylibjpeg-data: decoded {decoded}, skipped {skipped}, failed {}",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} pylibjpeg-data failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(decoded > 0, "no pylibjpeg-data files were decoded");
}

fn locate() -> Option<(PathBuf, PathBuf)> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut candidates = Vec::new();
    if let Some(value) = std::env::var_os("PYLIBJPEG_DATA_DIR") {
        candidates.push(PathBuf::from(value));
    }
    candidates.push(manifest.join("target/pylibjpeg-data"));
    // The fetch script writes to the workspace `target` directory by default.
    candidates.push(manifest.join("../target/pylibjpeg-data"));
    candidates.push(manifest.join("../pylibjpeg-data"));
    candidates.push(manifest.join("tests/data/pylibjpeg-data"));

    for candidate in candidates {
        let ds_dir = candidate.join("ljdata/ds");
        if ds_dir.is_dir() {
            return Some((candidate, ds_dir));
        }
        if candidate.is_dir() && contains_dcm(&candidate) {
            return Some((candidate.clone(), candidate));
        }
    }
    None
}

fn contains_dcm(dir: &Path) -> bool {
    let mut files = Vec::new();
    collect_dcm(dir, &mut files);
    !files.is_empty()
}

fn collect_dcm(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_dcm(&path, files);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dcm"))
        {
            files.push(path);
        }
    }
}

fn entries(root: &Path, ds_dir: &Path) -> Vec<Entry> {
    if let Some(entries) = load_manifest(&root.join("manifest.tsv")) {
        return entries;
    }

    let mut files = Vec::new();
    collect_dcm(ds_dir, &mut files);
    files.sort();
    files
        .into_iter()
        .filter_map(|path| {
            path.strip_prefix(ds_dir).ok().map(|rel| Entry {
                rel: rel.to_path_buf(),
                rows: None,
                columns: None,
                samples: None,
                bits_allocated: None,
                bits_stored: None,
                pixel_representation: None,
                frames: None,
                status: 0,
                matching: None,
            })
        })
        .collect()
}

fn load_manifest(path: &Path) -> Option<Vec<Entry>> {
    let text = fs::read_to_string(path).ok()?;
    let mut entries = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if line_number == 0 && line.starts_with("category\t") {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            11,
            "malformed manifest line {}: {line}",
            line_number + 1
        );
        entries.push(Entry {
            rel: PathBuf::from(fields[0]).join(fields[1]),
            rows: number(fields[2]),
            columns: number(fields[3]),
            samples: number(fields[4]),
            bits_allocated: number(fields[5]),
            bits_stored: number(fields[6]),
            pixel_representation: number(fields[7]),
            frames: number(fields[8]),
            status: number(fields[9]).unwrap_or(0),
            matching: if fields[10].is_empty() {
                None
            } else {
                Some(PathBuf::from(fields[10]))
            },
        });
    }
    Some(entries)
}

fn number<T: FromStr>(value: &str) -> Option<T> {
    if value.is_empty() {
        None
    } else {
        value.parse().ok()
    }
}

fn category(rel: &Path) -> Option<&str> {
    rel.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
}

fn skip_category(rel: &Path) -> bool {
    matches!(
        category(rel),
        Some("JPEGLossless") | Some("JPEGLosslessSV1")
    )
}

fn is_lossless_category(rel: &Path) -> bool {
    matches!(
        category(rel),
        Some("RLELossless") | Some("JPEG2000Lossless") | Some("JPEGLSLossless")
    )
}

fn is_jpeg_uid(uid: &str) -> bool {
    matches!(
        uid,
        "1.2.840.10008.1.2.4.50"
            | "1.2.840.10008.1.2.4.51"
            | "1.2.840.10008.1.2.4.52"
            | "1.2.840.10008.1.2.4.53"
            | "1.2.840.10008.1.2.4.54"
    )
}

fn decoder_available(uid: &str) -> bool {
    match uid {
        "1.2.840.10008.1.2.4.50" | "1.2.840.10008.1.2.4.51" => {
            cfg!(feature = "pixel-jpeg")
        }
        "1.2.840.10008.1.2.4.80" | "1.2.840.10008.1.2.4.81" => {
            cfg!(feature = "pixel-jpegls")
        }
        "1.2.840.10008.1.2.4.90" | "1.2.840.10008.1.2.4.91" => {
            cfg!(feature = "pixel-j2k")
        }
        "1.2.840.10008.1.2.4.110" => cfg!(feature = "pixel-jxl"),
        _ => matches!(
            uid,
            "1.2.840.10008.1.2"
                | "1.2.840.10008.1.2.1"
                | "1.2.840.10008.1.2.2"
                | "1.2.840.10008.1.2.1.99"
                | "1.2.840.10008.1.2.5"
        ),
    }
}
