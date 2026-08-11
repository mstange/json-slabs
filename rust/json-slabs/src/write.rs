//! [`Builder`] and other writing-related types.

use std::io::{self, BufWriter, Write};
use std::marker::PhantomData;

use crate::{
    FIXED_HEADER_SIZE, MAGIC, SLAB_TABLE_ENTRY_SIZE, SlabPlaceholder, SlabPrimitive, SlabType,
    VERSION,
};

/// A slab whose bytes are produced lazily at write time, for use
/// with [`Builder::add_streamed_slab`]`.
///
/// This trait is dyn-compatible; the builder stores `Box<dyn StreamedSlab>`.
///
/// You usually don't need to implement this. [`Builder::add_slab`] (which
/// takes a slice), [`Builder::add_slab_from_iter`] (which takes an iterator)
/// and [`Builder::add_json_slab`] should be sufficient for most use cases,
/// and they use internal implementations of this trait.
pub trait StreamedSlab {
    /// The slab type tag written into the slab table.
    fn slab_type(&self) -> SlabType;

    /// The exact number of bytes [`write`](Self::write) will produce.
    /// Called at add-slab time to plan slab table offsets.
    fn byte_length(&self) -> usize;

    /// Produce the slab bytes into `w`. Must write exactly
    /// [`byte_length`](Self::byte_length) bytes.
    ///
    /// This takes a `Box<Self>`, which is the dyn-compatible way of
    /// making a method that can only be called once.
    fn write(self: Box<Self>, w: &mut dyn Write) -> io::Result<()>;
}

/// Write all elements of `iter` as little-endian bytes to `w`.
/// We batch through a `BufWriter` to make the dynamic dispatch to the
/// `&mut dyn Write` happen once per batch rather than once per element.
fn write_le_iter<T, It>(iter: It, w: &mut dyn Write) -> io::Result<()>
where
    T: SlabPrimitive,
    It: Iterator<Item = T>,
{
    let mut bw = BufWriter::with_capacity(4096, w);
    for elem in iter {
        bw.write_all(elem.to_le_bytes().as_ref())?;
    }
    bw.flush()
}

/// Write exactly `count` elements from `iter` as little-endian bytes to `w`.
///
/// # Panics
///
/// Panics if `iter` yields a number of items other than `count`. The count
/// was used to lay out the file before any bytes were written, so a
/// mismatch is a caller bug that cannot be recovered from at this point;
/// see [`Builder::add_slab_from_iter`].
fn write_le_iter_exact<T, It>(
    mut iter: It,
    count: usize,
    slab_index: usize,
    w: &mut dyn Write,
) -> io::Result<()>
where
    T: SlabPrimitive,
    It: Iterator<Item = T>,
{
    let mut bw = BufWriter::with_capacity(4096, w);
    let mut written = 0usize;
    while written < count {
        let Some(elem) = iter.next() else {
            panic!(
                "slab {slab_index}: iterator passed to add_slab_from_iter yielded \
                 {written} items, but {count} were declared"
            );
        };
        bw.write_all(elem.to_le_bytes().as_ref())?;
        written += 1;
    }
    // Pull one more item to catch an over-long iterator. The count is a
    // hard contract in both directions: extra items would silently not
    // make it into the file.
    if iter.next().is_some() {
        panic!(
            "slab {slab_index}: iterator passed to add_slab_from_iter yielded \
             more than the {count} declared items"
        );
    }
    bw.flush()
}

/// [`StreamedSlab`] implementation for iterators.
///
/// When we write out the iterator data, there is one dynamic dispatch
/// to call `IterSlab::write` (i.e. one for the entire slab), but then
/// the `iter.next()` calls do **not** use dynamic dispatch because
/// `IterSlab` is monomorphized for the iterator type.
struct IterSlab<T, It> {
    iter: It,
    count: usize,
    /// Only used to identify this slab in the panic message if `iter`
    /// turns out not to yield exactly `count` items.
    slab_index: usize,
    _t: PhantomData<T>,
}

