//! Rust-native typed workflow orchestration for nano.rust.
//!
//! The workflow DAG is a portable IR. It can run through the in-process executor
//! or be exported as JSON for boundary adapters that submit standalone task
//! atoms to external schedulers.

pub mod artifacts;
pub mod error;
pub mod executor;
pub mod planner;
pub mod portable;
pub mod provenance;
pub mod sink;
pub mod tasks;

pub use artifacts::{ChunkSpec, Cutflow, EntryRange, Histogram1D, MergedOutput, PartialOutput};
pub use error::{Result, WorkflowError};
pub use executor::{ExecutionMode, ExecutionReport, Executor, RunStats, VerifiedExecutionReport};
pub use planner::{
    plan_muon_workflow, plan_workflow, plan_workflow_with_kernel_id, MapDone, MapNode, ReduceDone,
    ReduceNode, SinkNode, SourceNode, WorkflowNodeKind, WorkflowNodeSummary, WorkflowPlan,
};
pub use portable::{
    export_portable_graph, import_portable_graph, import_portable_graph_with_registry,
    PortableBranchSpec, PortableBranchType, PortableEdge, PortableGraph, PortableNode,
    PortableNodeKind, PORTABLE_SCHEMA_VERSION,
};
pub use provenance::{Manifest, CODE_SPEC_VERSION};
pub use sink::write_muon_skim;
pub use tasks::{
    merge_partial_files, merge_partials, muon_schema, read_merged_output, read_partial_output,
    run_chunk, run_chunk_to_path, run_chunk_with_kernel, write_merged_output, write_partial_output,
    KernelBinding, KernelRegistry, RunChunkRequest,
};
