//! [`SlabPlaceholder`]: a thin newtype around a slab index. Its
//! [`fmt::Display`][std::fmt::Display] impl prints the bare integer, so
//! it interpolates cleanly into a hand-built JSON skeleton, e.g.
//! `format!("{{\"$s\":{}}}", p)`.

use std::fmt;

/// A reference to a slab by its index in the file's slab table.
///
/// Returned from [`crate::write::Builder::add_slab`] when building a file,
/// and accepted by [`crate::read::ParsedFile::read`] and friends when
/// reading one back. The inner `usize` is the raw slab index; index 0 is
/// reserved for the root JSON skeleton, so placeholders returned by the
/// builder start at 1.
///
/// The [`fmt::Display`] impl prints the bare integer, so a placeholder
/// interpolates cleanly into a hand-built JSON skeleton, e.g.
/// `format!(r#"{{"$s":{}}}"#, p)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlabPlaceholder(pub usize);

impl SlabPlaceholder {
    /// The underlying slab table index.
    pub fn index(self) -> usize {
        self.0
    }
}

impl fmt::Display for SlabPlaceholder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
