//! Rust-native typed workflow orchestration for nano.rust.
//!
//! This first slice implements the ratified local DAG shape: chunked sources,
//! independent map nodes, an associative reduce, provenance manifests, and a
//! ROOT skim sink for the muon producer row.

pub mod artifacts;
pub mod error;
pub mod executor;
pub mod planner;
pub mod provenance;
pub mod sink;

pub use artifacts::{ChunkSpec, Cutflow, EntryRange, Histogram1D, MergedOutput, PartialOutput};
pub use error::{Result, WorkflowError};
pub use executor::{ExecutionMode, ExecutionReport, Executor, RunStats, VerifiedExecutionReport};
pub use planner::{
    plan_muon_workflow, plan_workflow, MapDone, MapNode, ReduceDone, ReduceNode, SinkNode,
    SourceNode, WorkflowPlan,
};
pub use provenance::{Manifest, CODE_SPEC_VERSION};
pub use sink::write_muon_skim;
