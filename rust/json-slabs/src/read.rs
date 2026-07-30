//! [`ParsedFile`] and [`SlabDirectory`], and other reading-related types.

use std::io::{self, Read};

use crate::{
    FIXED_HEADER_SIZE, Header, MAGIC, SLAB_TABLE_ENTRY_SIZE, SlabByteFormat, SlabBytes,
    SlabPlaceholder, SlabPrimitive, SlabTableEntry, SlabType, VERSION,
};
use thiserror::Error;

/// Error reading the fixed header, slab table, or slab data extents.
/// Returned by [`Header::parse`], [`SlabTableEntry::parse`], and
/// [`ParsedFile::parse`].
#[derive(Debug, Error)]
pub enum ParseError {
    /// Buffer is shorter than the 20-byte fixed header. Payload is the
    /// actual buffer length.
    #[error("file too short: need {FIXED_HEADER_SIZE} bytes, got {0}")]
    TooShort(usize),
    /// The first 8 bytes do not match [`MAGIC`].
    #[error("bad magic bytes")]
    BadMagic,
    /// The `version` field is not [`VERSION`].
    #[error("unsupported version {0}")]
    UnsupportedVersion(u32),
    /// The header's `slab_count` is zero. A well-formed file always has
    /// at least the root JSON slab.
    #[error("file has no slabs")]
    NoSlabs,
    /// The slab table extends past the end of the input buffer.
    #[error("slab table overruns buffer")]
    SlabTableOverrun,
    /// The header's `root_json_slab_index` is not less than its `slab_count`.
    #[error("root slab index {index} out of range (slab count: {slab_count})")]
    RootIndexOutOfRange {
        /// The out-of-range root slab index.
        index: usize,
        /// The file's total slab count.
        slab_count: usize,
    },
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
    #[error("slab {index} byte length {byte_length} not a multiple of element size {element_size}")]
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
            return Err(ParseError::RootIndexOutOfRange {
                index: root_json_slab_index,
                slab_count,
            });
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

/// Low-level alternative to [`ParsedFile`]. A `SlabDirectory` consumes the entire
/// front-part of the file, including the header.
#[derive(Debug, Clone)]
pub struct SlabDirectory {
    /// Decoded 20-byte fixed header.
    pub header: Header,
    /// One entry per slab, in slab-table order.
    pub entries: Vec<SlabTableEntry>,
}

impl SlabDirectory {
    /// Parse the fixed header and slab table from the front of an
    /// in-memory buffer. Payload bytes are not read or validated; call
    /// [`SlabDirectory::validate_extents`] separately to check that
    /// every slab body fits.
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let header = Header::parse(bytes)?;

        let table_end = FIXED_HEADER_SIZE + header.slab_count * SLAB_TABLE_ENTRY_SIZE;
        if table_end > bytes.len() {
            return Err(ParseError::SlabTableOverrun);
        }

        let mut entries = Vec::with_capacity(header.slab_count);
        for i in 0..header.slab_count {
            let pos = FIXED_HEADER_SIZE + i * SLAB_TABLE_ENTRY_SIZE;
            entries.push(SlabTableEntry::parse(
                &bytes[pos..pos + SLAB_TABLE_ENTRY_SIZE],
                i,
            )?);
        }

        Ok(Self { header, entries })
    }