impl<T, It> StreamedSlab for IterSlab<T, It>
where
    T: SlabPrimitive,
    It: Iterator<Item = T>,
{
    fn slab_type(&self) -> SlabType {
        T::SLAB_TYPE
    }
    fn byte_length(&self) -> usize {
        self.count * std::mem::size_of::<T>()
    }
    fn write(self: Box<Self>, w: &mut dyn Write) -> io::Result<()> {
        write_le_iter_exact(self.iter, self.count, self.slab_index, w)
    }
}

/// Write `slice` as little-endian bytes to `w`.
fn write_slice_le<T: SlabPrimitive>(slice: &[T], w: &mut dyn Write) -> io::Result<()> {
    // On little-endian hosts, the slab bytes are already the on-disk
    // representation and can be written with a single `write_all`; on
    // big-endian hosts, we fall through to the per-element encoding loop.
    if cfg!(target_endian = "little") {
        // SAFETY: `SlabPrimitive` requires `sealed::Sealed`, whose
        // (unsafe) contract is precisely that `T` is valid to view as
        // `size_of::<T>()` initialized bytes — no padding, no invalid
        // bit patterns, no interior mutability. `u8` has alignment 1, so
        // the resulting slice is trivially aligned, and it borrows from
        // `slice` so the lifetime is correct. On little-endian hosts
        // those bytes are already the on-disk representation.
        let bytes = unsafe {
            std::slice::from_raw_parts(slice.as_ptr() as *const u8, std::mem::size_of_val(slice))
        };
        w.write_all(bytes)
    } else {
        write_le_iter(slice.iter().copied(), w)
    }
}

/// [`StreamedSlab`] implementation for slices, for [`Builder::add_slab`].
struct SliceSlab<'a, T: SlabPrimitive> {
    slice: &'a [T],
}

impl<T: SlabPrimitive> StreamedSlab for SliceSlab<'_, T> {
    fn slab_type(&self) -> SlabType {
        T::SLAB_TYPE
    }
    fn byte_length(&self) -> usize {
        std::mem::size_of_val(self.slice)
    }
    fn write(self: Box<Self>, w: &mut dyn Write) -> io::Result<()> {
        write_slice_le(self.slice, w)
    }
}

/// [`StreamedSlab`] implementation for owned vectors, for
/// [`Builder::add_slab_from_vec`]. Same write path as [`SliceSlab`],
/// but the builder owns the elements so there is no lifetime bound.
struct VecSlab<T: SlabPrimitive>(Vec<T>);

impl<T: SlabPrimitive> StreamedSlab for VecSlab<T> {
    fn slab_type(&self) -> SlabType {
        T::SLAB_TYPE
    }
    fn byte_length(&self) -> usize {
        std::mem::size_of_val(&self.0[..])
    }
    fn write(self: Box<Self>, w: &mut dyn Write) -> io::Result<()> {
        write_slice_le(&self.0, w)
    }
}

/// [`StreamedSlab`] impl for [`Builder::add_json_slab`] — the whole
/// slab is a `Vec<u8>` already, so `write` is just a `write_all`.
struct JsonSlab(Vec<u8>);

impl StreamedSlab for JsonSlab {
    fn slab_type(&self) -> SlabType {
        SlabType::Json
    }
    fn byte_length(&self) -> usize {
        self.0.len()
    }
    fn write(self: Box<Self>, w: &mut dyn Write) -> io::Result<()> {
        w.write_all(&self.0)
    }
}

/// Create a JsonSlabs (JSLB) file for writing.
///
/// Callers add slabs one at a time and receive [`SlabPlaceholder`]s that
/// can be interpolated into the root JSON skeleton (via the type's
/// `Display` impl). Slab bytes are produced lazily: an iterator-backed
/// slab isn't consumed until [`Builder::finish`] / [`Builder::to_writer`]
/// runs, so no intermediate `Vec<T>` is materialized.
///
/// The `'a` lifetime bounds anything an added iterator borrows.
pub struct Builder<'a> {
    entries: Vec<Box<dyn StreamedSlab + 'a>>,
}

