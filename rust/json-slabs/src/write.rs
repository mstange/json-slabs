//! [`Builder`] and other writing-related types.

use std::io::{self, Write};

use zerocopy::{Immutable, IntoBytes};

use crate::{
    FIXED_HEADER_SIZE, MAGIC, SLAB_TABLE_ENTRY_SIZE, SlabByteFormat, SlabBytes, SlabPlaceholder,
    SlabPrimitive, VERSION,
};

/// A trait for things like `&[i32]` which can be converted to a [`SlabBytes`].
/// Used in [`Builder::add_slab`].
///
/// Implemented for slice and array types of the integers floats, and for
/// [`JsonBytes`].
///
/// Note on endianness: The implementations for `&[i32]` and other multi-byte
/// element slices will leave everything in the native endianness, and then
/// set [`SlabByteFormat`] to the appropriate value (e.g. [`SlabByteFormat::I32BE`]
/// on a big-endian system) so that the builder knows it needs to convert on
/// writing if needed (because the file always uses little endian).
pub trait AsSlabBytes<'a> {
    /// Return the type tag and backing bytes for this slab.
    fn as_slab_bytes(&self) -> SlabBytes<'a>;
}

impl<'a, T> AsSlabBytes<'a> for &'a [T]
where
    T: SlabPrimitive + IntoBytes + Immutable,
{
    fn as_slab_bytes(&self) -> SlabBytes<'a> {
        SlabBytes {
            slab_type: T::SLAB_TYPE,
            bytes: <[T] as IntoBytes>::as_bytes(*self),
        }
    }
}

impl<'a, T, const N: usize> AsSlabBytes<'a> for &'a [T; N]
where
    T: SlabPrimitive + IntoBytes + Immutable,
{
    fn as_slab_bytes(&self) -> SlabBytes<'a> {
        SlabBytes {
            slab_type: T::SLAB_TYPE,
            bytes: <[T] as IntoBytes>::as_bytes(self.as_slice()),
        }
    }
}

/// Wraps a byte slice which should be written as a [`SlabType::Json`](crate::SlabType::Json)
/// slab.
///
/// The raw bytes are not validated — the caller is responsible for
/// producing valid UTF-8 and well-formed JSON.
#[derive(Clone, Copy)]
pub struct JsonBytes<'a>(pub &'a [u8]);

impl<'a> AsSlabBytes<'a> for JsonBytes<'a> {
    fn as_slab_bytes(&self) -> SlabBytes<'a> {
        SlabBytes {
            slab_type: SlabByteFormat::Json,
            bytes: self.0,
        }
    }
}

/// Create a JsonSlabs (JSLB) file for writing.
///
/// For slab contents, only borrowed data is accepted, and this data
/// must outlive the builder. The only exception is the root JSON slab,
/// which is supplied last, either to [`Builder::finish`] or to
/// [`Builder::to_writer`] - the root JSON data only needs to outlive
/// that call.
///
/// You can store ephemeral slab data in an `elsa::FrozenVec<Vec<u8>>`
/// outside of the `Builder` to make the lifetimes work out correctly.
/// Please file an issue if this is too cumbersome.
pub struct Builder<'a> {
    entries: Vec<SlabBytes<'a>>,
}