    /// Read the fixed header and slab table from a stream. Advances
    /// `r` by exactly `FIXED_HEADER_SIZE + slab_count *
    /// SLAB_TABLE_ENTRY_SIZE` bytes on success. Payload bytes are not
    /// touched — the reader is left positioned at the first byte after
    /// the slab table.
    ///
    /// Header/table decode errors are surfaced as
    /// [`io::ErrorKind::InvalidData`].
    pub fn read<R: Read>(r: &mut R) -> io::Result<Self> {
        let mut header_buf = [0u8; FIXED_HEADER_SIZE];
        r.read_exact(&mut header_buf)?;
        let header = Header::parse(&header_buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let table_len = header
            .slab_count
            .checked_mul(SLAB_TABLE_ENTRY_SIZE)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "slab table size overflow")
            })?;
        let mut table = vec![0u8; table_len];
        r.read_exact(&mut table)?;

        let mut entries = Vec::with_capacity(header.slab_count);
        for i in 0..header.slab_count {
            let pos = i * SLAB_TABLE_ENTRY_SIZE;
            let entry = SlabTableEntry::parse(&table[pos..pos + SLAB_TABLE_ENTRY_SIZE], i)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            entries.push(entry);
        }

        Ok(Self { header, entries })
    }

    /// Check that every slab's `[start_offset, start_offset +
    /// byte_length)` range fits within a container of `file_len` bytes.
    pub fn validate_extents(&self, file_len: u64) -> Result<(), ParseError> {
        for (index, entry) in self.entries.iter().enumerate() {
            entry
                .start_offset
                .checked_add(entry.byte_length)
                .filter(|&e| e <= file_len)
                .ok_or(ParseError::SlabDataOverrun { index })?;
        }
        Ok(())
    }

    /// Index of the slab that holds the root JSON skeleton.
    pub fn root_json_index(&self) -> usize {
        self.header.root_json_slab_index
    }

    /// Slab-table entry for the root JSON skeleton slab.
    pub fn root_entry(&self) -> &SlabTableEntry {
        &self.entries[self.root_json_index()]
    }

    /// Total byte size of the header + slab table (i.e. the byte offset
    /// at which slab payloads may start).
    pub fn header_and_table_size(&self) -> u64 {
        (FIXED_HEADER_SIZE + self.entries.len() * SLAB_TABLE_ENTRY_SIZE) as u64
    }
}

/// The JSLB file contents, parsed from a slice of bytes.
///
/// Use [`SlabDirectory`] for a lower-level reading API, or if you don't
/// want to materialize the entire file contents into memory.
#[derive(Debug)]
pub struct ParsedFile<'a> {
    root_json_slab_index: usize,
    slabs: Vec<SlabBytes<'a>>,
}

impl<'a> ParsedFile<'a> {
    /// Parse a `.jslb` byte buffer. Validates the fixed header, the slab
    /// table, that each slab's bytes fit in `data`, and that the root
    /// slab has type [`SlabType::Json`]. Returns borrowed views into
    /// `data`. Multi-byte slabs are tagged as their `*LE` variant, since
    /// the on-disk format is always little-endian.
    pub fn parse(data: &'a [u8]) -> Result<ParsedFile<'a>, ParseError> {
        let dir = SlabDirectory::parse(data)?;
        dir.validate_extents(data.len() as u64)?;

        let slabs = dir
            .entries
            .iter()
            .map(|entry| SlabBytes {
                slab_type: entry.slab_type.to_byte_format(),
                bytes: &data[entry.start_offset as usize
                    ..(entry.start_offset + entry.byte_length) as usize],
            })
            .collect::<Vec<_>>();

        let root_json_slab_index = dir.header.root_json_slab_index;
        if slabs[root_json_slab_index].slab_type != SlabByteFormat::Json {
            return Err(ParseError::RootNotJson);
        }

        Ok(ParsedFile {
            root_json_slab_index,
            slabs,
        })
    }

    /// All slabs in declaration order.
    pub fn slabs(&self) -> &[SlabBytes<'a>] {
        &self.slabs
    }

    /// Index of the slab that holds the root JSON skeleton.
    pub fn root_json_slab_index(&self) -> usize {
        self.root_json_slab_index
    }

    /// Raw bytes of the root JSON skeleton slab.
    pub fn root_json_bytes(&self) -> &'a [u8] {
        self.slabs[self.root_json_slab_index].bytes
    }