impl<'a> Builder<'a> {
    /// Create an empty builder with no slabs registered.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a typed-array slab from a borrowed slice. The slice needs
    /// to outlive the builder.
    ///
    /// If your slice doesn't live long enough, use
    /// [`add_slab_from_vec`](Self::add_slab_from_vec) instead.
    pub fn add_slab<T>(&mut self, values: &'a [T]) -> SlabPlaceholder
    where
        T: SlabPrimitive + 'a,
    {
        self.add_streamed_slab(Box::new(SliceSlab { slice: values }))
    }

    /// Add a typed-array slab from an owned `Vec`.
    ///
    /// Use this when you cannot use [`Builder::add_slab`] (which takes a
    /// slice that outlives the builder).
    pub fn add_slab_from_vec<T>(&mut self, values: Vec<T>) -> SlabPlaceholder
    where
        T: SlabPrimitive + 'a,
    {
        self.add_streamed_slab(Box::new(VecSlab(values)))
    }

    /// Add a typed-array slab whose elements are produced by an iterator.
    ///
    /// The iterator must yield exactly `count` items. The count is needed
    /// up-front to reserve the right amount of space in the file and store
    /// the offsets in a table that's written out before the slab.
    ///
    /// # Panics
    ///
    /// [`Builder::to_writer`] / [`Builder::finish`] panics if the iterator
    /// yields a number of items other than `count`. By the time the
    /// iterator runs, `count` has already been baked into the slab table,
    /// so a mismatch cannot be repaired — it is a caller bug, in the same
    /// family as passing a bad length to `slice::copy_from_slice`.
    ///
    /// Note that `to_writer` writes as it goes, so this panic can leave
    /// the writer holding a partial, unusable file.
    ///
    /// If your element count comes from a source that might disagree with
    /// the iterator (a length field in an input file, say), validate it
    /// before calling this, or collect the elements and use
    /// [`add_slab_from_vec`](Self::add_slab_from_vec).
    pub fn add_slab_from_iter<T, I>(&mut self, count: usize, iter: I) -> SlabPlaceholder
    where
        T: SlabPrimitive + 'a,
        I: IntoIterator<Item = T>,
        I::IntoIter: 'a,
    {
        self.add_streamed_slab(Box::new(IterSlab::<T, I::IntoIter> {
            iter: iter.into_iter(),
            count,
            slab_index: self.next_slab_index(),
            _t: PhantomData,
        }))
    }

    /// Add a JSON sub-document slab. The `bytes` are moved into the
    /// builder and written as-is at flush time.
    ///
    /// The bytes are not validated: the caller is responsible for
    /// producing well-formed UTF-8 JSON.
    pub fn add_json_slab(&mut self, bytes: Vec<u8>) -> SlabPlaceholder {
        self.add_streamed_slab(Box::new(JsonSlab(bytes)))
    }

    /// Advanced: add a slab whose bytes are produced by a custom
    /// [`StreamedSlab`] impl. Prefer [`add_slab`](Self::add_slab) or
    /// [`add_json_slab`](Self::add_json_slab).
    pub fn add_streamed_slab(&mut self, s: Box<dyn StreamedSlab + 'a>) -> SlabPlaceholder {
        let idx = self.next_slab_index();
        self.entries.push(s);
        SlabPlaceholder(idx)
    }

    /// The slab table index the next added slab will get. Index 0 is
    /// reserved for the root JSON slab (written by `finish`), so
    /// user-added slabs are numbered starting at 1.
    fn next_slab_index(&self) -> usize {
        self.entries.len() + 1
    }

    /// Finalize the file into a contiguous `Vec<u8>`. `root_json` is the
    /// UTF-8 JSON skeleton that becomes the root `SlabType::Json` slab.
    ///
    /// # Panics
    ///
    /// Panics if a slab added with
    /// [`add_slab_from_iter`](Self::add_slab_from_iter) yields a number of
    /// items other than its declared count.
    pub fn finish(self, root_json: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        self.to_writer(root_json, &mut buf)
            .expect("writing to a Vec is infallible");
        buf
    }

    /// Finalize the file and write it to a writer. `root_json` is the
    /// UTF-8 JSON skeleton that becomes the root `SlabType::Json` slab.
    ///
    /// Bytes are streamed to `w` as they are produced, so a returned
    /// `Err` (which always comes from `w` itself) leaves `w` holding a
    /// partial, unusable file.
    ///
    /// # Panics
    ///
    /// Panics if a slab added with
    /// [`add_slab_from_iter`](Self::add_slab_from_iter) yields a number of
    /// items other than its declared count.
    pub fn to_writer<W: Write>(self, root_json: &[u8], w: &mut W) -> io::Result<()> {
        write_jslb(self.entries, root_json, w)
    }
}

impl Default for Builder<'_> {
    fn default() -> Self {
        Self::new()
    }
}

