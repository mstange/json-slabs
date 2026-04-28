//! Parse a `.jslb` byte buffer into a [`ParsedFile`] and resolve
//! [`SlabPlaceholder`]s back to typed data. Use [`ParsedFile::parse`] as
//! the entry point; from there, use [`ParsedFile::read`] for typed-array
//! slabs and [`ParsedFile::read_subjson_bytes`] / [`ParsedFile::read_raw`]
//! / [`ParsedFile::root_json_bytes`] for sub-JSON slabs, arbitrary slab
//! bytes, and the root skeleton.

use crate::format::{
    Header, SlabRef, SlabTableEntry, SlabType, FIXED_HEADER_SIZE, MAGIC, SLAB_TABLE_ENTRY_SIZE,
    VERSION,
};
use crate::SlabPlaceholder;
use thiserror::Error;

/// Error reading the fixed header, slab table, or slab data extents.
/// Returned by [`Header::parse`], [`SlabTableEntry::parse`], and
/// [`crate::read::ParsedFile::parse`].
#[derive(Debug, Error)]
pub enum ParseError {
    /// Buffer is shorter than the 20-byte fixed header. Payload is the
    /// actual buffer length.
    #[error("file too short: need {FIXED_HEADER_SIZE} bytes, got {0}")]
    TooShort(usize),
    /// The first 8 bytes do not match [`crate::format::MAGIC`].
    #[error("bad magic bytes")]
    BadMagic,
    /// The `version` field is not [`crate::format::VERSION`].
    #[error("unsupported version {0}")]
    UnsupportedVersion(u32),
    /// The header's `slab_count` is zero. A well-formed file always has
    /// at least the root JSON slab.
    #[error("file has no slabs")]
    NoSlabs,
    /// The slab table extends past the end of the input buffer.
    #[error("slab table overruns buffer")]
    SlabTableOverrun,
    /// `root_json_slab_index` (first payload) is not less than `slab_count`
    /// (second payload).
    #[error("root slab index {0} out of range (slab count: {1})")]
    RootIndexOutOfRange(u32, u32),
    /// A slab-table entry's type byte does not decode to a known
    /// [`SlabType`].
    #[error("slab {index} has unknown type {type_byte:#x}")]
    UnknownSlabType {
        /// Slab table index of the offending entry.
        index: usize,
        /// The raw type byte that failed to decode.
        type_byte: u32,
    },
    /// A slab's byte length is not a multiple of its element size.
    #[error(
        "slab {index} byte length {byte_length} not a multiple of element size {element_size}"
    )]
    UnalignedSlabLength {
        /// Slab table index of the offending entry.
        index: usize,
        /// The reported byte length.
        byte_length: u32,
        /// The element size implied by the slab's type.
        element_size: usize,
    },
    /// A slab's `[start_offset, start_offset + byte_length)` range does not
    /// fit inside the input buffer.
    #[error("slab {index} data overruns buffer")]
    SlabDataOverrun {
        /// Slab table index of the offending entry.
        index: usize,
    },
    /// The slab designated by `root_json_slab_index` is not of type
    /// [`SlabType::Json`].
    #[error("root slab is not TYPE_JSON")]
    RootNotJson,
}

impl Header {
    /// Parse the 20-byte fixed header. Validates magic, version,
    /// `slab_count > 0`, and `root_json_slab_index < slab_count`. Does not
    /// look at the slab table.
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        if bytes.len() < FIXED_HEADER_SIZE {
            return Err(ParseError::TooShort(bytes.len()));
        }
        if bytes[0..8] != MAGIC {
            return Err(ParseError::BadMagic);
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        if version != VERSION {
            return Err(ParseError::UnsupportedVersion(version));
        }
        let slab_count = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let root_json_slab_index = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;

        if slab_count == 0 {
            return Err(ParseError::NoSlabs);
        }
        if root_json_slab_index >= slab_count {
            return Err(ParseError::RootIndexOutOfRange(
                root_json_slab_index as u32,
                slab_count as u32,
            ));
        }

        Ok(Header {
            slab_count,
            root_json_slab_index,
        })
    }
}

