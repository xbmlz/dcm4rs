//! Options that relax reading for files that do not follow the standard.
//!
//! The defaults keep the strict reading this crate has always done: a malformed encoding is an
//! error, not something to paper over. Every relaxation has to be asked for, so a file that
//! breaks the standard never slips through unnoticed.

/// Reading behaviour for files written by implementations that depart from the standard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadOptions {
    /// Read an undefined length on an element whose VR is neither a sequence nor a byte value by
    /// scanning forward to its item delimiter, as pydicom does.
    ///
    /// PS3.5 only allows an undefined length on `SQ`, `UN`, `OB` and `OW`. Some writers put it
    /// on other VRs anyway and end the value with an item delimiter; without this option the
    /// parser reports [`InvalidData`](crate::Error::InvalidData), because the length is the only
    /// bound the encoding gives. The bytes before the delimiter become the value, which is what
    /// pydicom's `read_undefined_length_value` does.
    pub lenient_undefined_length: bool,
}
