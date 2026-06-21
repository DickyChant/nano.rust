//! Runtime JIT backend for per-event `nano-spec` IR.
//!
//! This crate is intentionally feature-gated. Enable the `jit` feature to use
//! the runtime compiler/loader path:
//!
//! ```text
//! cargo test -p nano-jit --features jit
//! ```
//!
//! The JIT path requires a Rust toolchain at runtime and pays compile latency
//! before the first event can run. It exists for the "arbitrary validated spec at
//! native speed without a manual rebuild" case. The default backends remain:
//! interpret for no-toolchain execution, and AOT codegen for build-time
//! compilation.
//!
//! ABI note: loaded kernels do not receive `&nano_core::Event`. Rust's ABI is
//! not stable across dynamic library boundaries, so the first slice exports a C
//! ABI function that takes only plain muon inputs (`nMuon`, `Muon_pt`, and
//! `Muon_eta` pointers/lengths) and writes a `#[repr(C)]` output row. The loaded
//! crate reconstructs its own internal `Event` and calls the generated Rust
//! producer inside the dylib.

#[cfg(feature = "jit")]
mod muon;

#[cfg(feature = "jit")]
pub use muon::{JitBuildProfile, JitError, JitMuonRow, JitMuonRunner};

#[cfg(not(feature = "jit"))]
pub const JIT_FEATURE_DISABLED: &str =
    "nano-jit runtime compilation is disabled; rebuild with --features jit";
