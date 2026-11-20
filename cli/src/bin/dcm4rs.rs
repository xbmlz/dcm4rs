//! The `dcm4rs` command: look at a DICOM file, print its data set, decompress it.
//!
//! Usage:
//!
//! ```text
//! dcm4rs info <file>
//! dcm4rs dump <file> [--limit <lines>]
//! dcm4rs decompress <input> <output>
//! ```

use std::process::ExitCode;

use dcm4rs::dictionary::tags;
use dcm4rs::{DataSet, DicomFile, Tag};

const USAGE: &str = "\
usage: dcm4rs <command> ...

commands:
  info <file>                       print the transfer syntax and the pixel summary
  dump <file> [--limit <lines>]     print one line per element
  decompress <input> <output>       decode the frames and write native Pixel Data";

/// What the arguments asked for.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Info { path: String },
    Dump { path: String, limit: Option<usize> },
    Decompress { input: String, output: String },
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("dcm4rs: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    for line in execute(parse(args)?)? {
        println!("{line}");
    }
    Ok(())
}

/// The lines one command prints.
fn execute(command: Command) -> Result<Vec<String>, String> {
    match command {
        Command::Info { path } => {
            let file = read(&path)?;
            let mut lines = vec![format!("File: {path}")];
            lines.extend(info_lines(&file));
            Ok(lines)
        }
        Command::Dump { path, limit } => {
            let file = read(&path)?;
            let lines: Vec<String> = file.dataset.formatted_lines().collect();
            let shown = limit.map_or(lines.len(), |limit| limit.min(lines.len()));
            let mut out: Vec<String> = lines[..shown].to_vec();
            if shown < lines.len() {
                out.push(format!("... {} more element(s)", lines.len() - shown));
            }
            Ok(out)
        }
        Command::Decompress { input, output } => {
            let mut file = read(&input)?;
            let syntax = file.transfer_syntax().map_err(|error| error.to_string())?;
            if !syntax.is_encapsulated() {
                return Err(format!(
                    "{input} holds native Pixel Data, there is nothing to decompress"
                ));
            }
            file.decompress().map_err(|error| error.to_string())?;
            file.write_file(&output)
                .map_err(|error| format!("{output}: {error}"))?;
            Ok(vec![format!("Wrote {output}")])
        }
    }
}

