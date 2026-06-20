//! Synchronous, NanoAOD-focused ROOT reader.
//!
//! This crate is the first strangler phase for the owned ROOT I/O core.  It
//! implements local TFile/TKey/TTree parsing, ROOT compressed block decoding,
//! scalar and NanoAOD-style leaf-count jagged branch reads, and bounded
//! basket-windowed chunk reads.  General object streaming and writing are still
//! out of scope for this strangler stage.

mod decompress;
mod error;
mod parse;
mod root_file;
mod tree;

pub use error::{Error, Result};
pub use root_file::{FileObject, RootFile};
pub use tree::{
    BranchInfo, ChunkedReader, ColumnChunk, ColumnData, ColumnRequest, LeafInfo, Scalar, Tree,
    TreeChunk,
};
