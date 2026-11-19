//! Options that relax reading for files that do not follow the standard.
//!
//! The defaults keep the strict reading this crate has always done: a malformed encoding is an
//! error, not something to paper over. Every relaxation has to be asked for, so a file that
//! breaks the standard never slips through unnoticed.

use crate::transfer_syntax::TransferSyntax;

/// Reading behaviour for files written by implementations that depart from the standard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

    /// Leave the values of elements at least this many bytes long out of the data set.
    ///
    /// Zero, the default, reads every value. A larger value matches pydicom's `defer_size`: the
    /// element is recorded by [`crate::DicomFile::deferred_elements`] and its tag stays out of
    /// the data set until [`crate::DicomFile::load_element`] reads it back from the file. Only a
    /// source that can be read again — the bytes passed to `read`, or the path passed to
    /// `read_file` — can defer.
    ///
    /// Deferring Pixel Data is what makes [`crate::DicomFile::iter_frames`] stream: with it, each
    /// native frame is read from the source when it is asked for instead of holding the samples
    /// all at once.
    pub defer_size: usize,

    /// Read a byte stream that carries no Part 10 preamble and `DICM` prefix as a bare data set,
    /// the way pydicom's `dcmread(..., force=True)` does.
    ///
    /// The bytes are parsed whole with [`assumed_syntax`](Self::assumed_syntax), and the file
    /// meta of the result carries only `(0002,0010)`, so encoding the file again produces the
    /// part 10 wrapper it was missing. A stream that does carry the prefix is read normally.
    pub no_preamble: bool,

    /// The transfer syntax [`no_preamble`](Self::no_preamble) assumes, since a bare data set
    /// gives no way to tell. The default is Implicit VR Little Endian, what pydicom assumes.
    pub assumed_syntax: TransferSyntax,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            lenient_undefined_length: false,
            defer_size: 0,
            no_preamble: false,
            assumed_syntax: TransferSyntax::ImplicitVrLittleEndian,
        }
    }
}
