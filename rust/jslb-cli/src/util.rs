use anyhow::{anyhow, Context, Result};
use json_slabs::format::{Header, SlabTableEntry, FIXED_HEADER_SIZE, SLAB_TABLE_ENTRY_SIZE};
use std::fs::File;
use std::io::{self, Read};

/// Read and parse the fixed header and slab table from the front of a
/// `.jslb` file. Does not validate that slab bodies fit in the file —
/// callers that care must check `start_offset + byte_length` against
/// the file length themselves.
pub fn read_header_and_table(file: &mut File) -> Result<(Header, Vec<SlabTableEntry>)> {
    let mut hdr = [0u8; FIXED_HEADER_SIZE];
    file.read_exact(&mut hdr).context("reading JSLB header")?;
    let header = Header::parse(&hdr)?;

    let table_len = header
        .slab_count
        .checked_mul(SLAB_TABLE_ENTRY_SIZE)
        .ok_or_else(|| anyhow!("slab table size overflow"))?;
    let mut table = vec![0u8; table_len];
    file.read_exact(&mut table)
        .context("reading JSLB slab table")?;

    let mut entries = Vec::with_capacity(header.slab_count);
    for i in 0..header.slab_count {
        let off = i * SLAB_TABLE_ENTRY_SIZE;
        entries.push(SlabTableEntry::parse(
            &table[off..off + SLAB_TABLE_ENTRY_SIZE],
            i,
        )?);
    }
    Ok((header, entries))
}

/// Read until `buf` is full or EOF. Returns the number of bytes read
/// (0 at EOF).
pub fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match r.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

pub fn digits(n: usize) -> usize {
    if n == 0 {
        1
    } else {
        n.ilog10() as usize + 1
    }
}