/// Read one command and its operands.
fn parse(args: &[String]) -> Result<Command, String> {
    let mut args = args.iter();
    let command = args.next().ok_or_else(usage)?;
    match command.as_str() {
        "info" => Ok(Command::Info {
            path: args.next().ok_or_else(usage)?.clone(),
        }),
        "dump" => {
            let path = args.next().ok_or_else(usage)?.clone();
            let mut limit = None;
            while let Some(option) = args.next() {
                match option.as_str() {
                    "--limit" => {
                        let value = args.next().ok_or("--limit needs a line count")?;
                        limit = Some(
                            value
                                .parse::<usize>()
                                .map_err(|_| format!("--limit {value} is not a line count"))?,
                        );
                    }
                    other => return Err(format!("unknown option {other}\n\n{USAGE}")),
                }
            }
            Ok(Command::Dump { path, limit })
        }
        "decompress" => Ok(Command::Decompress {
            input: args.next().ok_or_else(usage)?.clone(),
            output: args.next().ok_or_else(usage)?.clone(),
        }),
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}

fn usage() -> String {
    USAGE.to_owned()
}

fn read(path: &str) -> Result<DicomFile, String> {
    DicomFile::read_file(path).map_err(|error| format!("{path}: {error}"))
}

/// The lines `info` prints for a file.
fn info_lines(file: &DicomFile) -> Vec<String> {
    let mut lines = Vec::new();
    if let Ok(syntax) = file.transfer_syntax() {
        lines.push(format!("Transfer syntax: {syntax} [{}]", syntax.uid()));
    }
    for (label, tag) in [
        ("SOP class", tags::SOP_CLASS_UID),
        ("SOP instance", tags::SOP_INSTANCE_UID),
        ("Patient", tags::PATIENT_NAME),
        ("Patient ID", tags::PATIENT_ID),
        ("Study date", tags::STUDY_DATE),
        ("Modality", tags::MODALITY),
    ] {
        if let Some(text) = text(&file.dataset, tag) {
            lines.push(format!("{label}: {text}"));
        }
    }
    if let Ok(descriptor) = file.pixel_descriptor() {
        lines.push(format!(
            "Pixels: {} x {}, {} sample(s) per pixel, {} frame(s), {}",
            descriptor.columns,
            descriptor.rows,
            descriptor.samples_per_pixel,
            descriptor.number_of_frames,
            descriptor.photometric
        ));
        lines.push(format!(
            "Bits: {} stored of {} allocated",
            descriptor.bits_stored, descriptor.bits_allocated
        ));
        if let Some(length) = pixel_data_length(file) {
            lines.push(format!(
                "Pixel data: {length} bytes, {} per frame",
                descriptor.stored_frame_size()
            ));
        }
    }
    lines
}

/// The text of an element, trimmed, when it has one.
fn text(set: &DataSet, tag: Tag) -> Option<String> {
    let text = set.get(tag)?.string().ok()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// How many bytes the Pixel Data holds, read or left in the file.
fn pixel_data_length(file: &DicomFile) -> Option<usize> {
    if let Some(deferred) = file
        .deferred_elements()
        .iter()
        .find(|element| element.tag == tags::PIXEL_DATA)
    {
        return Some(deferred.length);
    }
    file.dataset
        .get(tags::PIXEL_DATA)?
        .value
        .as_bytes()
        .map(<[u8]>::len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dcm4rs::{TransferSyntax, Value, Vr};

    fn sample_file() -> DicomFile {
        let mut dataset = DataSet::new();
        dataset.set(
            tags::SOP_CLASS_UID,
            Vr::Ui,
            Value::from_str("1.2.840.10008.5.1.4.1.1.7"),
        );
        dataset.set(tags::SOP_INSTANCE_UID, Vr::Ui, Value::from_str("1.2.3.4"));
        dataset.set(tags::PATIENT_NAME, Vr::Pn, Value::from_str("Doe^Jane"));
        dataset.set(tags::PATIENT_ID, Vr::Lo, Value::from_str("P-01"));
        dataset.set(tags::MODALITY, Vr::Cs, Value::from_str("OT"));
        dataset.set(tags::ROWS, Vr::Us, Value::from_u16s(&[2]));
        dataset.set(tags::COLUMNS, Vr::Us, Value::from_u16s(&[2]));
        dataset.set(tags::SAMPLES_PER_PIXEL, Vr::Us, Value::from_u16s(&[1]));
        dataset.set(
            tags::PHOTOMETRIC_INTERPRETATION,
            Vr::Cs,
            Value::from_str("MONOCHROME2"),
        );
        dataset.set(tags::BITS_ALLOCATED, Vr::Us, Value::from_u16s(&[8]));
        dataset.set(tags::BITS_STORED, Vr::Us, Value::from_u16s(&[8]));
        dataset.set(tags::HIGH_BIT, Vr::Us, Value::from_u16s(&[7]));
        dataset.set(tags::PIXEL_REPRESENTATION, Vr::Us, Value::from_u16s(&[0]));
        let mut file =
            DicomFile::new(dataset, TransferSyntax::ExplicitVrLittleEndian).expect("file");
        file.set_pixel_data(vec![0u8; 4]).expect("pixels");
        file
    }

    #[test]
    fn info_reports_the_header_and_the_pixels() {
        let lines = info_lines(&sample_file());
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Transfer syntax:")),
            "{lines:?}"
        );
        assert!(lines
            .iter()
            .any(|line| line == "SOP class: 1.2.840.10008.5.1.4.1.1.7"));
        assert!(lines.iter().any(|line| line == "Patient: Doe^Jane"));
        assert!(lines
            .iter()
            .any(|line| line == "Pixels: 2 x 2, 1 sample(s) per pixel, 1 frame(s), MONOCHROME2"));
        assert!(lines
            .iter()
            .any(|line| line == "Pixel data: 4 bytes, 4 per frame"));
    }

    /// A file on disk the commands can read, removed by the caller.
    fn written_file(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        sample_file().write_file(&path).expect("write");
        path
    }

    #[test]
    fn info_lists_the_path_first_and_the_pixels_within() {
        let path = written_file("dcm4rs-info.dcm");
        let lines = execute(Command::Info {
            path: path.display().to_string(),
        })
        .expect("info");
        std::fs::remove_file(&path).expect("remove");
        assert_eq!(lines[0], format!("File: {}", path.display()));
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("Pixel data: 4 bytes")),
            "{lines:?}"
        );
    }

    #[test]
    fn dump_says_how_many_elements_it_left_out() {
        let path = written_file("dcm4rs-dump.dcm");
        let lines = execute(Command::Dump {
            path: path.display().to_string(),
            limit: Some(2),
        })
        .expect("dump");
        std::fs::remove_file(&path).expect("remove");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].chars().take(10).collect::<String>(), "(0008,0016");
        assert!(lines[2].ends_with("more element(s)"), "{:?}", lines[2]);
    }

    #[test]
    fn decompressing_native_pixel_data_is_refused() {
        let path = written_file("dcm4rs-native.dcm");
        let output = path.with_extension("out.dcm");
        let error = execute(Command::Decompress {
            input: path.display().to_string(),
            output: output.display().to_string(),
        })
        .unwrap_err();
        std::fs::remove_file(&path).expect("remove");
        assert!(error.contains("nothing to decompress"), "{error}");
    }

    #[test]
    fn a_missing_file_is_reported_with_its_path() {
        let missing = std::env::temp_dir().join("dcm4rs-missing.dcm");
        let error = execute(Command::Info {
            path: missing.display().to_string(),
        })
        .unwrap_err();
        assert!(error.starts_with(&missing.display().to_string()), "{error}");
    }

    #[test]
    fn dump_lists_the_elements_in_order() {
        let file = sample_file();
        let lines: Vec<String> = file.dataset.formatted_lines().collect();
        assert!(lines[0].starts_with("(0008,0016)"), "{}", lines[0]);
        assert!(lines.iter().any(|line| line.contains("Doe^Jane")));
    }

    #[test]
    fn parse_reads_the_three_commands() {
        assert_eq!(parse(&[]).unwrap_err(), USAGE);
        assert_eq!(parse(&["info".to_owned()]).unwrap_err(), USAGE);
        assert_eq!(
            parse(&["nope".to_owned()]).unwrap_err(),
            format!("unknown command nope\n\n{USAGE}")
        );
        assert_eq!(
            parse(&["info".to_owned(), "a.dcm".to_owned()]).unwrap(),
            Command::Info {
                path: "a.dcm".to_owned()
            }
        );
        assert_eq!(
            parse(&[
                "dump".to_owned(),
                "a.dcm".to_owned(),
                "--limit".to_owned(),
                "5".to_owned()
            ])
            .unwrap(),
            Command::Dump {
                path: "a.dcm".to_owned(),
                limit: Some(5)
            }
        );
        assert!(parse(&[
            "dump".to_owned(),
            "a.dcm".to_owned(),
            "--limit".to_owned(),
            "x".to_owned()
        ])
        .unwrap_err()
        .starts_with("--limit x is not"));
        assert_eq!(
            parse(&[
                "decompress".to_owned(),
                "in.dcm".to_owned(),
                "out.dcm".to_owned()
            ])
            .unwrap(),
            Command::Decompress {
                input: "in.dcm".to_owned(),
                output: "out.dcm".to_owned()
            }
        );
    }
}
