# dcm4rs

[![CI](https://github.com/xbmlz/dcm4rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/xbmlz/dcm4rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/xbmlz/dcm4rs/branch/main/graph/badge.svg)](https://codecov.io/gh/xbmlz/dcm4rs)
[![pylibjpeg-data](https://github.com/xbmlz/dcm4rs/actions/workflows/pylibjpeg-data.yml/badge.svg?branch=main)](https://github.com/xbmlz/dcm4rs/actions/workflows/pylibjpeg-data.yml)
[![crates.io](https://img.shields.io/crates/v/dcm4rs.svg)](https://crates.io/crates/dcm4rs)
[![docs.rs](https://docs.rs/dcm4rs/badge.svg)](https://docs.rs/dcm4rs)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.75-blue.svg)](#installation)

> **Work in progress.** dcm4rs is under active development and is **not ready for production
> use**. APIs may change without notice, and several codecs and the network layer are still
> missing (see [Status](#status)). Use it for experiments, prototypes and evaluation only.

A pure-Rust DICOM library: data model, PS3.6 data dictionary, Part 10 file I/O and feature-gated
pixel decoding. The core forbids unsafe code, stores values as the bytes defined by the standard,
and keeps third-party codecs behind explicit features.

## Installation

```toml
[dependencies]
dcm4rs = "0.2"
```

Rust 1.75 or newer is required. The default `pixel-jpeg` feature decodes eight-bit and 12-bit
JPEG; the remaining codecs are opt-in:

| Feature | Default | Decoder |
|---|---|---|
| `pixel-jpeg` | yes | 8-bit and 12-bit JPEG baseline, extended and progressive via `zune-jpeg` and `oxiarc-jpeg` |
| `pixel-jpegls` | no | JPEG-LS lossless and near-lossless via `charls` (needs CMake and a C++ toolchain) |
| `pixel-j2k` | no | JPEG 2000 / HTJ2K via `jpeg2k` with the pure-Rust `openjp2` port |
| `pixel-jxl` | no | JPEG XL via `jxl-oxide` |

JPEG Lossless (Process 14 and Selection Value 1) needs no feature: it is pure Rust in the core,
next to RLE Lossless.

## Quick start

```rust
use dcm4rs::dictionary::tags;
use dcm4rs::{DataSet, Tag, Value, Vr};

let mut ds = DataSet::new();
ds.set_string(tags::PATIENT_NAME, Vr::Pn, "Doe^John");
ds.set_u16(tags::COLUMNS, 512);
ds.set(tags::PIXEL_DATA, Vr::Ow, Value::from_u16s(&[0, 1]));

assert_eq!(ds.string(tags::PATIENT_NAME).unwrap(), "Doe^John");
assert_eq!(ds.u16(tags::COLUMNS).unwrap(), 512);
assert_eq!(ds.get(tags::PATIENT_NAME).unwrap().tag, Tag::new(0x0010, 0x0010));
```

## Typed values

Values are stored as the bytes the standard defines, so a file round-trips exactly, but the VRs
with internal structure are tedious to use as bytes. The `valuerep` module parses them on demand
and keeps the text each value was written as, so a value read and written back unmodified loses
nothing:

```rust
use dcm4rs::dictionary::tags;

let dataset = DicomFile::read_file("image.dcm")?.dataset;
let name = dataset.person_name(tags::PATIENT_NAME)?;      // PN: three groups of five components
let thickness = dataset.decimal_strings(tags::SLICE_THICKNESS)?; // DS: text + f64
let echo = dataset.integer_strings(tags::ECHO_NUMBERS)?;  // IS: text + i32
let date = dataset.date(tags::STUDY_DATE)?;               // DA: year / month / day
let time = dataset.time(tags::STUDY_TIME)?;               // TM: hour / minute / second / fraction
let stamp = dataset.date_time(tags::ACQUISITION_DATE_TIME)?; // DT: partial values are allowed
```

Each accessor checks the VR and reports `Error::WrongVr` on a mismatch. `DateTime` keeps the UTC
offset (`offset_minutes`) out of `Ord` on purpose: a value with an offset names a different
instant from the same fields without one, so comparing them silently would hide that.

## Reading and writing files

```rust
use dcm4rs::dictionary::tags;
use dcm4rs::{DicomFile, TransferSyntax};

let file = DicomFile::read_file("image.dcm").unwrap();
println!("{}", file.dataset.string(tags::PATIENT_NAME).unwrap());

assert_eq!(file.transfer_syntax().unwrap(), TransferSyntax::ExplicitVrLittleEndian);

// Stored bytes are written back as they are, so the data set must match the syntax.
let bytes = file.write().unwrap();
```

Supported input: explicit VR little endian, explicit VR big endian (retired), implicit VR,
deflated, sequences with defined or undefined lengths and encapsulated pixel data. A malformed
encoding is an error by default; `ReadOptions::lenient_undefined_length` relaxes the one common
case, an undefined length on a VR that does not allow it, by scanning to the item delimiter the
way pydicom does. Repeated elements keep the first occurrence, where pydicom warns; the tags that
appeared more than once are listed by `DataSet::duplicates`. Writing is canonical: sequences get
defined lengths, group length elements are dropped and values are padded to even length, so most
sources round-trip semantically rather than
byte for byte; the 128-byte preamble is kept as read, and a file built with `DicomFile::new`
starts with zeros. `DataSet::read` decodes a bare data set without a file meta;
`DataSet::to_endian` converts numeric values, including nested sequence items, to another byte
order.

## Pixel data

`DicomFile::pixel_frames` returns the encoded frames and `PixelDescriptor` describes them; use
`decode_frame` or `decode_pixel_data` to decode:

```rust
use dcm4rs::DicomFile;

let file = DicomFile::read_file("image.dcm").unwrap();
let descriptor = file.pixel_descriptor().unwrap();
let pixels = file.decode_frame(0).unwrap();
println!("{}x{}, {} bytes", descriptor.columns, descriptor.rows, pixels.len());
```

RLE Lossless, JPEG Lossless, 8-bit and 12-bit JPEG, JPEG-LS, JPEG 2000 / HTJ2K and JPEG XL are
decoded with
8, 12, 14, 15 or 16-bit samples. Frames follow the
Basic Offset Table and may span several fragments; an empty table falls back to
`Number of Frames` and JPEG end-of-image markers, as pydicom does. Codestream precision that
differs from `Bits Stored` is accepted, again matching pydicom.

## Data dictionary

The PS3.6 dictionary is generated and committed, so building needs no network access: 5212
standard elements and 88 repeating groups. Regenerate it with the generator in `cli`:

```bash
cargo run -p cli --bin dictgen -- data/part06.xml dcm4rs/src/dictionary/generated.rs
```

See [data/README.md](data/README.md) for the source files.

## Workspace

```
Cargo.toml  virtual workspace
dcm4rs/     library: Tag / Vr / Value / Element / DataSet, dictionary, file and pixel I/O
cli/        command-line tools; depends on dcm4rs, is not part of the library API
data/       PS3.6 DocBook source (not committed)
```

## Testing

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

CI runs formatting and lint checks, runs the suites under `cargo-llvm-cov` and uploads the LCOV
report to Codecov (add a `CODECOV_TOKEN` repository secret; upload failures do not block CI).
The test modules mirror pydicom's, one file per module, so behaviour can be compared module by
module.

There is an optional conformance run over the [pylibjpeg-data](https://github.com/pydicom/pylibjpeg-data)
corpus (about 70 MB, downloaded on demand, never committed); lossless categories are compared
byte-for-byte with the uncompressed `_matching_dataset`. It skips when the corpus is absent:

```bash
sh scripts/fetch-pylibjpeg-data.sh
cargo test --all-features --test pylibjpeg_data -- --nocapture
```

## Status

Implemented: data model and dictionary, Part 10 read/write, character set conversion, DEFLATE,
private dictionary registration, typed views for the structured VRs (`PN`, `DS`, `IS`, `DA`, `TM`
and `DT`), RLE / JPEG Lossless / 8-bit and 12-bit JPEG / JPEG-LS / JPEG 2000 / JPEG XL decoding.

Not implemented yet:

- MPEG pixel data; those frames are carried through compressed and report
  `Error::UnsupportedPixelData` on decode.
- Network services (association, C-FIND / C-MOVE / C-GET) as a separate `dcm4rs-net` package.
- Python-side conveniences such as JSON, deferred reads and `private_block`.

Where dcm4rs deliberately differs from pydicom, the difference is pinned by a test:

| Area | dcm4rs |
|---|---|
| Part 10 preamble | kept as read and written back; a file built with `DicomFile::new` starts with 128 zero bytes |
| Group length elements | never written; `(0002,0000)` and `(xxxx,0000)` are dropped |
| Ambiguous VR | corrected on write from Pixel Representation, Bits Allocated and LUT context |
| Unknown escape sequence | `Error::InvalidText` instead of pydicom's warning and default encoding |
| `DataSet` equality | compares elements; byte order and inherited character set do not take part |
| Unknown/undefined-length non-SQ element | `Error::InvalidData`, except `UN` (read as `SQ`) and `OB`/`OW` (read as fragments); `ReadOptions::lenient_undefined_length` scans to the item delimiter instead |

## License

MIT. See [LICENSE](LICENSE).
