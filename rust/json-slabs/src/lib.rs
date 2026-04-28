//! JsonSlabs (JSLB) — a binary container for JSON-shaped data with embedded
//! typed-array slabs. See [`FORMAT.md`][format] in the repo root for the
//! on-disk layout.
//!
//! [format]: https://github.com/mstange/json-slabs/blob/main/FORMAT.md
//!
//! # Writing
//!
//! ```
//! use json_slabs::write::{Builder, JsonBytes};
//!
//! let mut b = Builder::new();
//! let frame = b.add_slab(&[10i32, 20, 30]);
//! let sub = b.add_slab(JsonBytes(br#"{"name":"Firefox"}"#));
//! let skeleton = format!(
//!     r#"{{"frame":{{"$s":{}}},"meta":{{"$s":{}}}}}"#,
//!     frame.index(),
//!     sub.index(),
//! );
//! let bytes = b.finish(skeleton.as_bytes());
//! # assert!(bytes.starts_with(&json_slabs::format::MAGIC));
//! ```
//!
//! # Reading
//!
//! ```
//! use json_slabs::read::ParsedFile;
//! use json_slabs::SlabPlaceholder;
//!
//! # let mut b = json_slabs::write::Builder::new();
//! # let frame = b.add_slab(&[10i32, 20, 30]);
//! # let skeleton = format!(r#"{{"frame":{{"$s":{}}}}}"#, frame.index());
//! # let bytes = b.finish(skeleton.as_bytes());
//! let parsed = ParsedFile::parse(&bytes).unwrap();
//! // The root skeleton is just bytes — decode it however you like:
//! let _root: &[u8] = parsed.root_json_bytes();
//! // Read a typed slab back. `SlabPlaceholder(1)` here matches `frame`
//! // above (index 0 is reserved for the root JSON); in practice you'd
//! // recover the placeholder from the skeleton JSON.
//! let frame_back: Vec<i32> = parsed.read(SlabPlaceholder(1)).unwrap();
//! assert_eq!(frame_back, vec![10, 20, 30]);
//! ```
//!

pub mod format;
pub mod read;
pub mod stream_read;
pub mod write;

mod placeholder;

pub use placeholder::SlabPlaceholder;
