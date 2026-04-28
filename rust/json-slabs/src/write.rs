//! Build a `.jslb` file from a collection of typed slabs and a root JSON
//! skeleton. Start with [`Builder::new`], call [`Builder::add_slab`] for
//! each slab (which returns a [`SlabPlaceholder`] to reference in the
//! skeleton), and finish with [`Builder::finish`] or
//! [`Builder::to_writer`].

use std::io::{self, Write};

use zerocopy::{Immutable, IntoBytes};

use crate::format::{SlabType, FIXED_HEADER_SIZE, MAGIC, SLAB_TABLE_ENTRY_SIZE, VERSION};
use crate::SlabPlaceholder;

/// Carries a slice of bytes, along with the type tag.
///
/// The bytes are stored in host byte order. On little-endian hosts (the
/// common case) that matches the on-disk format directly. On big-endian
/// hosts the writer reorders each element when emitting the file, based
/// on the slab type's element size — see [`Builder::to_writer`].
#[derive(Debug, Clone, Copy)]
pub struct SlabBytes<'a> {
    /// The on-disk type tag of the slab.
    pub slab_type: SlabType,
    /// The slab's element bytes in host byte order. Length must be a
    /// multiple of `slab_type.element_size()`.
    pub bytes: &'a [u8],
}

/// A piece of data that can be written into a JSLB slab. Returns its
/// on-disk [`SlabType`] together with the backing bytes as a
/// [`SlabBytes`], with a lifetime tied to the source data via the trait's
/// `'a` parameter.
///
/// Built-in impls cover the numeric primitive slice and array types
/// (`&[T]` and `&[T; N]` for `T ∈ {u8, i8, u16, i16, u32, i32, u64, i64,
/// f32, f64}`) and [`JsonBytes`].
pub trait AsSlabBytes<'a> {
    /// Return the type tag and backing bytes for this slab.
    fn as_slab_bytes(&self) -> SlabBytes<'a>;
}

/// Numeric primitive that can appear as the element type of a typed-array
/// slab. Maps the Rust type to its on-disk [`SlabType`] and asserts (via
/// the `zerocopy` bounds) that reinterpreting `&[T]` as `&[u8]` is sound.
pub trait SlabPrimitive: IntoBytes + Immutable {
    /// The on-disk type tag that corresponds to this Rust primitive.
    const SLAB_TYPE: SlabType;
}

macro_rules! impl_slab_primitive {
    ($t:ty, $st:ident) => {
        impl SlabPrimitive for $t {
            const SLAB_TYPE: SlabType = SlabType::$st;
        }
    };
}

impl_slab_primitive!(u8, Uint8);
impl_slab_primitive!(i8, Int8);
impl_slab_primitive!(u16, Uint16);
impl_slab_primitive!(i16, Int16);
impl_slab_primitive!(u32, Uint32);
impl_slab_primitive!(i32, Int32);
impl_slab_primitive!(u64, Uint64);
impl_slab_primitive!(i64, Int64);
impl_slab_primitive!(f32, Float32);
impl_slab_primitive!(f64, Float64);

impl<'a, T: SlabPrimitive> AsSlabBytes<'a> for &'a [T] {
    fn as_slab_bytes(&self) -> SlabBytes<'a> {
        SlabBytes {
            slab_type: T::SLAB_TYPE,
            bytes: <[T] as IntoBytes>::as_bytes(*self),
        }
    }
}

impl<'a, T: SlabPrimitive, const N: usize> AsSlabBytes<'a> for &'a [T; N] {
    fn as_slab_bytes(&self) -> SlabBytes<'a> {
        SlabBytes {
            slab_type: T::SLAB_TYPE,
            bytes: <[T] as IntoBytes>::as_bytes(self.as_slice()),
        }
    }
}

/// Wraps a byte slice that should be written as a [`SlabType::Json`]
/// slab. The raw bytes are not validated — the caller is responsible for
/// producing well-formed JSON.
#[derive(Clone, Copy)]
pub struct JsonBytes<'a>(pub &'a [u8]);