impl SlabTableEntry {
    /// Parse one 12-byte slab-table entry. Validates the type byte and
    /// that `byte_length` is a multiple of the element size. Does not
    /// validate that the slab fits in any particular buffer — the caller
    /// supplies that bound.
    ///
    /// `bytes` must be at least `SLAB_TABLE_ENTRY_SIZE` long; only the
    /// first 12 bytes are read.
    pub fn parse(bytes: &[u8], index: usize) -> Result<Self, ParseError> {
        let type_byte = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let start_offset = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as u64;
        let byte_length = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as u64;

        let slab_type = SlabType::try_from(type_byte)
            .map_err(|type_byte| ParseError::UnknownSlabType { index, type_byte })?;

        let element_size = slab_type.element_size() as u64;
        if !byte_length.is_multiple_of(element_size) {
            return Err(ParseError::UnalignedSlabLength {
                index,
                byte_length: byte_length as u32,
                element_size: element_size as usize,
            });
        }

        Ok(SlabTableEntry {
            slab_type,
            start_offset,
            byte_length,
        })
    }
}

/// A `.jslb` file decoded into its fixed header and a slice into each
/// slab's bytes. Holds no `serde_json` state itself — the typed
/// resolvers are inherent methods.
#[derive(Debug)]
pub struct ParsedFile<'a> {
    root_json_slab_index: usize,
    slabs: Vec<SlabRef<'a>>,
}

impl<'a> ParsedFile<'a> {
    /// Parse a `.jslb` byte buffer. Validates the fixed header, the slab
    /// table, and that each slab's bytes fit in `data`. Returns
    /// borrowed views into `data`.
    pub fn parse(data: &'a [u8]) -> Result<ParsedFile<'a>, ParseError> {
        let Header {
            slab_count,
            root_json_slab_index,
        } = Header::parse(data)?;

        let slab_table_end = FIXED_HEADER_SIZE + slab_count * SLAB_TABLE_ENTRY_SIZE;
        if slab_table_end > data.len() {
            return Err(ParseError::SlabTableOverrun);
        }

        let mut slabs = Vec::with_capacity(slab_count);
        for index in 0..slab_count {
            let pos = FIXED_HEADER_SIZE + index * SLAB_TABLE_ENTRY_SIZE;
            let entry = SlabTableEntry::parse(&data[pos..pos + SLAB_TABLE_ENTRY_SIZE], index)?;

            let start = entry.start_offset as usize;
            let end = entry
                .start_offset
                .checked_add(entry.byte_length)
                .filter(|&e| e <= data.len() as u64)
                .ok_or(ParseError::SlabDataOverrun { index })? as usize;

            slabs.push(SlabRef {
                slab_type: entry.slab_type,
                data: &data[start..end],
            });
        }

        if slabs[root_json_slab_index].slab_type != SlabType::Json {
            return Err(ParseError::RootNotJson);
        }

        Ok(ParsedFile {
            root_json_slab_index,
            slabs,
        })
    }

    /// All slabs in declaration order.
    pub fn slabs(&self) -> &[SlabRef<'a>] {
        &self.slabs
    }

    /// Index of the slab that holds the root JSON skeleton.
    pub fn root_json_slab_index(&self) -> usize {
        self.root_json_slab_index
    }

    /// Raw bytes of the root JSON skeleton slab.
    pub fn root_json_bytes(&self) -> &'a [u8] {
        self.slabs[self.root_json_slab_index].data
    }

