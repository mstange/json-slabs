//! End-to-end tests exercising the crate's read + write APIs. The
//! crate has no serde dependency; callers hand-build the root JSON
//! skeleton (or use their own JSON library).

use json_slabs::format::SlabType;
use json_slabs::read::{DecodeError, ParsedFile};
use json_slabs::write::{Builder, JsonBytes, SlabBytes};
use json_slabs::SlabPlaceholder;

/// Hand-build a JSLB with a typed-array slab and a sub-JSON slab,
/// using only `add_slab` (with primitive-slice and `JsonBytes`
/// `AsSlabBytes` impls) and `finish`, then read it back with
/// `read::<T>` / `read_subjson_bytes` / `root_json_bytes`.
#[test]
fn byte_level_roundtrip() {
    let mut b = Builder::new();
    let frame = b.add_slab(&[10i32, 20, 30]);
    let sub = b.add_slab(JsonBytes(br#"{"name":"Firefox","version":28}"#));

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

/// Wrapping a byte slice in [`JsonBytes`] makes [`Builder::add_slab`]
/// emit a JSON slab.
#[test]
fn json_bytes_creates_json_slab() {
    let mut b = Builder::new();
    let p = b.add_slab(JsonBytes(b"[1,2,3]"));
    let skeleton = format!(r#"{{"x":{{"$s":{}}}}}"#, p.index());
    let bytes = b.finish(skeleton.as_bytes());
    let parsed = ParsedFile::parse(&bytes).unwrap();
    let slab = parsed.slab_at(p).unwrap();
    assert_eq!(slab.slab_type, SlabType::Json);
    assert_eq!(slab.data, b"[1,2,3]");
}

/// `SlabPlaceholder(usize)` constructor and `index()` accessor work
/// without any serde at all.
#[test]
fn placeholder_construct_and_inspect() {
    let p = SlabPlaceholder(42);
    assert_eq!(p.index(), 42);
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

/// `Builder::add_slab_bytes` accepts a runtime-typed `SlabBytes` pair —
/// useful when the slab type isn't known at compile time (e.g. when
/// re-emitting bytes parsed from another file).
#[test]
fn add_slab_bytes_with_runtime_type() {
    // Raw little-endian bytes for [-1i16, 0, 1].
    let raw: [u8; 6] = [0xff, 0xff, 0x00, 0x00, 0x01, 0x00];
    let mut b = Builder::new();
    let p = b.add_slab_bytes(SlabBytes {
        slab_type: SlabType::Int16,
        bytes: &raw,
    });
    let skeleton = format!(r#"{{"x":{{"$s":{}}}}}"#, p.index());
    let bytes = b.finish(skeleton.as_bytes());
    let parsed = ParsedFile::parse(&bytes).unwrap();
    let back: Vec<i16> = parsed.read(p).unwrap();
    assert_eq!(back, vec![-1, 0, 1]);
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
        let start = slab.data.as_ptr() as usize - bytes.as_ptr() as usize;
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
