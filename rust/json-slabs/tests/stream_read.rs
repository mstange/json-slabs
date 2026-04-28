//! Tests for [`RootJsonReader`], the streaming root-slab reader.

use std::io::{self, Cursor, Read};

use json_slabs::stream_read::RootJsonReader;
use json_slabs::write::{Builder, JsonBytes};

fn build_with_root(root_json: &[u8]) -> Vec<u8> {
    // Include a couple of trailing slabs so the file has actual bytes past
    // the root — the reader should still yield exactly root_json.len().
    let mut b = Builder::new();
    let _ = b.add_slab(&[1u32, 2, 3]);
    let _ = b.add_slab(&[0.5f64, 1.5, 2.5]);
    let _ = b.add_slab(JsonBytes(br#"{"nested":true}"#));
    b.finish(root_json)
}

#[test]
fn yields_exactly_the_root_json_bytes() {
    let root = br#"{"meta":{"version":1},"libs":[]}"#;
    let bytes = build_with_root(root);

    let mut reader = RootJsonReader::new(Cursor::new(&bytes)).unwrap();
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, root);
}

#[test]
fn stops_at_root_end_even_with_trailing_bytes() {
    // Concatenate arbitrary junk after the JSLB file; the reader must not
    // read past the root slab's byte_length.
    let root = br#"[1,2,3]"#;
    let mut bytes = build_with_root(root);
    bytes.extend_from_slice(b"GARBAGE-AFTER-FILE");

    let mut reader = RootJsonReader::new(Cursor::new(&bytes)).unwrap();
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, root);

    // A follow-up read still returns EOF.
    let mut buf = [0u8; 4];
    assert_eq!(reader.read(&mut buf).unwrap(), 0);
}

#[test]
fn small_read_buffer_returns_bytes_incrementally() {
    let root = br#"{"a":1,"b":2,"c":3}"#;
    let bytes = build_with_root(root);

    let mut reader = RootJsonReader::new(Cursor::new(&bytes)).unwrap();
    let mut out = Vec::new();
    let mut buf = [0u8; 4];
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    assert_eq!(out, root);
}

#[test]
fn bad_magic_is_invalid_data() {
    let mut bytes = build_with_root(br#"{}"#);
    // Corrupt the first byte of the magic.
    bytes[0] ^= 0xFF;

    let err = RootJsonReader::new(Cursor::new(&bytes)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn truncated_header_is_unexpected_eof() {
    let bytes = build_with_root(br#"{}"#);
    // Cut off partway through the fixed header.
    let truncated = &bytes[..10];

    let err = RootJsonReader::new(Cursor::new(truncated)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

/// Hand-craft a two-slab JSLB whose root JSON slab is at index 1, forcing
/// [`RootJsonReader`] to actually skip past a preceding typed slab. This
/// codepath isn't reachable via `Builder`, which always emits root at
/// index 0.
fn build_with_root_after_typed_slab(root_json: &[u8]) -> Vec<u8> {
    // Layout:
    //   [ 0..20)  header
    //   [20..44)  slab table (2 * 12 bytes)
    //   [44..52)  slab 0: i32 (8 bytes)
    //   [52..  )  slab 1: root JSON
    let typed: [i32; 2] = [0x1111_1111, 0x2222_2222];
    let typed_bytes: &[u8] = zerocopy_bytes(&typed);
    let table_end: u32 = 44;
    let typed_len: u32 = typed_bytes.len() as u32;
    let root_offset: u32 = table_end + typed_len;
    let root_len: u32 = root_json.len() as u32;

    let mut out = Vec::new();
    // Fixed header.
    out.extend_from_slice(&[0xDC, 0xDF, 0x4A, 0x53, 0x4C, 0x42, 0x01, 0x00]); // MAGIC
    out.extend_from_slice(&1u32.to_le_bytes()); // VERSION
    out.extend_from_slice(&2u32.to_le_bytes()); // slab_count
    out.extend_from_slice(&1u32.to_le_bytes()); // root_json_slab_index
                                                // Slab table.
    out.extend_from_slice(&0x04u32.to_le_bytes()); // slab 0 type: i32
    out.extend_from_slice(&table_end.to_le_bytes());
    out.extend_from_slice(&typed_len.to_le_bytes());
    out.extend_from_slice(&0x0Au32.to_le_bytes()); // slab 1 type: JSON
    out.extend_from_slice(&root_offset.to_le_bytes());
    out.extend_from_slice(&root_len.to_le_bytes());
    // Slab data.
    out.extend_from_slice(typed_bytes);
    out.extend_from_slice(root_json);
    out
}

fn zerocopy_bytes<T>(slice: &[T]) -> &[u8] {
    // Test-only: reinterpret a slice as bytes. Safe here because the
    // caller passes plain integer arrays with no padding.
    unsafe { std::slice::from_raw_parts(slice.as_ptr() as *const u8, std::mem::size_of_val(slice)) }
}

#[test]
fn root_at_non_zero_index_skips_preceding_slab() {
    let root = br#"{"root":"after typed"}"#;
    let bytes = build_with_root_after_typed_slab(root);

    let mut reader = RootJsonReader::new(Cursor::new(&bytes)).unwrap();
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, root);
}

#[test]
fn truncated_before_root_slab_is_unexpected_eof() {
    let bytes = build_with_root_after_typed_slab(br#"{"x":1}"#);
    // Header (20) + slab table (24) = 44. Cut mid-way through the
    // preceding typed slab so the skip-to-root phase runs out of bytes.
    let truncated = &bytes[..47];

    let err = RootJsonReader::new(Cursor::new(truncated)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn empty_root_slab_yields_eof_immediately() {
    let bytes = build_with_root(b"");

    let mut reader = RootJsonReader::new(Cursor::new(&bytes)).unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(reader.read(&mut buf).unwrap(), 0);
}
