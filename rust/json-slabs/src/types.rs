//! Types related to the JSLB (JsonSlabs) file format.

use std::fmt;

/// Magic bytes at the start of every `.jslb` file.
pub const MAGIC: [u8; 8] = [0xDC, 0xDF, 0x4A, 0x53, 0x4C, 0x42, 0x01, 0x00];

/// On-disk format version this crate reads and writes.
pub const VERSION: u32 = 1;

/// The object key that marks a slab reference in a JSON slab.
///
/// A JSON object of the form `{ "$s": <slab-index> }` — and no other
/// members — is a placeholder for the contents of another slab. See
/// [`SlabPlaceholder`].
pub const SLAB_REF_KEY: &str = "$s";

/// Size of the fixed file header in bytes.
pub const FIXED_HEADER_SIZE: usize = 20;

/// Size of one slab-table entry in bytes. These entries immediately follow
/// the file header.
pub const SLAB_TABLE_ENTRY_SIZE: usize = 12;

/// The type of a slab, as stored in the file.
///
/// All numbers are stored in little-endian byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SlabType {
    /// An array of signed 8-bit integers.
    Int8 = 0x00,
    /// An array of unsigned 8-bit integers.
    Uint8 = 0x01,
    /// An array of signed 16-bit integers, little-endian on disk.
    Int16 = 0x02,
    /// An array of unsigned 16-bit integers, little-endian on disk.
    Uint16 = 0x03,
    /// An array of signed 32-bit integers, little-endian on disk.
    Int32 = 0x04,
    /// An array of unsigned 32-bit integers, little-endian on disk.
    Uint32 = 0x05,
    /// An array of iEEE 754 binary32 floats, little-endian on disk.
    Float32 = 0x06,
    /// An array of iEEE 754 binary64 floats, little-endian on disk.
    Float64 = 0x07,
    /// An array of signed 64-bit integers, little-endian on disk.
    Int64 = 0x08,
    /// An array of unsigned 64-bit integers, little-endian on disk.
    Uint64 = 0x09,
    /// UTF-8 JSON bytes. Used both for the root skeleton slab and for
    /// sub-JSON slabs referenced from the skeleton.
    Json = 0x0a,
}

impl SlabType {
    /// Size in bytes of a single element of this slab type.
    pub fn element_size(self) -> usize {
        match self {
            Self::Int16 | Self::Uint16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::Float64 | Self::Int64 | Self::Uint64 => 8,
            _ => 1,
        }
    }

    /// Short human-readable name (`"i32"`, `"f64"`, `"json"`, ...).
    /// Used by [`fmt::Display`].
    pub fn name(self) -> &'static str {
        match self {
            Self::Int8 => "i8",
            Self::Uint8 => "u8",
            Self::Int16 => "i16",
            Self::Uint16 => "u16",
            Self::Int32 => "i32",
            Self::Uint32 => "u32",
            Self::Float32 => "f32",
            Self::Float64 => "f64",
            Self::Int64 => "i64",
            Self::Uint64 => "u64",
            Self::Json => "json",
        }
    }
}

impl fmt::Display for SlabType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl TryFrom<u32> for SlabType {
    type Error = u32;
    fn try_from(v: u32) -> Result<Self, Self::Error> {
        Ok(match v {
            0x00 => Self::Int8,
            0x01 => Self::Uint8,
            0x02 => Self::Int16,
            0x03 => Self::Uint16,
            0x04 => Self::Int32,
            0x05 => Self::Uint32,
            0x06 => Self::Float32,
            0x07 => Self::Float64,
            0x08 => Self::Int64,
            0x09 => Self::Uint64,
            0x0a => Self::Json,
            _ => return Err(v),
        })
    }
}

/// A reference to a slice of slab bytes, annotated with the slab type.
///
/// Returned by [`ParsedFile::slab_at`](crate::ParsedFile::slab_at) and friends.
/// Multi-byte integer/float slabs are always little-endian on disk.
#[derive(Debug, Clone, Copy)]
pub struct SlabBytes<'a> {
    /// The element type of `bytes`.
    pub slab_type: SlabType,
    /// Borrowed slab bytes. Length is a multiple of
    /// `slab_type.element_size()`.
    pub bytes: &'a [u8],
}

impl<'a> SlabBytes<'a> {
    /// Number of typed elements in the slab, i.e.
    /// `bytes.len() / slab_type.element_size()`.
    pub fn element_count(&self) -> usize {
        self.bytes.len() / self.slab_type.element_size()
    }
}

/// Numeric primitive that can appear as the element type of a
/// typed-array slab, implemented by `i8`, `u8`, `i16`, etc.
///
/// Used as the trait bound for `T` in [`ParsedFile::read::<T>`](crate::ParsedFile::read)
/// and the various [`Builder::add_slab`](crate::Builder::add_slab) methods.
///
/// This trait is sealed, because we want to be able to write a slice
/// of primitives to the file by reinterpreting the slice of primitives
/// as a slice of bytes (which works at least on little-endian machines;
/// the file format always uses little-endian). We use a sealed (and
/// unsafe) marker trait to indicate that this reinterpretion is sound.
pub trait SlabPrimitive: Copy + sealed::Sealed {
    /// The [`SlabType`] tag for this Rust primitive on disk.
    const SLAB_TYPE: SlabType;
    /// Fixed-size byte array type produced by [`to_le_bytes`](Self::to_le_bytes).
    type LeBytes: AsRef<[u8]>;
    /// Encode a single element as its little-endian byte representation.
    fn to_le_bytes(self) -> Self::LeBytes;
    /// Decode a single element from its little-endian byte
    /// representation. `bytes` must be exactly
    /// `SLAB_TYPE.element_size()` long.
    fn from_le_bytes(bytes: &[u8]) -> Self;
}

