//! DICOM data model and file I/O.
//!
//! Four constraints shape this crate:
//!
//! 1. **Stored as-is**: element values are kept as the raw bytes defined by the standard; number
//!    and text interpretation is deferred until read time. Data under any transfer syntax
//!    (explicit/implicit VR, big endian, and whatever is added later) can therefore be written
//!    back byte for byte, and the parser never has to guess.
//! 2. **Explicit endianness**: numeric decoding always takes an [`Endian`]; there is no hidden
//!    global byte-order state.
//! 3. **Lean dependencies**: character set decoding uses `encoding_rs`; JPEG decoding uses
//!    `zune-jpeg` behind the default `pixel-jpeg` feature, JPEG-LS uses `charls` behind the
//!    `pixel-jpegls` feature, JPEG 2000 uses `jpeg2k` (with the pure-Rust `openjp2` port)
//!    behind `pixel-j2k`, JPEG XL uses `jxl-oxide` behind `pixel-jxl`, and further codecs stay
//!    feature-gated, so `dcm4rs` stays embeddable in long-lived products such as a PACS.
//! 4. **Canonical writing**: sequences are written with defined lengths and group length
//!    elements are dropped, so a data set read from an undefined-length file comes back
//!    canonicalized while decoded content stays identical.
//!
//! ```
//! use dcm4rs::{DataSet, Tag, Value, Vr};
//!
//! let mut ds = DataSet::new();
//! ds.set(Tag::new(0x0010, 0x0010), Vr::Pn, Value::from_str("Doe^John"));
//! ds.set(Tag::new(0x0028, 0x0010), Vr::Us, Value::from_u16s(&[512]));
//!
//! assert_eq!(ds.string(Tag::new(0x0010, 0x0010)).unwrap(), "Doe^John");
//! assert_eq!(ds.u16(Tag::new(0x0028, 0x0010)).unwrap(), 512);
//!
//! let syntax = dcm4rs::TransferSyntax::ExplicitVrLittleEndian;
//! let bytes = ds.write(syntax).unwrap();
//! assert_eq!(DataSet::read(&bytes, syntax).unwrap(), ds);
//! ```

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![warn(missing_docs)]

mod charset;
mod dataset;
mod element;
mod endian;
mod error;
mod file;
mod private_dict;
mod reader;
mod tag;
mod transfer_syntax;
mod value;
mod vr;
mod writer;

pub mod dictionary;
pub mod pixels;

pub use charset::Charset;
pub use dataset::DataSet;
pub use element::Element;
pub use endian::Endian;
pub use error::{Error, Result};
pub use file::DicomFile;
pub use pixels::{
    PhotometricInterpretation, PixelDescriptor, PixelRepresentation, PlanarConfiguration,
};
pub use private_dict::PrivateDictionary;
pub use tag::Tag;
pub use transfer_syntax::TransferSyntax;
pub use value::Value;
pub use vr::Vr;
