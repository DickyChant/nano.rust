//! Synchronous, NanoAOD-focused ROOT reader.
//!
//! This crate is the first strangler phase for the owned ROOT I/O core.  It
//! deliberately implements only the read foundation: local TFile/TKey/TTree
//! parsing, ROOT compressed block decoding, and scalar fixed-size branch reads.
//! Jagged branches, general object streaming, and writing are intentionally out
//! of scope for this phase.

mod decompress;
mod error;
mod parse;
mod root_file;
mod tree;

pub use error::{Error, Result};
pub use root_file::{FileObject, RootFile};
pub use tree::{BranchInfo, LeafInfo, Scalar, Tree};
