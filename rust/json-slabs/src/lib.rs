//! A crate for reading and writing JsonSlabs (JSLB) files — a binary
//! container format for JSON-shaped data with embedded typed-array slabs.
//!
//! JSON slabs use placeholder objects of the form `{ "$s": <slab-index> }`
//! to refer to data that's stored in other slabs.
//!
//! This crate does not include any JSON parsing or JSON writing code; for JSON
//! slabs, this crate only works with raw bytes and leaves the parsing /
//! placeholder substitution to the consumer.
//!
//! See [`FORMAT.md`][format] in the repo root for the on-disk layout.
//!
//! Use [`Builder`] for writing and [`ParsedFile`] for reading.
//!
//! [format]: https://github.com/mstange/json-slabs/blob/main/FORMAT.md
//!
//! # Writing
//!
//! ```
//! use json_slabs::Builder;
//!
//! let mut b = Builder::new();
//! let numbers = b.add_slab(&[10i32, 20, 30]);
//! let sub_json = b.add_json_slab(br#"["hello", "world"]"#.to_vec());
//! // `{:#}` on a placeholder prints the whole `{"$s": N}` reference.
//! let skeleton = format!(r#"{{"numbers":{numbers:#},"splitOutArray":{sub_json:#}}}"#);
//! let bytes = b.finish(skeleton.as_bytes());
//! # assert!(bytes.starts_with(&json_slabs::MAGIC));
//! ```
//!
//! # Reading
//!
//! ```
//! use json_slabs::{ParsedFile, SlabPlaceholder};
//!
//! # let mut b = json_slabs::Builder::new();
//! # let numbers = b.add_slab(&[10i32, 20, 30]);
//! # let skeleton = format!(r#"{{"numbers":{numbers:#}}}"#);
//! # let bytes = b.finish(skeleton.as_bytes());
//! let parsed = ParsedFile::parse(&bytes).unwrap();
//! // The root JSON slab (as raw JSON bytes).
//! let _root: &[u8] = parsed.root_json_bytes();
//! // Read a typed slab back. `SlabPlaceholder(1)` here matches the
//! // `{"$s": 1}` placeholder found in the root JSON; in practice
//! // you'd parse the JSON and get the slab index from the parsed
//! // placeholder.
//! let numbers_back: Vec<i32> = parsed.read(SlabPlaceholder(1)).unwrap();
//! assert_eq!(numbers_back, vec![10, 20, 30]);
//! ```
//!

mod read;
mod types;
mod write;

pub use read::*;
pub use types::*;
pub use write::*;
