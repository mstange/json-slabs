//! On-disk container layout: constants, raw decoded structs, and the
//! `SlabType` enum. No file-building or file-parsing logic lives here —
//! see [`crate::write`] and [`crate::read`]. This module is data-only;
//! the corresponding parsing routines ([`Header::parse`],
//! [`SlabTableEntry::parse`]) live in [`crate::read`].

use std::fmt;

/// Magic bytes at the start of every `.jslb` file.
pub const MAGIC: [u8; 8] = [0xDC, 0xDF, 0x4A, 0x53, 0x4C, 0x42, 0x01, 0x00];

/// On-disk format version this crate reads and writes.
pub const VERSION: u32 = 1;

/// Size of the fixed file header in bytes.
pub const FIXED_HEADER_SIZE: usize = 20;

/// Size of one slab-table entry in bytes.
pub const SLAB_TABLE_ENTRY_SIZE: usize = 12;

/// Element type of a slab. The discriminant matches the on-disk
/// `slab_type` field in the slab-table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SlabType {
    /// Signed 8-bit integers.
    Int8 = 0x00,
    /// Unsigned 8-bit integers.
    Uint8 = 0x01,
    /// Signed 16-bit integers, little-endian on disk.
    Int16 = 0x02,
    /// Unsigned 16-bit integers, little-endian on disk.
    Uint16 = 0x03,
    /// Signed 32-bit integers, little-endian on disk.
    Int32 = 0x04,
    /// Unsigned 32-bit integers, little-endian on disk.
    Uint32 = 0x05,
    /// IEEE 754 binary32 floats, little-endian on disk.
    Float32 = 0x06,
    /// IEEE 754 binary64 floats, little-endian on disk.
    Float64 = 0x07,
    /// Signed 64-bit integers, little-endian on disk.
    Int64 = 0x08,
    /// Unsigned 64-bit integers, little-endian on disk.
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

/// A typed view of one slab's bytes inside a parsed file. Held by
/// [`crate::read::ParsedFile`].
#[derive(Debug, Clone)]
pub struct SlabRef<'a> {
    /// The on-disk type tag of this slab.
    pub slab_type: SlabType,
    /// Borrowed slab bytes. For multi-byte numeric slab types, elements
    /// are little-endian.
    pub data: &'a [u8],
}

impl<'a> SlabRef<'a> {
    /// Number of typed elements in the slab, i.e.
    /// `data.len() / slab_type.element_size()`.
    pub fn element_count(&self) -> usize {
        self.data.len() / self.slab_type.element_size()
    }
}

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
