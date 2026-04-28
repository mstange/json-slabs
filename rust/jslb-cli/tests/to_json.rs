//! End-to-end tests for `jslb to-json`. We build small JSLB files
//! in-memory via `json_slabs::Builder`, run the compiled CLI binary on
//! them, and parse the output back through `serde_json` for comparison
//! — that way differences in number formatting or whitespace don't
//! cause spurious failures.

use json_slabs::write::{Builder, JsonBytes};
use serde_json::{json, Value};
use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

fn jslb_bin() -> &'static str {
    env!("CARGO_BIN_EXE_jslb")
}

fn write_jslb(bytes: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(bytes).expect("write");
    f.flush().expect("flush");
    f
}

fn run_to_json(jslb_path: &std::path::Path) -> Value {
    let out = Command::new(jslb_bin())
        .arg("to-json")
        .arg(jslb_path)
        .output()
        .expect("run jslb");
    assert!(
        out.status.success(),
        "jslb to-json failed: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    serde_json::from_slice(&out.stdout).expect("parse json output")
}

#[test]
fn covers_all_typed_arrays_and_subjson() {
    let mut b = Builder::new();
    let i8s = b.add_slab(&[-1i8, 0, 1]);
    let u8s = b.add_slab(&[0u8, 255]);
    let i16s = b.add_slab(&[i16::MIN, 0, i16::MAX]);
    let u16s = b.add_slab(&[0u16, u16::MAX]);
    let i32s = b.add_slab(&[i32::MIN, 0, i32::MAX]);
    let u32s = b.add_slab(&[0u32, u32::MAX]);
    let f32s = b.add_slab(&[0.5f32, -1.25]);
    let f64s = b.add_slab(&[1.0f64, 0.5, -0.25]);
    let i64s = b.add_slab(&[-1i64, 0, 1]);
    let u64s = b.add_slab(&[0u64, 1, 2]);

    // Sub-JSON slab whose own skeleton contains a placeholder.
    let nested_u8 = b.add_slab(&[7u8, 8, 9]);
    let sub = format!(r#"{{"items":{{"$s":{nested_u8}}},"label":"nested"}}"#);
    let sub_idx = b.add_slab(JsonBytes(sub.as_bytes()));

    let skeleton = format!(
        r#"{{
            "i8":  {{"$s":{i8s}}},
            "u8":  {{"$s":{u8s}}},
            "i16": {{"$s":{i16s}}},
            "u16": {{"$s":{u16s}}},
            "i32": {{"$s":{i32s}}},
            "u32": {{"$s":{u32s}}},
            "f32": {{"$s":{f32s}}},
            "f64": {{"$s":{f64s}}},
            "i64": {{"$s":{i64s}}},
            "u64": {{"$s":{u64s}}},
            "sub": {{"$s":{sub_idx}}}
        }}"#
    );
    let bytes = b.finish(skeleton.as_bytes());
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());

    assert_eq!(got["i8"], json!([-1, 0, 1]));
    assert_eq!(got["u8"], json!([0, 255]));
    assert_eq!(got["i16"], json!([i16::MIN, 0, i16::MAX]));
    assert_eq!(got["u16"], json!([0, u16::MAX]));
    assert_eq!(got["i32"], json!([i32::MIN, 0, i32::MAX]));
    assert_eq!(got["u32"], json!([0u32, u32::MAX]));
    assert_eq!(got["i64"], json!([-1, 0, 1]));
    assert_eq!(got["u64"], json!([0u64, 1, 2]));

    // Floats: compare by length + numeric closeness.
    let f32_arr = got["f32"].as_array().unwrap();
    assert_eq!(f32_arr.len(), 2);
    assert!((f32_arr[0].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert!((f32_arr[1].as_f64().unwrap() - -1.25).abs() < 1e-6);
    let f64_arr = got["f64"].as_array().unwrap();
    assert_eq!(f64_arr.len(), 3);
    assert_eq!(f64_arr[1].as_f64().unwrap(), 0.5);

    // Recursion through sub-JSON: its own placeholder must also be expanded.
    assert_eq!(got["sub"]["items"], json!([7, 8, 9]));
    assert_eq!(got["sub"]["label"], json!("nested"));
}

/// `{"$s": N, "tag": "real"}` — has the magic key first but isn't a
/// placeholder because there's a second key. Must round-trip as a
/// regular object.
#[test]
fn dollar_s_plus_other_key_is_regular_object() {
    let mut b = Builder::new();
    let payload = b.add_slab(&[1u8, 2, 3]);
    // The skeleton: a regular object whose first key happens to be `$s`,
    // and whose value at index `payload` would *be* a valid slab — but
    // because there's a second key it's a real object and `$s` is a
    // plain integer member.
    let skel = format!(r#"{{"$s":{payload},"tag":"real","other":{{"$s":{payload}}}}}"#);
    let bytes = b.finish(skel.as_bytes());
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());

    // First object: literal `$s` member preserved, second member preserved,
    // *and* the nested single-key object IS a placeholder, so it expands.
    assert_eq!(got["$s"], json!(payload.index()));
    assert_eq!(got["tag"], json!("real"));
    assert_eq!(got["other"], json!([1, 2, 3]));
}

/// `{"$s": "hello"}` — single key but value is not a non-negative
/// integer. Must round-trip as a regular object.
#[test]
fn dollar_s_with_non_integer_is_regular_object() {
    let b: Builder = Builder::new();
    let bytes = b.finish(br#"{"$s":"hello"}"#);
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());
    assert_eq!(got, json!({"$s": "hello"}));
}

/// `{"$s": -1}` — single key but negative. Must round-trip as a
/// regular object.
#[test]
fn dollar_s_with_negative_is_regular_object() {
    let b: Builder = Builder::new();
    let bytes = b.finish(br#"{"$s":-1}"#);
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());
    assert_eq!(got, json!({"$s": -1}));
}

/// Placeholder nested inside a normal object: must be resolved.
#[test]
fn placeholder_nested_in_real_object() {
    let mut b = Builder::new();
    let payload = b.add_slab(&[10i32, 20, 30]);
    let skel = format!(r#"{{"foo":{{"$s":{payload}}}}}"#);
    let bytes = b.finish(skel.as_bytes());
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());
    assert_eq!(got, json!({ "foo": [10, 20, 30] }));
}

/// Exercise the drain path: a typed-array slab large enough that the
/// streaming writer must flush its String buffer mid-array.
#[test]
fn large_typed_array_streams_correctly() {
    let n = 100_000usize;
    let data: Vec<i32> = (0..n as i32).collect();
    let mut b = Builder::new();
    let idx = b.add_slab(&data[..]);
    let skel = format!(r#"{{"big":{{"$s":{idx}}}}}"#);
    let bytes = b.finish(skel.as_bytes());
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());
    let arr = got["big"].as_array().unwrap();
    assert_eq!(arr.len(), n);
    assert_eq!(arr[0].as_i64().unwrap(), 0);
    assert_eq!(arr[n - 1].as_i64().unwrap(), (n - 1) as i64);
    assert_eq!(arr[n / 2].as_i64().unwrap(), (n / 2) as i64);
}

/// Skeleton with a placeholder followed by content that forces the
/// parser's input buffer to refill. Regression test for a bug where
/// expanding a typed-array placeholder moved the underlying file
/// offset out from under the skeleton parser.
#[test]
fn placeholder_then_long_string_does_not_corrupt_parser() {
    let mut b = Builder::new();
    let payload = b.add_slab(&[1i32, 2, 3]);
    let big = "x".repeat(100 * 1024);
    let skel = format!(r#"{{"big":{{"$s":{payload}}},"note":"{big}"}}"#);
    let bytes = b.finish(skel.as_bytes());
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());
    assert_eq!(got["big"], json!([1, 2, 3]));
    assert_eq!(got["note"].as_str().unwrap().len(), 100 * 1024);
    assert!(got["note"].as_str().unwrap().chars().all(|c| c == 'x'));
}

/// Empty object and empty array as values must round-trip.
#[test]
fn empty_containers() {
    let b: Builder = Builder::new();
    let bytes = b.finish(br#"{"obj":{},"arr":[],"both":[{},[]]}"#);
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());
    assert_eq!(got, json!({"obj": {}, "arr": [], "both": [{}, []]}));
}

/// Booleans and null pass through.
#[test]
fn booleans_and_null() {
    let b: Builder = Builder::new();
    let bytes = b.finish(br#"{"a":true,"b":false,"c":null,"d":[true,null,false]}"#);
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());
    assert_eq!(
        got,
        json!({"a": true, "b": false, "c": null, "d": [true, null, false]})
    );
}

/// Strings with escapes and unicode pass through.
#[test]
fn strings_with_escapes() {
    let b: Builder = Builder::new();
    let bytes = b.finish(r#"{"k":"hello\nworld","u":"snow☃man","q":"a\"b"}"#.as_bytes());
    let f = write_jslb(&bytes);
    let got = run_to_json(f.path());
    assert_eq!(got["k"], json!("hello\nworld"));
    assert_eq!(got["u"], json!("snow☃man"));
    assert_eq!(got["q"], json!("a\"b"));
}

/// Out-of-range slab index is reported as an error, not a panic.
#[test]
fn out_of_range_slab_index_errors() {
    let b: Builder = Builder::new();
    let bytes = b.finish(br#"{"x":{"$s":99}}"#);
    let f = write_jslb(&bytes);
    let out = Command::new(jslb_bin())
        .arg("to-json")
        .arg(f.path())
        .output()
        .expect("run jslb");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("slab index 99 out of range"),
        "stderr: {err:?}"
    );
}

/// File output mode produces the same JSON as stdout mode.
#[test]
fn file_output_matches_stdout() {
    let mut b = Builder::new();
    let nums = b.add_slab(&[100u32, 200, 300]);
    let skel = format!(r#"{{"v":{{"$s":{nums}}}}}"#);
    let bytes = b.finish(skel.as_bytes());
    let f = write_jslb(&bytes);

    let stdout_value = run_to_json(f.path());

    let out_file = NamedTempFile::new().unwrap();
    let status = Command::new(jslb_bin())
        .arg("to-json")
        .arg(f.path())
        .arg(out_file.path())
        .status()
        .expect("run jslb to file");
    assert!(status.success());
    let file_text = std::fs::read_to_string(out_file.path()).unwrap();
    let file_value: Value = serde_json::from_str(&file_text).expect("parse file output");
    assert_eq!(stdout_value, file_value);
    assert_eq!(file_value, json!({ "v": [100, 200, 300] }));
}