impl<'a> AsSlabBytes<'a> for JsonBytes<'a> {
    fn as_slab_bytes(&self) -> SlabBytes<'a> {
        SlabBytes {
            slab_type: SlabType::Json,
            bytes: self.0,
        }
    }
}

/// Accumulates borrowed slab byte ranges and emits the finished `.jslb`
/// file. Slab data lives in caller-managed storage for the builder's
/// `'a` lifetime; the builder only holds [`SlabBytes`] pairs.
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

    /// Register a slab. The returned [`SlabPlaceholder`] addresses the
    /// slab by index in the order it was added; pass it through the
    /// skeleton JSON (directly or via the serde repr `{"$s": N}`) so the
    /// reader can find it.
    pub fn add_slab<D: AsSlabBytes<'a>>(&mut self, data: D) -> SlabPlaceholder {
        self.add_slab_bytes(data.as_slab_bytes())
    }

    /// Register a pre-resolved [`SlabBytes`]. Use when the slab type is
    /// only known at runtime, or when you've already obtained the
    /// `(SlabType, &[u8])` pair from somewhere else (e.g. forwarding
    /// from a parsed file). The caller is responsible for ensuring the
    /// byte length is a multiple of the type's element size.
    pub fn add_slab_bytes(&mut self, slab: SlabBytes<'a>) -> SlabPlaceholder {
        // Slab table index 0 is reserved for the root JSON slab (written
        // by `finish`), so user-added slabs are numbered starting at 1.
        let idx = self.entries.len() + 1;
        self.entries.push(slab);
        SlabPlaceholder(idx)
    }

    /// Finalize the file into a contiguous `Vec<u8>`. `root_json` is the
    /// UTF-8 JSON skeleton that becomes the root TYPE_JSON slab.
    pub fn finish(self, root_json: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        self.to_writer(root_json, &mut buf)
            .expect("writing to a Vec is infallible");
        buf
    }

    /// Stream the `.jslb` byte image into `w`. Multi-byte slab data is
    /// flipped to little-endian on big-endian hosts (per slab element
    /// size); on little-endian hosts data passes through unchanged.
    /// Inter-slab alignment padding is written as zero bytes.
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
        std::iter::once((SlabType::Json, root_json))
            .chain(entries.iter().map(|sb| (sb.slab_type, sb.bytes)))
    };

    let mut offsets = Vec::with_capacity(slab_count);
    let mut pos = slab_table_end;
    for (slab_type, data) in all_slabs() {
        let align = slab_type.element_size();
        pos = (pos + align - 1) & !(align - 1);
        offsets.push(pos);
        pos += data.len();
    }

    w.write_all(&MAGIC)?;
    w.write_all(&VERSION.to_le_bytes())?;
    w.write_all(&(slab_count as u32).to_le_bytes())?;
    w.write_all(&(root_json_slab_index as u32).to_le_bytes())?;

    for ((slab_type, data), &offset) in all_slabs().zip(offsets.iter()) {
        w.write_all(&(slab_type as u32).to_le_bytes())?;
        w.write_all(&(offset as u32).to_le_bytes())?;
        w.write_all(&(data.len() as u32).to_le_bytes())?;
    }

    let mut written = slab_table_end;
    for ((slab_type, data), &offset) in all_slabs().zip(offsets.iter()) {
        write_padding(w, offset - written)?;
        written = offset;
        write_slab_data(w, slab_type, data)?;
        written += data.len();
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

#[cfg(target_endian = "little")]
fn write_slab_data<W: Write>(w: &mut W, _slab_type: SlabType, bytes: &[u8]) -> io::Result<()> {
    w.write_all(bytes)
}

#[cfg(target_endian = "big")]
fn write_slab_data<W: Write>(w: &mut W, slab_type: SlabType, bytes: &[u8]) -> io::Result<()> {
    let elem_size = slab_type.element_size();
    if elem_size == 1 {
        return w.write_all(bytes);
    }
    let mut buf = [0u8; 8];
    for chunk in bytes.chunks_exact(elem_size) {
        for (i, &b) in chunk.iter().enumerate() {
            buf[elem_size - 1 - i] = b;
        }
        w.write_all(&buf[..elem_size])?;
    }
    Ok(())
}
