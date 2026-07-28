use json_slabs::SlabTableEntry;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

/// A `Read` adapter over a byte range of an underlying `File`. Each
/// call to `read` re-seeks to the reader's remembered position before
/// reading, so multiple `SlabReader`s sharing a `&File` (e.g. the
/// skeleton parser's reader plus a `SlabReader` for an in-progress
/// typed-array placeholder) don't step on each other's offsets.
///
/// Why we re-seek every time: the file's offset is shared. `Read for
/// &File` and `Seek for &File` (both stable) operate on that shared
/// offset, and so does any other `SlabReader` over the same `File`.
/// Re-seeking before each read costs one extra syscall per buffered
/// chunk, which is negligible compared to the read itself.
pub struct SlabReader<'a> {
    file: &'a File,
    offset: u64,
    remaining: u64,
}

impl<'a> SlabReader<'a> {
    pub fn new(file: &'a File, desc: &SlabTableEntry) -> Self {
        Self {
            file,
            offset: desc.start_offset,
            remaining: desc.byte_length,
        }
    }
}

impl Read for SlabReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let want = (buf.len() as u64).min(self.remaining) as usize;
        let mut f = self.file;
        f.seek(SeekFrom::Start(self.offset))?;
        let n = f.read(&mut buf[..want])?;
        self.offset += n as u64;
        self.remaining -= n as u64;
        Ok(n)
    }
}