impl<'a> Builder<'a> {
    /// Create an empty builder with no slabs registered.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a slab. Use the returned [`SlabPlaceholder`] to refer to this
    /// slab from a JSON slab, via a `{"$s": N}` placeholder object.
    ///
    /// Only borrowed data is accepted, and this data must outlive the builder.
    ///
    /// You can store ephemeral slab data in an `elsa::FrozenVec<Vec<u8>>`
    /// outside of the `Builder` to make the lifetimes work out correctly.
    /// Please file an issue if this is too cumbersome.
    pub fn add_slab<D: AsSlabBytes<'a>>(&mut self, data: D) -> SlabPlaceholder {
        self.add_slab_bytes(data.as_slab_bytes())
    }

    /// Like [`Builder::add_slab`], but allows you to pass a [`SlabBytes`] directly.
    pub fn add_slab_bytes(&mut self, slab: SlabBytes<'a>) -> SlabPlaceholder {
        // Slab table index 0 is reserved for the root JSON slab (written
        // by `finish`), so user-added slabs are numbered starting at 1.
        let idx = self.entries.len() + 1;
        self.entries.push(slab);
        SlabPlaceholder(idx)
    }

    /// Finalize the file into a contiguous `Vec<u8>`. `root_json` is the
    /// UTF-8 JSON skeleton that becomes the root `TYPE_JSON` slab.
    pub fn finish(self, root_json: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        self.to_writer(root_json, &mut buf)
            .expect("writing to a Vec is infallible");
        buf
    }

    /// Finalize the file and write it to a writer. `root_json` is the
    /// UTF-8 JSON skeleton that becomes the root `TYPE_JSON` slab.
    pub fn to_writer<W: Write>(self, root_json: &[u8], w: &mut W) -> io::Result<()> {
        write_jslb(&self.entries, root_json, w)
    }
}

impl Default for Builder<'_> {
    fn default() -> Self {
        Self::new()
    }
}

fn write_jslb<W: Write>(entries: &[SlabBytes<'_>], root_json: &[u8], w: &mut W) -> io::Result<()> {
    let slab_count = entries.len() + 1;
    // Slab table index 0 is reserved for the root JSON slab, so a
    // streaming consumer can read the first slab table entry and
    // immediately know where the root JSON ends. Placeholder indices
    // returned by `add_slab` / `add_slab_bytes` account for this and
    // start at 1.
    let root_json_slab_index = 0;
    let slab_table_end = FIXED_HEADER_SIZE + slab_count * SLAB_TABLE_ENTRY_SIZE;

    let all_slabs = || {
        std::iter::once(SlabBytes {
            slab_type: SlabByteFormat::Json,
            bytes: root_json,
        })
        .chain(entries.iter().copied())
    };

    let mut offsets = Vec::with_capacity(slab_count);
    let mut pos = slab_table_end;
    for sb in all_slabs() {
        let align = sb.slab_type.element_size();
        pos = (pos + align - 1) & !(align - 1);
        offsets.push(pos);
        pos += sb.bytes.len();
    }

    w.write_all(&MAGIC)?;
    w.write_all(&VERSION.to_le_bytes())?;
    w.write_all(&(slab_count as u32).to_le_bytes())?;
    w.write_all(&(root_json_slab_index as u32).to_le_bytes())?;

    for (sb, &offset) in all_slabs().zip(offsets.iter()) {
        w.write_all(&(sb.slab_type.on_disk_type() as u32).to_le_bytes())?;
        w.write_all(&(offset as u32).to_le_bytes())?;
        w.write_all(&(sb.bytes.len() as u32).to_le_bytes())?;
    }

    let mut written = slab_table_end;
    for (sb, &offset) in all_slabs().zip(offsets.iter()) {
        write_padding(w, offset - written)?;
        written = offset;
        write_slab_data(w, sb.slab_type, sb.bytes)?;
        written += sb.bytes.len();
    }
    Ok(())
}

fn write_padding<W: Write>(w: &mut W, n: usize) -> io::Result<()> {
    const PAD: [u8; 8] = [0; 8];
    let mut remaining = n;
    while remaining > 0 {
        let chunk = remaining.min(PAD.len());
        w.write_all(&PAD[..chunk])?;
        remaining -= chunk;
    }
    Ok(())
}

fn write_slab_data<W: Write>(w: &mut W, in_type: SlabByteFormat, bytes: &[u8]) -> io::Result<()> {
    if !in_type.needs_swap_on_write() {
        return w.write_all(bytes);
    }
    let elem_size = in_type.element_size();
    debug_assert!(bytes.len().is_multiple_of(elem_size));
    let mut buf = [0u8; 8];
    for chunk in bytes.chunks_exact(elem_size) {
        for (i, &b) in chunk.iter().enumerate() {
            buf[elem_size - 1 - i] = b;
        }
        w.write_all(&buf[..elem_size])?;
    }
    Ok(())
}
