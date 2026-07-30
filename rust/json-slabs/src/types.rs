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

    /// The [`SlabByteFormat`] that describes bytes read from disk for a
    /// slab of this type. Multi-byte types map to their little-endian
    /// variant since the on-disk format is always little-endian.
    pub fn to_byte_format(self) -> SlabByteFormat {
        match self {
            Self::Int8 => SlabByteFormat::I8,
            Self::Uint8 => SlabByteFormat::U8,
            Self::Int16 => SlabByteFormat::I16LE,
            Self::Uint16 => SlabByteFormat::U16LE,
            Self::Int32 => SlabByteFormat::I32LE,
            Self::Uint32 => SlabByteFormat::U32LE,
            Self::Int64 => SlabByteFormat::I64LE,
            Self::Uint64 => SlabByteFormat::U64LE,
            Self::Float32 => SlabByteFormat::F32LE,
            Self::Float64 => SlabByteFormat::F64LE,
            Self::Json => SlabByteFormat::Json,
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

/// The in-memory storage type of the elements in a [`SlabBytes`].
///
/// For multi-byte element types, this includes the endianness
/// so that a [`SlabBytes`]'s `&[u8]` can wrap e.g. a `&[u32]`
/// slab on a big-endian system without forgetting that the contents
/// need to be endian-swapped during writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlabByteFormat {
    /// `&[i8]`. Single-byte elements have no byte order.
    I8,
    /// `&[u8]`. Single-byte elements have no byte order.
    U8,
    /// `&[i16]`, little-endian: already in on-disk order.
    I16LE,
    /// `&[i16]`, big-endian: swapped on write.
    I16BE,
    /// `&[u16]`, little-endian: already in on-disk order.
    U16LE,
    /// `&[u16]`, big-endian: swapped on write.
    U16BE,
    /// `&[i32]`, little-endian: already in on-disk order.
    I32LE,
    /// `&[i32]`, big-endian: swapped on write.
    I32BE,
    /// `&[u32]`, little-endian: already in on-disk order.
    U32LE,
    /// `&[u32]`, big-endian: swapped on write.
    U32BE,
    /// `&[i64]`, little-endian: already in on-disk order.
    I64LE,
    /// `&[i64]`, big-endian: swapped on write.
    I64BE,
    /// `&[u64]`, little-endian: already in on-disk order.
    U64LE,
    /// `&[u64]`, big-endian: swapped on write.
    U64BE,
    /// `&[f32]`, little-endian: already in on-disk order.
    F32LE,
    /// `&[f32]`, big-endian: swapped on write.
    F32BE,
    /// `&[f64]`, little-endian: already in on-disk order.
    F64LE,
    /// `&[f64]`, big-endian: swapped on write.
    F64BE,
    /// UTF-8 JSON bytes, for the root skeleton slab and sub-JSON slabs.
    Json,
}

impl SlabByteFormat {
    /// The on-disk type tag corresponding to this in-memory byte format.
    ///
    /// The byte layout may not match 1:1 due to endianness differences -
    /// `SlabByteFormat` allows big-endian, but the file always stores
    /// little-endian. See [`SlabByteFormat::needs_swap_on_write`].
    pub fn on_disk_type(self) -> SlabType {
        match self {
            Self::I8 => SlabType::Int8,
            Self::U8 => SlabType::Uint8,
            Self::I16LE | Self::I16BE => SlabType::Int16,
            Self::U16LE | Self::U16BE => SlabType::Uint16,
            Self::I32LE | Self::I32BE => SlabType::Int32,
            Self::U32LE | Self::U32BE => SlabType::Uint32,
            Self::I64LE | Self::I64BE => SlabType::Int64,
            Self::U64LE | Self::U64BE => SlabType::Uint64,
            Self::F32LE | Self::F32BE => SlabType::Float32,
            Self::F64LE | Self::F64BE => SlabType::Float64,
            Self::Json => SlabType::Json,
        }
    }

    /// Size in bytes of one element of this type. Equal to
    /// `self.on_disk_type().element_size()`.
    pub fn element_size(self) -> usize {
        self.on_disk_type().element_size()
    }

    /// True when the tagged bytes are big-endian and each element must
    /// be reversed to reach the on-disk (little-endian) format.
    pub fn needs_swap_on_write(self) -> bool {
        matches!(
            self,
            Self::I16BE
                | Self::U16BE
                | Self::I32BE
                | Self::U32BE
                | Self::I64BE
                | Self::U64BE
                | Self::F32BE
                | Self::F64BE
        )
    }
}

/// A reference to a slice of bytes, annotated with the slab type.
///
/// Produced by the [`AsSlabBytes::as_slab_bytes`](crate::AsSlabBytes::as_slab_bytes)
/// trait method implementations, which is what allows [`Builder::add_slab`](crate::Builder::add_slab)
/// to accept `&[u32]` slices etc.
///
/// Also returned by [`ParsedFile::slab_at`](crate::ParsedFile::slab_at) and friends.
#[derive(Debug, Clone, Copy)]
pub struct SlabBytes<'a> {
    /// The element type and byte order of `bytes`.
    pub slab_type: SlabByteFormat,
    /// Borrowed slab bytes. Length must be a multiple of
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
/// Used as the trait bound for `T` in [`ParsedFile::read::<T>`](crate::ParsedFile::read),
/// so that `read` knows how to create the Vec elements from the bytes in
/// the file.
///
/// Also used by the [`AsSlabBytes`](crate::AsSlabBytes) implementations
/// for `&[T]` / `&[T; N]`.
pub trait SlabPrimitive: Copy {
    /// The [`SlabByteFormat`] that corresponds to this Rust primitive
    /// on the current host. Rust numeric slices are always host byte
    /// order.
    const SLAB_TYPE: SlabByteFormat;
    /// Decode a single element from its little-endian byte
    /// representation. `bytes` must be exactly
    /// `SLAB_TYPE.element_size()` long.
    fn from_le_bytes(bytes: &[u8]) -> Self;
}

impl SlabPrimitive for u8 {
    const SLAB_TYPE: SlabByteFormat = SlabByteFormat::U8;
    fn from_le_bytes(bytes: &[u8]) -> Self {
        bytes[0]
    }
}
impl SlabPrimitive for i8 {
    const SLAB_TYPE: SlabByteFormat = SlabByteFormat::I8;
    fn from_le_bytes(bytes: &[u8]) -> Self {
        bytes[0] as i8
    }
}

macro_rules! impl_slab_primitive {
    ($t:ty, $le:ident, $be:ident) => {
        impl SlabPrimitive for $t {
            #[cfg(target_endian = "little")]
            const SLAB_TYPE: SlabByteFormat = SlabByteFormat::$le;
            #[cfg(target_endian = "big")]
            const SLAB_TYPE: SlabByteFormat = SlabByteFormat::$be;
            fn from_le_bytes(bytes: &[u8]) -> Self {
                <$t>::from_le_bytes(bytes.try_into().unwrap())
            }
        }
    };
}

impl_slab_primitive!(u16, U16LE, U16BE);
impl_slab_primitive!(i16, I16LE, I16BE);
impl_slab_primitive!(u32, U32LE, U32BE);
impl_slab_primitive!(i32, I32LE, I32BE);
impl_slab_primitive!(u64, U64LE, U64BE);
impl_slab_primitive!(i64, I64LE, I64BE);
impl_slab_primitive!(f32, F32LE, F32BE);
impl_slab_primitive!(f64, F64LE, F64BE);

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
