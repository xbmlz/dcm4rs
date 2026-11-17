# Data source

`part06.xml` is the DocBook source of DICOM PS3.6 (currently 2026d), used to generate
`dcm4rs/src/dictionary/generated.rs`. The file is about 9.6 MB and is not committed.

To fetch and regenerate:

```bash
curl -L -o data/part06.xml https://dicom.nema.org/medical/dicom/current/source/docbook/part06/part06.xml
cargo run -p cli --bin dictgen -- data/part06.xml dcm4rs/src/dictionary/generated.rs
```

The generator locates its columns by the five headers `Tag` / `Name` / `Keyword` / `VR` / `VM`
only, so layout changes in PS3.6 (new tables, added or removed description columns) do not break
parsing. It also:

- strips the zero-width spaces DocBook inserts into keywords (`U+200B` and the like);
- detects retired elements from the sixth column (`RET`);
- turns patterns such as `60xx,0010` into repeating-group entries with mask and value;
- turns `PatientName` into the `PATIENT_NAME` constant name.

To check: after generating, confirm the entry counts match the official registry (currently 5212
entries and 88 repeating groups), and spot-check common attributes with
`cargo test -p dcm4rs`.
