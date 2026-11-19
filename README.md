# *dcm4rs*

[![CI](https://github.com/xbmlz/dcm4rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/xbmlz/dcm4rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/xbmlz/dcm4rs/branch/main/graph/badge.svg)](https://codecov.io/gh/xbmlz/dcm4rs)
[![pylibjpeg-data](https://github.com/xbmlz/dcm4rs/actions/workflows/pylibjpeg-data.yml/badge.svg?branch=main)](https://github.com/xbmlz/dcm4rs/actions/workflows/pylibjpeg-data.yml)
[![crates.io](https://img.shields.io/crates/v/dcm4rs.svg)](https://crates.io/crates/dcm4rs)
[![docs.rs](https://docs.rs/dcm4rs/badge.svg)](https://docs.rs/dcm4rs)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.75-blue.svg)](#installation)

*dcm4rs* is a pure-Rust library for working with [DICOM](https://www.dicomstandard.org/) files. It
reads, modifies and writes DICOM data sets, and decodes and encodes *Pixel Data*, without unsafe
code in the core and without a C toolchain unless you ask for one. Values are stored as the bytes
the standard defines, so a file that is read and written back unmodified loses nothing.

Note that *dcm4rs* is a general-purpose framework concerned with reading and writing DICOM data.
It does not handle the specifics of individual SOP classes, and DICOM networking — associations
and the DIMSE services — belongs to a separate `dcm4rs-net` package, the way *pydicom* keeps
networking in [*pynetdicom*](https://github.com/pydicom/pynetdicom). The DICOM JSON model,
media storage directories and waveform decoding are part of the library.

> **Work in progress.** *dcm4rs* is under active development and is **not ready for production
> use**. APIs may change without notice, and several codecs and the network layer are still
> missing (see [Status](#status)). Use it for experiments, prototypes and evaluation only.

## Installation

```toml
[dependencies]
dcm4rs = "0.2"
```

Rust 1.75 or newer is required. The default `pixel-jpeg` feature decodes eight-bit and 12-bit
JPEG; the remaining codecs are opt-in:

| Feature | Default | Codec |
|---|---|---|
| `pixel-jpeg` | yes | 8-bit and 12-bit JPEG baseline, extended and progressive via `zune-jpeg` and `oxiarc-jpeg` |
| `pixel-jpegls` | no | JPEG-LS lossless and near-lossless via `charls` (needs CMake and a C++ toolchain) |
| `pixel-j2k` | no | JPEG 2000 / HTJ2K via `jpeg2k` with the pure-Rust `openjp2` port |
| `pixel-jxl` | no | JPEG XL decoding via `jxl-oxide`, encoding via `zune-jpegxl` (needs Rust 1.80) |

JPEG Lossless (Process 14 and Selection Value 1) needs no feature: it is pure Rust in the core,
next to RLE Lossless.

## Documentation

The [API reference](https://docs.rs/dcm4rs) is on docs.rs; every module documents the standard
clauses it implements.

## Data sets

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

The VRs with internal structure are tedious to use as bytes, so the `valuerep` module parses them
on demand and, like *pydicom*, keeps the text each value was written as:

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
way *pydicom* does. Repeated elements keep the first occurrence, where *pydicom* warns; the tags
that appeared more than once are listed by `DataSet::duplicates`. Writing is canonical: sequences
get defined lengths, group length elements are dropped and values are padded to even length, so
most sources round-trip semantically rather than byte for byte; the 128-byte preamble is kept as
read, and a file built with `DicomFile::new` starts with zeros. `DataSet::read` decodes a bare data
set without a file meta; `DataSet::to_endian` converts numeric values, including nested sequence
items, to another byte order.

## *Pixel Data*

Compressed and uncompressed *Pixel Data* is available as the frames it is:
`DicomFile::pixel_frames` returns the encoded frames, `PixelDescriptor` describes them, and
`decode_frame` or `decode_pixel_data` produce samples.

```rust
use dcm4rs::DicomFile;

let file = DicomFile::read_file("image.dcm").unwrap();
let descriptor = file.pixel_descriptor().unwrap();
let pixels = file.decode_frame(0).unwrap();
println!("{}x{}, {} bytes", descriptor.columns, descriptor.rows, pixels.len());
```

### Colour

`decode_rgb_frame` decodes a frame and returns displayable RGB: a `PALETTE COLOR` image is mapped
through its colour lookup table, `YBR_FULL`, `YBR_FULL_422` and `YBR_PARTIAL_422` are converted,
and a monochrome frame comes back as the samples it stores. The pieces are public on their own
too: `convert_color_space`, `expand_ybr422`, `PaletteColorLut` and, for one-bit values such as
Overlay Data, `unpack_bits` and `pack_bits`.

```rust
let rgb = file.decode_rgb_frame(0).unwrap();
let mapped = file.apply_color_lut(0).unwrap();
```

### Decompressing

RLE Lossless, JPEG Lossless, 8-bit and 12-bit JPEG, JPEG-LS, JPEG 2000 / HTJ2K and JPEG XL are
decoded with 8, 12, 14, 15 or 16-bit samples. Frames follow the Basic Offset Table and may span
several fragments; an empty table falls back to `Number of Frames` and JPEG end-of-image
markers, as *pydicom* does. Codestream precision that differs from `Bits Stored` is accepted,
again matching *pydicom*.

MPEG video stays undecoded on purpose — as far as *dcm4che* goes, and no further. Frames come out
of `encapsulated_frames` as the compressed bitstreams they are, `mpeg2_sequence_header` reads the
MPEG-2 sequence header into Rows, Columns, Cine Rate, Frame Time and Pixel Aspect Ratio, and
asking for a pixel reports `Error::UnsupportedPixelData`.

### Compressing

`compress_rle`, `compress_jpeg(quality)`, `compress_jpegls`, `compress_j2k` and `compress_jxl`
replace the frames with encapsulated data and switch the transfer syntax. Baseline JPEG writes
8-bit samples and turns a colour frame into `YBR_FULL_422`; the others are lossless:

| Method | Transfer syntax |
|---|---|
| `compress_rle` | RLE Lossless |
| `compress_jpeg(q)` | JPEG Baseline (8-bit, `YBR_FULL_422` for colour) |
| `compress_jpegls` | JPEG-LS Lossless |
| `compress_j2k` | JPEG 2000 Lossless |
| `compress_jxl` | JPEG XL Lossless |

### Writing native samples

`decompress` decodes every encapsulated frame and rewrites the file as Explicit VR Little Endian.
The JPEG, JPEG 2000 / HTJ2K and JPEG XL decoders hand back RGB, so a colour image they produced
becomes `RGB` with an interleaved planar configuration; RLE Lossless keeps the coded samples and
so keeps its photometric interpretation. `set_pixel_data` is the way back in: it stores native
samples and keeps `Number of Frames` in step with their length.

## Data dictionary

The PS3.6 dictionary is generated and committed, so building needs no network access: 5212
standard elements and 88 repeating groups. Regenerate it with the generator in `cli`:

```bash
cargo run -p cli --bin dictgen -- data/part06.xml dcm4rs/src/dictionary/generated.rs
```

See [data/README.md](data/README.md) for the source files.

## Contributing

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

CI runs formatting and lint checks, runs the suites under `cargo-llvm-cov` and uploads the LCOV
report to Codecov (add a `CODECOV_TOKEN` repository secret; upload failures do not block CI).
The test modules mirror *pydicom*'s, one file per module, so behaviour can be compared module by
module.

There is an optional conformance run over the
[pylibjpeg-data](https://github.com/pydicom/pylibjpeg-data) corpus (about 70 MB, downloaded on
demand, never committed); lossless categories are compared byte-for-byte with the uncompressed
`_matching_dataset`. It skips when the corpus is absent:

```bash
sh scripts/fetch-pylibjpeg-data.sh
cargo test --all-features --test pylibjpeg_data -- --nocapture
```

The workspace:

```
Cargo.toml  virtual workspace
dcm4rs/     library: Tag / Vr / Value / Element / DataSet, dictionary, file and pixel I/O
cli/        command-line tools; depends on dcm4rs, is not part of the library API
data/       PS3.6 DocBook source (not committed)
```

## Status

Implemented: data model and dictionary, Part 10 read/write, character set conversion, DEFLATE,
private dictionary registration, private creator blocks, deferred reads, the DICOM JSON model
(PS3.18 F.2), modality and VOI LUTs, media storage directories (the DICOMDIR), waveform decoding,
typed views for the structured VRs (`PN`, `DS`, `IS`, `DA`, `TM` and `DT`), UID generation under the
`2.25.` UUID root, colour conversion and palette colour lookup, pixel data write-back and
decompression, RLE Lossless, baseline
JPEG, JPEG-LS, JPEG 2000 and JPEG XL encoding, and RLE / JPEG Lossless / 8-bit and 12-bit JPEG /
JPEG-LS / JPEG 2000 / JPEG XL decoding.

Not implemented yet:

- Network services (association, C-FIND / C-MOVE / C-GET) as a separate `dcm4rs-net` package.

Where *dcm4rs* deliberately differs from *pydicom*, the difference is pinned by a test:

| Area | dcm4rs |
|---|---|
| Part 10 preamble | kept as read and written back; a file built with `DicomFile::new` starts with 128 zero bytes |
| Group length elements | never written; `(0002,0000)` and `(xxxx,0000)` are dropped |
| Ambiguous VR | corrected on write from Pixel Representation, Bits Allocated and LUT context |
| Unknown escape sequence | `Error::InvalidText` instead of *pydicom*'s warning and default encoding |
| `DataSet` equality | compares elements; byte order and inherited character set do not take part |
| Unknown/undefined-length non-SQ element | `Error::InvalidData`, except `UN` (read as `SQ`) and `OB`/`OW` (read as fragments); `ReadOptions::lenient_undefined_length` scans to the item delimiter instead |
| JSON `BulkDataURI` | `Error::InvalidValue` on read, because fetching it needs whatever hosts the payload |
| JSON encapsulated pixel data | `Error::UnsupportedPixelData` on write, because the fragments would come back as one opaque blob |
| Deferred reads | the tag stays out of the data set until `load_element` re-reads it, rather than holding a placeholder whose value fetches on access |
| DICOMDIR | the tree is rebuilt from the record flags, not from the byte offsets; writing recomputes the offsets from the record order |
| `decompress` | the photometric interpretation becomes `RGB` when the codec handed back RGB, because the expanded chroma no longer matches `YBR_FULL_422`; *pydicom* leaves the attribute alone |
| Colour conversion | eight-bit three-sample frames only; `YBR_ICT` and `YBR_RCT` need no conversion because the JPEG 2000 decoder already applies their inverse |
| Generated UIDs | the `2.25.` UUID-derived root, which needs no registration, instead of *pydicom*'s `1.2.826.0.1.3680043.8.498.` prefix |

## License

MIT. See [LICENSE](LICENSE).
