//! Synchronous, NanoAOD-focused ROOT reader.
//!
//! This crate is the first strangler phase for the owned ROOT I/O core.  It
//! implements local TFile/TKey/TTree parsing, ROOT compressed block decoding,
//! scalar and NanoAOD-style leaf-count jagged branch reads, and bounded
//! basket-windowed chunk reads, plus an uncompressed TTree writer for the same
//! scalar and jagged-by-counter subset.

mod decompress;
mod error;
mod parse;
mod root_file;
mod tree;
pub mod write;

pub use error::{Error, Result};
#[cfg(feature = "http")]
pub use root_file::HttpSourceOptions;
pub use root_file::{FileObject, RootFile, Source};
pub use tree::{
    BranchInfo, ChunkedReader, ColumnChunk, ColumnData, ColumnRequest, LeafInfo, Scalar, Tree,
    TreeChunk,
};