    /// Look up a placeholder's slab. Returns the underlying [`SlabBytes`]
    /// for callers that want to handle decoding themselves — or forward
    /// the value straight into [`Builder::add_slab_bytes`](crate::Builder::add_slab_bytes)
    /// for zero-copy re-emission.
    pub fn slab_at(&self, p: SlabPlaceholder) -> Result<&SlabBytes<'a>, DecodeError> {
        self.slabs.get(p.0).ok_or(DecodeError::SlabIndexOutOfRange {
            index: p.0,
            slab_count: self.slabs.len(),
        })
    }

    /// Read a typed-array slab as `Vec<T>`. The slab's stored on-disk
    /// type must match `T`'s on-disk type.
    pub fn read<T: SlabPrimitive>(&self, p: SlabPlaceholder) -> Result<Vec<T>, DecodeError> {
        let slab = self.slab_at(p)?;
        let found = slab.slab_type.on_disk_type();
        let expected = T::SLAB_TYPE.on_disk_type();
        if found != expected {
            return Err(DecodeError::SlabTypeMismatch {
                index: p.0,
                expected,
                found,
            });
        }
        let elem_size = expected.element_size();
        Ok(slab
            .bytes
            .chunks_exact(elem_size)
            .map(T::from_le_bytes)
            .collect())
    }

    /// Read a TYPE_JSON sub-slab as raw bytes. The slab must have type
    /// [`SlabType::Json`]. Use this when you want to decode the JSON
    /// yourself instead of going through `serde_json::from_slice`.
    pub fn read_subjson_bytes(&self, p: SlabPlaceholder) -> Result<&'a [u8], DecodeError> {
        let slab = self.slab_at(p)?;
        let found = slab.slab_type.on_disk_type();
        if found != SlabType::Json {
            return Err(DecodeError::SlabTypeMismatch {
                index: p.0,
                expected: SlabType::Json,
                found,
            });
        }
        Ok(slab.bytes)
    }

    /// Raw bytes of an arbitrary slab regardless of its type. Useful for
    /// callers that want to do their own decoding (e.g. zero-copy
    /// `bytemuck::cast_slice` on aligned input; multi-byte slabs are
    /// always little-endian on disk).
    pub fn read_raw(&self, p: SlabPlaceholder) -> Result<&'a [u8], DecodeError> {
        Ok(self.slab_at(p)?.bytes)
    }
}

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

/// Streaming reader exposing only the root JSON slab of a JSLB byte stream.
///
/// This can be used by consumers which are only interested in the JSON
/// at the root of the JSLB file and not in anything else.
///
/// Useful for compressed JSLB files (e.g. .jslb.gz) because you'll only
/// end up compressing the start of the file, provided the root JSON slab
/// is the first slab.
#[derive(Debug)]
pub struct RootJsonReader<R: Read> {
    inner: R,
    remaining: u64,
}

impl<R: Read> RootJsonReader<R> {
    /// Wrap `inner`, reading the fixed header and slab table so that
    /// subsequent reads yield exactly the root JSON slab's bytes.
    ///
    /// Fails with [`io::ErrorKind::InvalidData`] if the header or slab
    /// table is malformed, if the designated root slab is not
    /// [`SlabType::Json`], or if the root slab's byte range overlaps the
    /// slab table. Fails with [`io::ErrorKind::UnexpectedEof`] if the
    /// stream ends before the root slab's start offset.
    pub fn new(mut inner: R) -> io::Result<Self> {
        let dir = SlabDirectory::read(&mut inner)?;
        let consumed = dir.header_and_table_size();
        let root = *dir.root_entry();

        if root.slab_type != SlabType::Json {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSLB root slab is not TYPE_JSON",
            ));
        }
        if root.start_offset < consumed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "JSLB root slab overlaps the slab table",
            ));
        }

        let to_skip = root.start_offset - consumed;
        let skipped = io::copy(&mut (&mut inner).take(to_skip), &mut io::sink())?;
        if skipped < to_skip {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "JSLB stream ended before the root JSON slab",
            ));
        }

        Ok(Self {
            inner,
            remaining: root.byte_length,
        })
    }
}

impl<R: Read> Read for RootJsonReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let take = (buf.len() as u64).min(self.remaining) as usize;
        let n = self.inner.read(&mut buf[..take])?;
        self.remaining -= n as u64;
        Ok(n)
    }
}