fn write_jslb<W: Write>(
    entries: Vec<Box<dyn StreamedSlab + '_>>,
    root_json: &[u8],
    w: &mut W,
) -> io::Result<()> {
    let slab_count = entries.len() + 1;
    // Slab table index 0 is reserved for the root JSON slab, so a
    // streaming consumer can read the first slab table entry and
    // immediately know where the root JSON ends. Placeholder indices
    // returned by `add_slab` start at 1.
    let root_json_slab_index: u32 = 0;
    let slab_table_end = FIXED_HEADER_SIZE + slab_count * SLAB_TABLE_ENTRY_SIZE;

    // Two-pass over entries. First pass gathers (slab_type, byte_length)
    // via the trait methods to compute offsets. Second pass moves each
    // boxed streamer through `StreamedSlab::write` (which takes
    // `self: Box<Self>`) to produce the bytes.
    let mut header_rows: Vec<(SlabType, usize, usize)> = Vec::with_capacity(slab_count);
    let mut pos = slab_table_end;

    // Row 0: root JSON.
    let align = SlabType::Json.element_size();
    pos = (pos + align - 1) & !(align - 1);
    header_rows.push((SlabType::Json, pos, root_json.len()));
    pos += root_json.len();

    // Rows 1..: user entries.
    for e in &entries {
        let ty = e.slab_type();
        let align = ty.element_size();
        pos = (pos + align - 1) & !(align - 1);
        let len = e.byte_length();
        header_rows.push((ty, pos, len));
        pos += len;
    }

    // Fixed header + slab table.
    w.write_all(&MAGIC)?;
    w.write_all(&VERSION.to_le_bytes())?;
    w.write_all(&(slab_count as u32).to_le_bytes())?;
    w.write_all(&root_json_slab_index.to_le_bytes())?;

    for &(ty, offset, len) in &header_rows {
        w.write_all(&(ty as u32).to_le_bytes())?;
        w.write_all(&(offset as u32).to_le_bytes())?;
        w.write_all(&(len as u32).to_le_bytes())?;
    }

    // Slab payloads, with alignment padding as computed above.
    let mut written = slab_table_end;
    let mut rows = header_rows.into_iter();

    // Root JSON.
    let (_, root_offset, root_len) = rows.next().expect("root row");
    write_padding(w, root_offset - written)?;
    written = root_offset;
    w.write_all(root_json)?;
    written += root_len;

    // User entries.
    for (entry, (_, offset, len)) in entries.into_iter().zip(rows) {
        write_padding(w, offset - written)?;
        written = offset;
        entry.write(w)?;
        written += len;
    }
    debug_assert_eq!(written, pos, "planned and actual file length disagree");
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
