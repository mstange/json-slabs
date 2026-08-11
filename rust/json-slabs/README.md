# json-slabs

[![crates.io](https://img.shields.io/crates/v/json-slabs.svg)](https://crates.io/crates/json-slabs)
[![docs.rs](https://img.shields.io/docsrs/json-slabs)](https://docs.rs/json-slabs)

Read and write JsonSlabs (JSLB) files — a binary container format for JSON-shaped
data with embedded typed-array slabs. The array data is stored out-of-line and at
its natural alignment, so it can be read as a zero-copy view into the file buffer
and its size doesn't affect the parsing cost of the object structure.

This crate contains no JSON parser or writer. For JSON slabs it works with raw
bytes and leaves parsing and placeholder substitution to the consumer.

## Writing

```rust
use json_slabs::Builder;

let mut b = Builder::new();
let numbers = b.add_slab(&[10i32, 20, 30]);
let sub_json = b.add_json_slab(br#"["hello", "world"]"#.to_vec());
// `{:#}` on a placeholder prints the whole `{"$s": N}` reference.
let skeleton = format!(r#"{{"numbers":{numbers:#},"splitOutArray":{sub_json:#}}}"#);
let bytes = b.finish(skeleton.as_bytes());
```

The slice passed to `Builder::add_slab` needs to outlive the builder.
You can use `Builder::add_slab_from_vec(values)` instead if you want the
builder to hold on to the data.

For columns that don't already exist in memory, use
`Builder::add_slab_from_iter(count, iter)`. The iterator is consumed during
`Builder::finish` / `Builder::to_writer` when the file bytes are written.

## Reading

```rust
use json_slabs::{ParsedFile, SlabPlaceholder};

let parsed = ParsedFile::parse(&bytes).unwrap();
// The root JSON slab, as raw JSON bytes:
// br#"{"numbers":{"$s":1},"splitOutArray":{"$s":2}}"#
let root: &[u8] = parsed.root_json_bytes();
// In practice you'd parse that JSON and get the slab index from the
// parsed `{"$s": N}` placeholder.
let numbers: Vec<i32> = parsed.read(SlabPlaceholder(1)).unwrap();
assert_eq!(numbers, vec![10, 20, 30]);
```

See the [API docs](https://docs.rs/json-slabs) for the full API, and
[FORMAT.md](https://github.com/mstange/json-slabs/blob/main/FORMAT.md) for the
on-disk layout.

## License

MIT. See [LICENSE](LICENSE).
