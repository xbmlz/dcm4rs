# dcm4rs

[![CI](https://github.com/xbmlz/dcm4rs/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/xbmlz/dcm4rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/xbmlz/dcm4rs/branch/main/graph/badge.svg)](https://codecov.io/gh/xbmlz/dcm4rs)
[![pylibjpeg-data](https://github.com/xbmlz/dcm4rs/actions/workflows/pylibjpeg-data.yml/badge.svg?branch=main)](https://github.com/xbmlz/dcm4rs/actions/workflows/pylibjpeg-data.yml)
[![crates.io](https://img.shields.io/crates/v/dcm4rs.svg)](https://crates.io/crates/dcm4rs)
[![docs.rs](https://docs.rs/dcm4rs/badge.svg)](https://docs.rs/dcm4rs)
[![License](https://img.shields.io/crates/l/dcm4rs.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.75-blue.svg)](#installation)

A pure-Rust DICOM library: data model, PS3.6 dictionary, Part 10 file I/O, and feature-gated
pixel decoding. The core forbids unsafe code, keeps values as the bytes defined by the standard,
and leaves third-party codecs behind explicit features so it stays easy to embed in long-lived
products such as a PACS.

## Installation

```toml
[dependencies]
dcm4rs = "0.1"
```

Rust 1.75 or newer is required. The default `pixel-jpeg` feature decodes 8-bit JPEG; the
remaining codecs are opt-in:

| Feature | Default | Decoder |
|---|---|---|
| `pixel-jpeg` | yes | 8-bit JPEG baseline, extended and progressive via `zune-jpeg` |
| `pixel-jpegls` | no | JPEG-LS lossless and near-lossless via `charls` (CMake and a C++ toolchain required) |
| `pixel-j2k` | no | JPEG 2000 / HTJ2K via `jpeg2k` with the pure-Rust `openjp2` port |
| `pixel-jxl` | no | JPEG XL via `jxl-oxide` |

## Quick start

```rust
use dcm4rs::dictionary::tags;
use dcm4rs::{DataSet, Endian, Tag, Value, Vr};

let mut ds = DataSet::new();
ds.set_string(tags::PATIENT_NAME, Vr::Pn, "Doe^John");
ds.set_u16(tags::COLUMNS, 512);
ds.set(tags::PIXEL_DATA, Vr::Ow, Value::from_u16s(&[0, 1]));

assert_eq!(ds.string(tags::PATIENT_NAME).unwrap(), "Doe^John");
assert_eq!(ds.u16(tags::COLUMNS).unwrap(), 512);
assert_eq!(ds.get(tags::PATIENT_NAME).unwrap().tag, Tag::new(0x0010, 0x0010));

// Big-endian transfer syntax: bytes are stored big endian and declared when decoding.
let mut big = DataSet::with_endian(Endian::Big);
big.set(tags::ROWS, Vr::Us, Value::from_bytes(vec![0x02, 0x00]));
assert_eq!(big.u16(tags::ROWS).unwrap(), 512);

// Dictionary lookup.
let entry = dcm4rs::dictionary::lookup(tags::PIXEL_DATA).unwrap();
assert!(entry.vr.is_ambiguous()); // OB or OW, decided by context
```

## Workspace layout

```
Cargo.toml  virtual workspace
dcm4rs/     library: data model (Tag / Vr / Value / Element / DataSet) + dictionary + file I/O
cli/        command-line package: depends on dcm4rs only, not part of the library's public API
data/       PS3.6 DocBook source (not committed)
```

With just two packages there is no `crates/` layer: uv, ruff and rust-analyzer use `crates/`
because they have dozens of members; two packages read more directly in the flat layout used by
clap / tokio / serde, and `cargo run -p cli` is just as short.

Dependencies flow one way: `cli ──▶ dcm4rs`; the library never depends on the command-line tool.
Third parties depend only on the `dcm4rs` library, not on `cli`. New commands (validate, dump,
transcode, ...) go into `cli/src/bin/`; the library itself ships no binary.

The naming follows the shape of pydicom / fo-dicom: **one library package, with the dictionary as
a module inside it** (`dcm4rs::dictionary`), rather than splitting it into `core` / `dict` packages
the way dcm4che does — Java can call a package `dcm4che-core`, but in Rust the name `core` clashes
with the sysroot crate, so copying that split would only bring trouble. If the dictionary ever
needs to be compiled separately, `dcm4rs-dictionary` can be split out then.

## Design decisions

1. **Values are stored as raw bytes.** `Value::Bytes` keeps the bytes that appear in the standard;
   interpreting numbers and text happens at read time. Data under any transfer syntax (explicit or
   implicit VR, big endian) can be written back byte for byte, and the parser never has to guess.
2. **Explicit endianness.** Decoders such as `Element::u16s(endian)` require an `Endian`, and
   `DataSet` carries one set from the transfer syntax for convenience accessors such as
   `ds.u16(tag)`. There is no hidden global state.
3. **Strict rather than lenient.** A VR that does not match the decoding, a length that is not a
   multiple of the unit, or text that is not valid for its character set all produce explicit
   errors (`Error::WrongVr` / `WrongLength` / `InvalidUtf8` / `InvalidText` /
   `UnsupportedCharacterSet`) instead of silently yielding garbage.
4. **Tag order is standard order.** `DataSet` stores elements in a `BTreeMap`, so iteration is
   naturally ascending by (group, element), matching the order required for writing.
5. **Lean dependencies.** Character set decoding uses `encoding_rs`, DEFLATE uses `flate2` with
   its pure-Rust backend; JPEG decoding uses `zune-jpeg` behind the default `pixel-jpeg` feature,
   JPEG-LS uses `charls` behind the `pixel-jpegls` feature, JPEG 2000 uses `jpeg2k` (with the
   pure-Rust `openjp2` port) behind `pixel-j2k`, JPEG XL uses `jxl-oxide` behind `pixel-jxl`, and
   further codecs stay feature-gated.

## File I/O

`DicomFile::read` / `DicomFile::read_file` open a Part 10 file; `DataSet::read` decodes a bare
data set; `write` encodes both again.

```rust
use dcm4rs::dictionary::tags;
use dcm4rs::{DicomFile, TransferSyntax};

let file = DicomFile::read_file("image.dcm").unwrap();
println!("{}", file.dataset.string(tags::PATIENT_NAME).unwrap());

let syntax = file.transfer_syntax().unwrap();
assert_eq!(syntax, TransferSyntax::ExplicitVrLittleEndian);

// Encode again: stored bytes are written as-is, so the syntax must match the data set.
let bytes = file.write().unwrap();
```

Supported: explicit VR little endian, explicit VR big endian (retired), implicit VR (the VR of
each element comes from the dictionary, ambiguous entries resolved per PS3.5 6.2.2), sequences
with defined or undefined lengths, encapsulated pixel data (`Value::Fragments`). A file meta
without `(0002,0010)` falls back to the default transfer syntax (Implicit VR Little Endian), as
pydicom does. Writing is canonical: sequences get defined lengths, group length elements are
dropped and text is padded to even length, so an undefined-length source round-trips semantically
rather than byte for byte. Deflated (`1.2.840.10008.1.2.1.99`) is inflated with raw DEFLATE on
read and written back deflated; unknown transfer syntaxes report
`Error::UnsupportedTransferSyntax`.

`DataSet::to_endian` converts numeric values, including nested sequence items, to another byte
order; text, `OB`/`UN` values and encapsulated fragments are copied unchanged. Ambiguous VRs are
corrected on write from context. `DataSet::read_with_private_dictionary` and
`DicomFile::read_with_private_dictionary` resolve private element VRs from a caller-supplied
`PrivateDictionary`; private creators are read from the stream.

## Pixel data

Pixel data frames are available without decoding through `DicomFile::pixel_frames`, and a
validated `PixelDescriptor` describes them. RLE Lossless, 8-bit JPEG (baseline / extended /
progressive), JPEG-LS (lossless / near-lossless), JPEG 2000 / HTJ2K and JPEG XL are decoded by
`DicomFile::decode_frame` and `decode_pixel_data`; JPEG lossless and MPEG report
`Error::UnsupportedPixelData`.

JPEG-LS is behind the `pixel-jpegls` feature and builds CharLS at compile time, so it needs
CMake and a C++ toolchain. JPEG 2000 / HTJ2K is behind `pixel-j2k` and uses the pure-Rust
`openjp2` port; JPEG XL is behind `pixel-jxl` and uses `jxl-oxide`.

## Tests and checks

The suites mirror pydicom's test modules one file per module, so behaviour can be compared
module by module: `tag.rs`, `data_element.rs`, `dataset.rs`, `dictionary.rs`, `file_reader.rs`,
`file_writer.rs`, `transfer_syntax.rs`, `pixels.rs`. `tests/common/mod.rs` builds the byte fixtures;
every helper there emits canonical bytes so tests can assert exact output. Where dcm4rs
intentionally behaves differently from pydicom, the test says so in a comment and the difference
is listed below.

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

CI runs the same formatting and lint checks, runs the test suite under `cargo-llvm-cov`, and
uploads the LCOV report to Codecov. Add a `CODECOV_TOKEN` repository secret to enable uploads;
`fail_ci_if_error` is off so a Codecov outage does not block CI.

The optional `pylibjpeg_data` conformance test decodes the
[pylibjpeg-data](https://github.com/pydicom/pylibjpeg-data) corpus and compares lossless categories
byte-for-byte with the uncompressed `_matching_dataset`. The data (about 70 MB) is downloaded on
demand and is never committed:

```bash
sh scripts/fetch-pylibjpeg-data.sh
cargo test --all-features --test pylibjpeg_data -- --nocapture
```

Without a checkout the test skips. `.github/workflows/pylibjpeg-data.yml` runs the same steps.

## Data dictionary

Generated from the official PS3.6 (currently 2026d): 5212 standard elements (retired ones
included), 88 repeating groups, 5212 tag constants. The generator lives in
`cli/src/bin/dictgen.rs`; it depends on `dcm4rs` to validate VRs and keeps no second VR list. The
generated file is committed, so building needs no network access; see
[data/README.md](data/README.md) to regenerate:

```bash
cargo run -p cli --bin dictgen -- data/part06.xml dcm4rs/src/dictionary/generated.rs
```

## Differences from pydicom

Each row below is pinned by a test, so changing the behaviour means updating both:

| Area | pydicom | dcm4rs |
|---|---|---|
| Part 10 preamble | kept in `dataset.preamble`; `dcmwrite(enforce_file_format=True)` fills 128 zero bytes when absent | dropped on read, written as 128 zero bytes |
| File meta group length | `(0002,0000)` written and recomputed only with `enforce_file_format=True` (`filewriter.py:990-1011`) | never written (there is no enforce mode), so the default output matches |
| Data set group length | written as stored | `(xxxx,0000)` elements dropped on write |
| Ambiguous VR on write | corrected from context (`US/SS`, pixel data `OB/OW`, LUT Data, overlays) by `correct_ambiguous_vr` | corrected from the same context (Pixel Representation, Bits Allocated, Waveform Bits Allocated, LUT Descriptor) |
| Private data element VR | looked up in the private dictionary once the creator is known (`hooks._private_vr_for_tag`) | `UN` unless a caller-supplied `PrivateDictionary` resolves it |
| Undefined length on a non-SQ element | value read up to the Sequence Delimitation Item and kept as bytes (`filereader.py:319-335`) | `Error::InvalidData`, except `UN` (read as `SQ`) and `OB`/`OW` (read as fragments, see next row) |
| Undefined-length `OB`/`OW` | read as bytes up to the delimiter unless it is Pixel Data | parsed as fragments wherever it appears |
| Duplicate elements | warned about; the earlier instance is kept (`filereader.py:441-444`) | the earlier instance is kept, silently |
| `DataSet` equality | compares elements | compares elements and the byte order |
| Byte order | values are decoded to Python numbers, so writing with another endianness converts them | raw bytes stay as stored; `DataSet::to_endian` converts them explicitly |
| Character sets | full PS3.3 C.12.1.1, including the ISO 2022 code extensions | ISO_IR 6/13/100/101/109/110/126/127/138/144/148/166/192, GBK, GB18030 and ISO 2022 IR 6/13/58/87/100/101/109/110/126/127/138/144/148/149/159/166; an unknown or undeclared escape sequence reports `InvalidText` where pydicom warns and falls back to the default encoding |
| Pixel data | `pixel_array` decodes RLE, JPEG and JPEG 2000 when the plugins are installed | descriptor and frame access; RLE Lossless, 8-bit JPEG, JPEG-LS, JPEG 2000 / HTJ2K and JPEG XL are decoded, other codecs report `UnsupportedPixelData` |
| Python-side API | `PersonName`, `DSfloat`, `IS`, `repr`, VM, `walk`, `private_block`, JSON, pixel decoders, deferred reads | typed accessors (`string_values`, `decimal_values`, `integer_values`, `u16s`, ...) over raw bytes |

## Not implemented yet (next phase)

- Pixel data decoding beyond RLE Lossless, 8-bit JPEG, JPEG-LS, JPEG 2000 / HTJ2K and
  JPEG XL: JPEG lossless and MPEG; those frames are carried through as they are, still
  compressed, and report `UnsupportedPixelData` when decoding is requested.
- UL/DIMSE network layer: a separate package `dcm4rs-net` (following pynetdicom and dcm4che-net)
  that depends on `dcm4rs`. Networking needs an async/thread runtime, so it stays out of the core
  library; `dcm4rs` remains lean (association, C-FIND/C-MOVE/C-GET, ...).

## Reference implementations

Semantics follow pydicom / pynetdicom, conformance and edge-case encoding are checked against
DCMTK, the async network API shape follows fo-dicom, and the enterprise feature list follows
dcm4che.

## License

MIT OR Apache-2.0.
