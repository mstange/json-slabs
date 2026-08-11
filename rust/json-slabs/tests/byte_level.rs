//! End-to-end tests exercising the crate's read + write APIs. The
//! crate has no serde dependency; callers hand-build the root JSON
//! skeleton (or use their own JSON library).

use json_slabs::{Builder, DecodeError, ParsedFile, SlabDirectory, SlabPlaceholder, SlabType};

/// Hand-build a JSLB with a typed-array slab and a sub-JSON slab,
/// using `add_slab` / `add_json_slab` and `finish`, then read it back
/// with `read::<T>` / `read_subjson_bytes` / `root_json_bytes`.
#[test]
fn byte_level_roundtrip() {
    let mut b = Builder::new();
    let frame = b.add_slab(&[10i32, 20, 30]);
    let sub = b.add_json_slab(br#"{"name":"Firefox","version":28}"#.to_vec());

    // Hand-built skeleton — `SlabPlaceholder` is `pub usize`, so we
    // can interpolate the index directly.
    let skeleton = format!(
        r#"{{"frame":{{"$s":{}}},"meta":{{"$s":{}}}}}"#,
        frame.index(),
        sub.index(),
    );
    let bytes = b.finish(skeleton.as_bytes());

    let parsed = ParsedFile::parse(&bytes).expect("parse");

    // Round-trip the typed slab.
    let frame_back: Vec<i32> = parsed.read(frame).expect("frame");
    assert_eq!(frame_back, vec![10, 20, 30]);

    // Round-trip the sub-JSON slab as raw bytes.
    let sub_bytes = parsed.read_subjson_bytes(sub).expect("sub");
    assert_eq!(sub_bytes, br#"{"name":"Firefox","version":28}"#);

    // The skeleton itself is just the bytes we wrote.
    let skel_bytes = parsed.root_json_bytes();
    assert!(skel_bytes.starts_with(br#"{"frame":{"$s":"#));
}

/// `read_subjson_bytes` validates that the slab type is JSON.
#[test]
fn read_subjson_bytes_rejects_typed_array() {
    let mut b = Builder::new();
    let typed = b.add_slab(&[1u32, 2, 3]);
    let skeleton = format!(r#"{{"x":{{"$s":{}}}}}"#, typed.index());
    let bytes = b.finish(skeleton.as_bytes());

    let parsed = ParsedFile::parse(&bytes).unwrap();
    let err = parsed
        .read_subjson_bytes(typed)
        .expect_err("should reject typed-array slab");
    match err {
        DecodeError::SlabTypeMismatch {
            index,
            expected,
            found,
        } => {
            assert_eq!(index, 1);
            assert_eq!(expected, SlabType::Json);
            assert_eq!(found, SlabType::Uint32);
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
}

/// `add_json_slab` produces a slab tagged as [`SlabType::Json`].
#[test]
fn add_json_slab_creates_json_slab() {
    let mut b = Builder::new();
    let p = b.add_json_slab(b"[1,2,3]".to_vec());
    let skeleton = format!(r#"{{"x":{{"$s":{}}}}}"#, p.index());
    let bytes = b.finish(skeleton.as_bytes());
    let parsed = ParsedFile::parse(&bytes).unwrap();
    let slab = parsed.slab_at(p).unwrap();
    assert_eq!(slab.slab_type, SlabType::Json);
    assert_eq!(slab.bytes, b"[1,2,3]");
}

/// `SlabPlaceholder(usize)` constructor and `index()` accessor work
/// without any serde at all.
#[test]
fn placeholder_construct_and_inspect() {
    let p = SlabPlaceholder(42);
    assert_eq!(p.index(), 42);
    assert_eq!(p.to_string(), "42");
    assert_eq!(format!("{p:#}"), r#"{"$s":42}"#);
}

/// `Builder::to_writer` streams the same bytes that `Builder::finish`
/// produces in memory.
#[test]
fn to_writer_matches_finish() {
    let frame: &[i32] = &[1, 2, -3, i32::MAX];
    let weights: &[f64] = &[0.5, 1.0, 1.5];
    let skeleton = r#"{"f":{"$s":1},"w":{"$s":2}}"#;

    let mut b1 = Builder::new();
    let _ = b1.add_slab(frame);
    let _ = b1.add_slab(weights);
    let from_finish = b1.finish(skeleton.as_bytes());

    let mut b2 = Builder::new();
    let _ = b2.add_slab(frame);
    let _ = b2.add_slab(weights);
    let mut buf: Vec<u8> = Vec::new();
    b2.to_writer(skeleton.as_bytes(), &mut buf)
        .expect("to_writer");

    assert_eq!(from_finish, buf);
    let parsed = ParsedFile::parse(&buf).expect("parse");
    assert_eq!(parsed.slabs().len(), 3);
}

/// `add_slab_from_vec` is the owned counterpart of `add_slab`: it
/// produces identical bytes, and the source data may be dropped before
/// the builder is finished.
#[test]
fn add_slab_from_vec_matches_add_slab() {
    let frame: &[i32] = &[1, 2, -3, i32::MAX];
    let weights: &[f64] = &[0.5, 1.0, 1.5];
    let skeleton = r#"{"f":{"$s":1},"w":{"$s":2}}"#;

    let mut b1 = Builder::new();
    let _ = b1.add_slab(frame);
    let _ = b1.add_slab(weights);
    let borrowed = b1.finish(skeleton.as_bytes());

    let mut b2 = Builder::new();
    {
        // Both sources go out of scope before `finish`.
        let frame_owned = frame.to_vec();
        let weights_owned = weights.to_vec();
        let _ = b2.add_slab_from_vec(frame_owned);
        let _ = b2.add_slab_from_vec(weights_owned);
    }
    let owned = b2.finish(skeleton.as_bytes());

    assert_eq!(borrowed, owned);
}

/// Reading past the end of the slab table returns a structured error
/// rather than panicking.
#[test]
fn slab_index_out_of_range_is_structured_error() {
    let b: Builder = Builder::new();
    let bytes = b.finish(b"{}");
    let parsed = ParsedFile::parse(&bytes).unwrap();
    let err = parsed
        .read::<f64>(SlabPlaceholder(7))
        .expect_err("should reject");
    match err {
        DecodeError::SlabIndexOutOfRange { index, slab_count } => {
            assert_eq!(index, 7);
            // Only the root JSON slab is present.
            assert_eq!(slab_count, 1);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

/// Each slab's start offset is padded up to its element size, so JS
/// readers can take zero-copy typed-array views on aligned input.
#[test]
fn slab_offsets_are_aligned() {
    let mut b = Builder::new();
    let a = b.add_slab(&[1u8, 2, 3]);
    let c = b.add_slab(&[10u16]);
    let d = b.add_slab(&[100u32, 200, 300]);
    let e = b.add_slab(&[1.5f64, 2.5]);
    let skeleton = format!(
        r#"{{"a":{{"$s":{}}},"c":{{"$s":{}}},"d":{{"$s":{}}},"e":{{"$s":{}}}}}"#,
        a.index(),
        c.index(),
        d.index(),
        e.index(),
    );
    let bytes = b.finish(skeleton.as_bytes());
    let parsed = ParsedFile::parse(&bytes).unwrap();
    for (i, slab) in parsed.slabs().iter().enumerate() {
        let elem_size = slab.slab_type.element_size();
        let start = slab.bytes.as_ptr() as usize - bytes.as_ptr() as usize;
        assert_eq!(
            start % elem_size,
            0,
            "slab {i} ({:?}) starts at offset {start}, not a multiple of {elem_size}",
            slab.slab_type
        );
    }
}

/// Every supported primitive type roundtrips through a typed-array slab.
#[test]
fn all_element_types_roundtrip() {
    macro_rules! check {
        ($ty:ty, $vals:expr) => {{
            let mut b = Builder::new();
            let p = b.add_slab(&$vals);
            let skeleton = format!(r#"{{"x":{{"$s":{}}}}}"#, p.index());
            let bytes = b.finish(skeleton.as_bytes());
            let parsed = ParsedFile::parse(&bytes).unwrap();
            let back: Vec<$ty> = parsed.read::<$ty>(p).unwrap();
            assert_eq!(back, $vals.to_vec());
        }};
    }
    check!(i8, [-1_i8, 0, 127]);
    check!(u8, [1_u8, 2, 255]);
    check!(i16, [-1000_i16, 1000]);
    check!(u16, [0_u16, 65535]);
    check!(i32, [-100_000_i32, 100_000]);
    check!(u32, [1_u32, 4_000_000_000]);
    check!(f32, [1.5_f32, -2.5]);
    check!(f64, [std::f64::consts::PI, -1e100]);
    check!(i64, [i64::MIN, 0, i64::MAX]);
    check!(u64, [0_u64, u64::MAX]);
}

/// `SlabDirectory::parse` (buffer) and `SlabDirectory::read` (stream)
/// agree on the header and slab table for the same file.
#[test]
fn slab_directory_read_matches_parse() {
    let mut b = Builder::new();
    let frame = b.add_slab(&[10i32, 20, 30]);
    let sub = b.add_json_slab(br#"{"name":"Firefox"}"#.to_vec());
    let skeleton = format!(
        r#"{{"frame":{{"$s":{}}},"meta":{{"$s":{}}}}}"#,
        frame.index(),
        sub.index(),
    );
    let bytes = b.finish(skeleton.as_bytes());

    let from_buf = SlabDirectory::parse(&bytes).unwrap();
    let from_stream = {
        let mut cursor = std::io::Cursor::new(&bytes);
        SlabDirectory::read(&mut cursor).unwrap()
    };

    assert_eq!(from_buf.header.slab_count, from_stream.header.slab_count);
    assert_eq!(
        from_buf.header.root_json_slab_index,
        from_stream.header.root_json_slab_index,
    );
    assert_eq!(from_buf.entries.len(), from_stream.entries.len());
    for (a, b) in from_buf.entries.iter().zip(&from_stream.entries) {
        assert_eq!(a.slab_type, b.slab_type);
        assert_eq!(a.start_offset, b.start_offset);
        assert_eq!(a.byte_length, b.byte_length);
    }

    // Extents fit inside the buffer; root points at a JSON slab.
    from_buf.validate_extents(bytes.len() as u64).unwrap();
    assert_eq!(from_buf.root_entry().slab_type, SlabType::Json);
    assert_eq!(
        from_buf.header_and_table_size(),
        from_buf.root_entry().start_offset,
        "no gap between slab table and the root JSON slab in this file",
    );
}

/// `SlabDirectory::read` leaves the stream positioned exactly at the
/// first byte after the slab table, so callers can seek from there.
#[test]
fn slab_directory_read_leaves_cursor_after_table() {
    let mut b = Builder::new();
    let p = b.add_slab(&[1u32, 2, 3]);
    let skeleton = format!(r#"{{"x":{{"$s":{}}}}}"#, p.index());
    let bytes = b.finish(skeleton.as_bytes());

    let mut cursor = std::io::Cursor::new(&bytes);
    let dir = SlabDirectory::read(&mut cursor).unwrap();
    assert_eq!(cursor.position(), dir.header_and_table_size());
}

/// `validate_extents` catches a slab that would run past the end of the
/// stated file length.
#[test]
fn slab_directory_validate_extents_rejects_overrun() {
    let mut b = Builder::new();
    let _ = b.add_slab(&[1u32, 2, 3]);
    let bytes = b.finish(br#"{}"#);
    let dir = SlabDirectory::parse(&bytes).unwrap();
    // Pretend the file is a byte shorter than it really is.
    let short = (bytes.len() - 1) as u64;
    assert!(dir.validate_extents(short).is_err());
}

/// A `.scan()`-based iterator (non-`ExactSizeIterator`) can be passed
/// through `add_slab_from_iter` — the explicit `count` parameter is
/// what the builder needs. This exercises the "deltas" pattern.
#[test]
fn scan_iterator_streams_correctly() {
    let times: [u64; 4] = [10, 30, 45, 100];
    let iter = times.iter().scan(0u64, |prev, &cur| {
        let delta = cur - *prev;
        *prev = cur;
        Some(delta as f64)
    });
    let mut b = Builder::new();
    let p = b.add_slab_from_iter(times.len(), iter);
    let skeleton = format!(r#"{{"x":{{"$s":{}}}}}"#, p.index());
    let bytes = b.finish(skeleton.as_bytes());
    let parsed = ParsedFile::parse(&bytes).unwrap();
    let back: Vec<f64> = parsed.read::<f64>(p).unwrap();
    assert_eq!(back, vec![10.0, 20.0, 15.0, 55.0]);
}

/// The slice-based `add_slab` (which takes the LE fast path on
/// little-endian hosts by reinterpreting `&[T]` as `&[u8]`) produces
/// byte-identical output to the per-element iterator path.
#[test]
fn slice_fast_path_matches_iter_path() {
    let vals: [i32; 5] = [-1, 0, 1, i32::MIN, i32::MAX];

    let mut b1 = Builder::new();
    let p1 = b1.add_slab(&vals);
    let bytes1 = b1.finish(format!(r#"{{"x":{p1:#}}}"#).as_bytes());

    let mut b2 = Builder::new();
    let p2 = b2.add_slab_from_iter(vals.len(), vals.iter().copied());
    let bytes2 = b2.finish(format!(r#"{{"x":{p2:#}}}"#).as_bytes());

    assert_eq!(bytes1, bytes2);
}

/// The `count` passed to `add_slab_from_iter` is a hard contract: it is
/// baked into the slab table before the iterator runs, so a short
/// iterator is a caller bug and panics rather than producing a file
/// whose slab table lies about its contents. The panic message names the
/// slab so it can be found among many.
#[test]
#[should_panic(expected = "slab 2: iterator passed to add_slab_from_iter yielded 3 items")]
fn short_iterator_panics() {
    let mut b = Builder::new();
    b.add_slab(&[1u8, 2, 3]);
    let p = b.add_slab_from_iter(5, vec![1.0f64, 2.0, 3.0]);
    b.finish(format!(r#"{{"x":{p:#}}}"#).as_bytes());
}

/// Likewise for an over-long iterator: the extra items would silently
/// not make it into the file.
#[test]
#[should_panic(expected = "yielded more than the 2 declared items")]
fn long_iterator_panics() {
    let mut b = Builder::new();
    let p = b.add_slab_from_iter(2, vec![1.0f64, 2.0, 3.0]);
    b.finish(format!(r#"{{"x":{p:#}}}"#).as_bytes());
}

/// An owned `Vec<T>` can be handed to `add_slab_from_iter` directly —
/// no external arena is needed to keep it alive until write time.
#[test]
fn owned_vec_streams_correctly() {
    let values: Vec<f64> = vec![1.5, 2.5, 3.5];
    let len = values.len();
    let mut b = Builder::new();
    let p = b.add_slab_from_iter(len, values);
    let skeleton = format!(r#"{{"x":{{"$s":{}}}}}"#, p.index());
    let bytes = b.finish(skeleton.as_bytes());
    let parsed = ParsedFile::parse(&bytes).unwrap();
    let back: Vec<f64> = parsed.read::<f64>(p).unwrap();
    assert_eq!(back, vec![1.5, 2.5, 3.5]);
}