    /// Look up a placeholder's slab. Returns the underlying [`SlabRef`] for
    /// callers that want to handle decoding themselves.
    pub fn slab_at(&self, p: SlabPlaceholder) -> Result<&SlabRef<'a>, DecodeError> {
        self.slabs.get(p.0).ok_or(DecodeError::SlabIndexOutOfRange {
            index: p.0,
            slab_count: self.slabs.len(),
        })
    }

    /// Read a typed-array slab as `Vec<T>`. The slab's stored type must
    /// match `T::SLAB_TYPE`.
    pub fn read<T: SlabElement>(&self, p: SlabPlaceholder) -> Result<Vec<T>, DecodeError> {
        let slab = self.slab_at(p)?;
        if slab.slab_type != T::SLAB_TYPE {
            return Err(DecodeError::SlabTypeMismatch {
                index: p.0,
                expected: T::SLAB_TYPE,
                found: slab.slab_type,
            });
        }
        let elem_size = T::SLAB_TYPE.element_size();
        Ok(slab
            .data
            .chunks_exact(elem_size)
            .map(T::from_le_bytes)
            .collect())
    }

    /// Read a TYPE_JSON sub-slab as raw bytes. The slab must have type
    /// [`SlabType::Json`]. Use this when you want to decode the JSON
    /// yourself instead of going through `serde_json::from_slice`.
    pub fn read_subjson_bytes(&self, p: SlabPlaceholder) -> Result<&'a [u8], DecodeError> {
        let slab = self.slab_at(p)?;
        if slab.slab_type != SlabType::Json {
            return Err(DecodeError::SlabTypeMismatch {
                index: p.0,
                expected: SlabType::Json,
                found: slab.slab_type,
            });
        }
        Ok(slab.data)
    }

    /// Raw bytes of an arbitrary slab regardless of its type. Useful for
    /// callers that want to do their own decoding (e.g. zero-copy
    /// `bytemuck::cast_slice` on aligned input).
    pub fn read_raw(&self, p: SlabPlaceholder) -> Result<&'a [u8], DecodeError> {
        Ok(self.slab_at(p)?.data)
    }
}

/// Numeric types that can be stored in a typed-array slab. Each impl knows
/// its [`SlabType`] and how to decode a single LE-encoded element. Used
/// by [`ParsedFile::read`].
pub trait SlabElement: Copy + 'static {
    /// The on-disk type tag that this Rust type maps to.
    const SLAB_TYPE: SlabType;
    /// Decode a single element from its little-endian byte representation.
    /// `bytes` must be exactly `SLAB_TYPE.element_size()` long.
    fn from_le_bytes(bytes: &[u8]) -> Self;
}

macro_rules! impl_slab_element {
    ($t:ty, $st:ident) => {
        impl SlabElement for $t {
            const SLAB_TYPE: SlabType = SlabType::$st;
            fn from_le_bytes(bytes: &[u8]) -> Self {
                <$t>::from_le_bytes(bytes.try_into().unwrap())
            }
        }
    };
}

impl SlabElement for i8 {
    const SLAB_TYPE: SlabType = SlabType::Int8;
    fn from_le_bytes(bytes: &[u8]) -> Self {
        bytes[0] as i8
    }
}
impl SlabElement for u8 {
    const SLAB_TYPE: SlabType = SlabType::Uint8;
    fn from_le_bytes(bytes: &[u8]) -> Self {
        bytes[0]
    }
}
impl_slab_element!(i16, Int16);
impl_slab_element!(u16, Uint16);
impl_slab_element!(i32, Int32);
impl_slab_element!(u32, Uint32);
impl_slab_element!(f32, Float32);
impl_slab_element!(f64, Float64);
impl_slab_element!(i64, Int64);
impl_slab_element!(u64, Uint64);

/// Error from [`ParsedFile::parse`] and the `ParsedFile::read*` resolvers.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// JSLB header / slab table is malformed.
    #[error("JSLB parse error: {0}")]
    Parse(#[from] ParseError),
    /// A [`SlabPlaceholder`] referenced an index past the end of the slab table.
    #[error("slab index {index} out of range (slab count: {slab_count})")]
    SlabIndexOutOfRange {
        /// The out-of-range placeholder index.
        index: usize,
        /// The file's total slab count.
        slab_count: usize,
    },
    /// A typed read was performed against a slab of the wrong type.
    #[error("slab {index} has type {found} but caller expected {expected}")]
    SlabTypeMismatch {
        /// Slab table index of the offending slab.
        index: usize,
        /// The type the caller asked for.
        expected: SlabType,
        /// The type the slab actually has on disk.
        found: SlabType,
    },
}
