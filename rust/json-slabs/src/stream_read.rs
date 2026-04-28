//! Streaming reader for the root JSON slab of a JSLB byte stream.
//!
//! Unlike [`crate::read::ParsedFile`], which requires the whole file to be
//! in memory as `&[u8]`, [`RootJsonReader`] wraps any `Read` and exposes
//! just the root JSON slab's bytes as its own `Read`. It reads the fixed
//! header and slab table up front, skips ahead to the root slab, and then
//! yields exactly `byte_length` bytes before EOF — regardless of any
//! slabs that follow. Non-root slabs are never accessed, so callers that
//! only need the skeleton can stream a JSLB file without buffering it.

use std::io::{self, Read};

use crate::format::{Header, SlabTableEntry, SlabType, FIXED_HEADER_SIZE, SLAB_TABLE_ENTRY_SIZE};

/// Wraps a `Read` yielding a JSLB byte stream and exposes the root JSON
/// slab's bytes as its own `Read`. At construction, reads the header and
/// slab table, then discards bytes up to the root slab's start offset.
/// Subsequent reads yield exactly the root slab's `byte_length` bytes and
/// then EOF, regardless of any slabs that may follow.
///
/// If the caller wants to hand this to `serde_json::from_reader`, wrap it
/// in a `BufReader` *around* the `RootJsonReader` (not inside): the
/// `remaining` cap enforced by this type stops the buffered layer from
/// prefetching past the root slab.
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
        let mut header_buf = [0u8; FIXED_HEADER_SIZE];
        inner.read_exact(&mut header_buf)?;
        let header = Header::parse(&header_buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let mut consumed = FIXED_HEADER_SIZE as u64;
        let mut root_entry: Option<SlabTableEntry> = None;
        let mut entry_buf = [0u8; SLAB_TABLE_ENTRY_SIZE];
        for index in 0..header.slab_count {
            inner.read_exact(&mut entry_buf)?;
            consumed += SLAB_TABLE_ENTRY_SIZE as u64;
            let entry = SlabTableEntry::parse(&entry_buf, index)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            if index == header.root_json_slab_index {
                root_entry = Some(entry);
            }
        }
        // Header::parse rejects root_json_slab_index >= slab_count, so the
        // loop above always assigns root_entry.
        let root = root_entry.expect("root index in bounds");

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
