# Changelog

## 0.3.0 — 2026-11-19

### Added

- `DicomDir` and `DirectoryRecord`, the media storage directory of PS3.10 8.5. The record tree is
  rebuilt from each record's offset and sibling flags, so the byte offsets a writer computed are
  not needed to read it.
- `Waveform`, which decodes the multiplex groups of `(5400,0100)` into per-channel samples with
  Channel Sensitivity, its correction factor and Channel Baseline applied (PS3.3 C.10.9.1).
- The `mpeg` module: the MPEG-2 sequence header read without decoding the video, giving Rows,
  Columns, Cine Rate, Frame Time and Pixel Aspect Ratio. `DicomFile::mpeg2_sequence_header` reads
  it and `apply_mpeg2_sequence_header` writes those attributes into the data set.
- The DICOM JSON model of PS3.18 F.2: `DataSet::to_json` and `DataSet::from_json`, with values
  decoded to the type their VR names. `BulkDataURI` is written but never fetched.
- `PrivateBlock`, with `DataSet::private_block` and `DataSet::private_block_or_create`, following
  the creator block layout of PS3.5 7.8.1.
- Pixel value transforms of PS3.3 C.11.2 and C.11.6: `apply_modality_lut`, `apply_voi_lut` and
  `apply_windowing` work one frame at a time and never change the stored pixel data.
- Deferred reads: `ReadOptions::defer_size` keeps long values out of the data set, which
  `DicomFile::deferred_elements`, `is_deferred`, `load_element` and `load_deferred` then read back
  from the source the file was read from.
- Compression, which turns native Pixel Data into encapsulated frames: `compress_rle` (RLE
  Lossless), `compress_jpeg` (JPEG Baseline), `compress_jpegls` (JPEG-LS Lossless), `compress_j2k`
  (JPEG 2000 Lossless) and `compress_jxl` (JPEG XL Lossless, through `zune-jpegxl`).
- `DicomFile::from_parts` builds a file from its meta group, data set and preamble.

### Changed

- `pixel-jxl` encodes through `zune-jpegxl` instead of `oxideav-jpegxl` and needs Rust 1.80.

## 0.2.0 — 2026-11-18

First public release, ported from pydicom's `dataset`, `dictionary`, `uid` and `encaps` modules and
checked against the pylibjpeg-data conformance corpus.

### Added

- `DataSet`, `Element`, `Tag` and the PS3.6 data dictionary, with explicit VR little and big
  endian, implicit VR and deflated transfer syntaxes.
- `DicomFile`: Part 10 files with a file meta group, sequences of defined or undefined length,
  and encapsulated pixel data. The 128-byte preamble is kept as read.
- `ReadOptions::lenient_undefined_length`, which relaxes the one common malformed encoding: an
  undefined length on a VR that does not allow it, scanned to the item delimiter the way pydicom
  does.
- Typed value views over the stored bytes: `Date`, `Time`, `DateTime`, `PersonName`,
  `DecimalString` and `IntegerString`.
- `Uid`, backed by the PS3.6 registry, and `is_valid` for syntactic checks.
- `PrivateDictionary` and `itemize_fragment` for private tags and encapsulated fragments.
- `DataSet::duplicates`, listing tags the stream carried more than once.
- Pixel decoders: JPEG Baseline and Extended 12-bit, JPEG Lossless (Process 14), and the optional
  `pixel-jpegls`, `pixel-j2k` and `pixel-jxl` features.

### Changed

- `DicomFile` carries a `preamble: [u8; 128]` field. Code that builds a `DicomFile` with a struct
  literal must supply it; `DicomFile::new` fills it with zeros.
- `DataSet` equality compares elements only; the byte order and the character set are not part of
  it, so two data sets read from the same bytes in different encodings compare equal.
