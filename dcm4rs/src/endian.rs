/// Byte order.
///
/// DICOM's default transfer syntaxes are little endian; `Explicit VR Big Endian` is retired but
/// may still be encountered. This crate does not record byte order in elements: callers pass it
/// explicitly when decoding.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Endian {
    /// Little endian (the DICOM default).
    #[default]
    Little,
    /// Big endian (the retired Explicit VR Big Endian).
    Big,
}

impl Endian {
    /// Decode a 16-bit unsigned integer.
    pub fn u16(self, bytes: [u8; 2]) -> u16 {
        match self {
            Self::Little => u16::from_le_bytes(bytes),
            Self::Big => u16::from_be_bytes(bytes),
        }
    }

    /// Decode a 16-bit signed integer.
    pub fn i16(self, bytes: [u8; 2]) -> i16 {
        match self {
            Self::Little => i16::from_le_bytes(bytes),
            Self::Big => i16::from_be_bytes(bytes),
        }
    }

    /// Decode a 32-bit unsigned integer.
    pub fn u32(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }

    /// Decode a 32-bit signed integer.
    pub fn i32(self, bytes: [u8; 4]) -> i32 {
        match self {
            Self::Little => i32::from_le_bytes(bytes),
            Self::Big => i32::from_be_bytes(bytes),
        }
    }

    /// Decode a 64-bit unsigned integer.
    pub fn u64(self, bytes: [u8; 8]) -> u64 {
        match self {
            Self::Little => u64::from_le_bytes(bytes),
            Self::Big => u64::from_be_bytes(bytes),
        }
    }

    /// Decode a 64-bit signed integer.
    pub fn i64(self, bytes: [u8; 8]) -> i64 {
        match self {
            Self::Little => i64::from_le_bytes(bytes),
            Self::Big => i64::from_be_bytes(bytes),
        }
    }

    /// Decode a single-precision float (bits reinterpreted, nothing beyond the byte swap).
    pub fn f32(self, bytes: [u8; 4]) -> f32 {
        f32::from_bits(self.u32(bytes))
    }

    /// Decode a double-precision float.
    pub fn f64(self, bytes: [u8; 8]) -> f64 {
        f64::from_bits(self.u64(bytes))
    }
}