mod sealed {
    /// This makes it so that [`SlabPrimitive`](super::SlabPrimitive)
    /// cannot be implemented outside this crate.
    ///
    /// This trait also guarantees the invariant that the `unsafe` block
    /// in `write_slice_le` relies on when it reinterprets `&[Self]` as
    /// `&[u8]`.
    ///
    /// # Safety
    ///
    /// `Self` must be valid to view as `size_of::<Self>()` initialized
    /// bytes: no padding, no uninhabited or niche-restricted values, no
    /// interior mutability, and no pointers (whose bytes are not
    /// meaningful on disk). In practice: only the built-in integer and
    /// float primitives qualify.
    pub unsafe trait Sealed {}
}

// SAFETY for all of the following: these are the built-in integer and
// float primitives. Each is a fixed-size scalar with no padding, no
// invalid bit patterns, and no interior mutability.
unsafe impl sealed::Sealed for u8 {}
unsafe impl sealed::Sealed for i8 {}
unsafe impl sealed::Sealed for u16 {}
unsafe impl sealed::Sealed for i16 {}
unsafe impl sealed::Sealed for u32 {}
unsafe impl sealed::Sealed for i32 {}
unsafe impl sealed::Sealed for u64 {}
unsafe impl sealed::Sealed for i64 {}
unsafe impl sealed::Sealed for f32 {}
unsafe impl sealed::Sealed for f64 {}

impl SlabPrimitive for u8 {
    const SLAB_TYPE: SlabType = SlabType::Uint8;
    type LeBytes = [u8; 1];
    fn to_le_bytes(self) -> [u8; 1] {
        [self]
    }
    fn from_le_bytes(bytes: &[u8]) -> Self {
        bytes[0]
    }
}
impl SlabPrimitive for i8 {
    const SLAB_TYPE: SlabType = SlabType::Int8;
    type LeBytes = [u8; 1];
    fn to_le_bytes(self) -> [u8; 1] {
        [self as u8]
    }
    fn from_le_bytes(bytes: &[u8]) -> Self {
        bytes[0] as i8
    }
}

macro_rules! impl_slab_primitive {
    ($t:ty, $tag:ident, $n:literal) => {
        impl SlabPrimitive for $t {
            const SLAB_TYPE: SlabType = SlabType::$tag;
            type LeBytes = [u8; $n];
            fn to_le_bytes(self) -> [u8; $n] {
                <$t>::to_le_bytes(self)
            }
            fn from_le_bytes(bytes: &[u8]) -> Self {
                <$t>::from_le_bytes(bytes.try_into().unwrap())
            }
        }
    };
}

impl_slab_primitive!(u16, Uint16, 2);
impl_slab_primitive!(i16, Int16, 2);
impl_slab_primitive!(u32, Uint32, 4);
impl_slab_primitive!(i32, Int32, 4);
impl_slab_primitive!(u64, Uint64, 8);
impl_slab_primitive!(i64, Int64, 8);
impl_slab_primitive!(f32, Float32, 4);
impl_slab_primitive!(f64, Float64, 8);

/// Decoded form of the 20-byte fixed header.
#[derive(Debug, Clone, Copy)]
pub struct Header {
    /// Total number of slabs in the file (including the root JSON slab).
    pub slab_count: usize,
    /// Slab table index of the root JSON skeleton slab. Guaranteed to be
    /// less than `slab_count` when produced by [`Header::parse`].
    pub root_json_slab_index: usize,
}

/// One slab-table entry, decoded but not yet bound to file/buffer data.
#[derive(Debug, Clone, Copy)]
pub struct SlabTableEntry {
    /// The on-disk type tag of the slab.
    pub slab_type: SlabType,
    /// Byte offset from the start of the file to the slab's first byte.
    pub start_offset: u64,
    /// Slab length in bytes. Guaranteed to be a multiple of
    /// `slab_type.element_size()` when produced by [`SlabTableEntry::parse`].
    pub byte_length: u64,
}

/// A reference to a slab by its index in the file's slab table.
///
/// Returned from [`Builder::add_slab`](crate::Builder::add_slab) when building a file,
/// and accepted by [`ParsedFile::read`](crate::ParsedFile::read) and friends when
/// reading one back. The inner `usize` is the raw slab index.
///
/// This type implements [`fmt::Display`]:
/// - `format!("{}", SlabPlaceholder(2))` produces `2`
/// - `format!("{:#}, SlabPlaceholder(2))` produces `{"$s":2}`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlabPlaceholder(pub usize);

impl SlabPlaceholder {
    /// The underlying slab table index.
    pub fn index(self) -> usize {
        self.0
    }
}

impl fmt::Display for SlabPlaceholder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, r#"{{"{SLAB_REF_KEY}":{}}}"#, self.0)
        } else {
            self.0.fmt(f)
        }
    }
}
