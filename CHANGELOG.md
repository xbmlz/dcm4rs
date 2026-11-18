# Changelog

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
