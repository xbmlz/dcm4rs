//! Typed views over the values this crate stores as bytes.
//!
//! The data model keeps values exactly as the standard defines them, so a file can be written
//! back byte for byte and a malformed value survives reading. That is the right storage, but it
//! makes the VRs with internal structure tedious to use: `DS` and `IS` carry a number *and* the
//! text it was written as, `PN` carries up to three groups of five components. These views parse
//! on demand and keep the original text, so nothing is lost when a value is read and written
//! back unmodified — the same reason pydicom's `DSfloat` keeps `original_string`.

mod date_time;
mod decimal_string;
mod integer_string;
mod person_name;

pub use date_time::{Date, DateTime, Time};
pub use decimal_string::DecimalString;
pub use integer_string::IntegerString;
pub use person_name::PersonName;
