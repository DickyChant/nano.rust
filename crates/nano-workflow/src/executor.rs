use nano_core::BranchSchema;
use rayon::prelude::*;

use crate::artifacts::{MergedOutput, PartialOutput};
use crate::error::{Result, WorkflowError};
use crate::planner::{Kernel, MapDone, MapNode, ReduceDone, ReduceNode, SinkNode, WorkflowPlan};
use crate::provenance::{
    manifest_matches, map_key, read_branch_signature, reduce_key, sink_key, write_manifest,
    Manifest,
};
use crate::sink::write_muon_skim;
use crate::tasks::{
    read_merged_output, read_partial_output, run_chunk_with_kernel, write_merged_output,
    write_partial_output,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Serial,
    Parallel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RunStats {
    pub executed: usize,
    pub skipped: usize,
}

impl RunStats {
    fn record(&mut self, skipped: bool) {
        if skipped {
            self.skipped += 1;
        } else {
            self.executed += 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionReport {
    pub mode: ExecutionMode,
    pub maps: RunStats,
    pub reduce: RunStats,
    pub sink: RunStats,
    pub merged: MergedOutput,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedExecutionReport {
    pub serial: ExecutionReport,
    pub parallel: ExecutionReport,
    pub merged: MergedOutput,
}

#[derive(Debug, Default)]
pub struct Executor;

impl Executor {
    pub fn new() -> Self {
        Self
    }

    pub fn run(&self, plan: &WorkflowPlan, mode: ExecutionMode) -> Result<ExecutionReport> {
        match mode {
            ExecutionMode::Serial => self.run_serial(plan),
            ExecutionMode::Parallel => self.run_parallel(plan),
        }
    }

    pub fn run_serial(&self, plan: &WorkflowPlan) -> Result<ExecutionReport> {
        let mut map_stats = RunStats::default();
        let mut map_outputs = Vec::with_capacity(plan.maps.len());
        for map in &plan.maps {
            let (done, skipped) = run_map(map, &plan.schema, plan.chunk_size, &plan.kernel)?;
            map_stats.record(skipped);
            map_outputs.push(done);
        }
        let (reduce, reduce_skipped) = run_reduce(&plan.reduce, map_outputs, &plan.schema)?;
        let (merged, sink_skipped) = run_sink(&plan.sink, &reduce, &plan.schema)?;
        Ok(ExecutionReport {
            mode: ExecutionMode::Serial,
            maps: map_stats,
            reduce: RunStats {
                executed: usize::from(!reduce_skipped),
                skipped: usize::from(reduce_skipped),
            },
            sink: RunStats {
                executed: usize::from(!sink_skipped),
                skipped: usize::from(sink_skipped),
            },
            merged,
        })
    }

    pub fn run_parallel(&self, plan: &WorkflowPlan) -> Result<ExecutionReport> {
        let map_results = plan
            .maps
            .par_iter()
            .map(|map| run_map(map, &plan.schema, plan.chunk_size, &plan.kernel))
            .collect::<Result<Vec<_>>>()?;

        let mut map_stats = RunStats::default();
        let map_outputs = map_results
            .into_iter()
            .map(|(done, skipped)| {
                map_stats.record(skipped);
                done
            })
            .collect::<Vec<_>>();

        let (reduce, reduce_skipped) =
            run_reduce_parallel(&plan.reduce, map_outputs, &plan.schema)?;
        let (merged, sink_skipped) = run_sink(&plan.sink, &reduce, &plan.schema)?;
        Ok(ExecutionReport {
            mode: ExecutionMode::Parallel,
            maps: map_stats,
            reduce: RunStats {
                executed: usize::from(!reduce_skipped),
                skipped: usize::from(reduce_skipped),
            },
            sink: RunStats {
                executed: usize::from(!sink_skipped),
                skipped: usize::from(sink_skipped),
            },
            merged,
        })
    }

    pub fn run_verified(
        &self,
        serial_plan: &WorkflowPlan,
        parallel_plan: &WorkflowPlan,
    ) -> Result<VerifiedExecutionReport> {
        let serial = self.run_serial(serial_plan)?;
        let parallel = self.run_parallel(parallel_plan)?;
        if serial.merged != parallel.merged {
            return Err(WorkflowError::Assertion(
                "serial and parallel workflow outputs differ".to_string(),
            ));
        }
        Ok(VerifiedExecutionReport {
            merged: serial.merged.clone(),
            serial,
            parallel,
        })
    }
}

fn run_map(
    node: &MapNode,
    schema: &BranchSchema,
    chunk_size: usize,
    kernel: &Kernel,
) -> Result<(MapDone, bool)> {
    let key = map_key(&node.chunk, schema)?;
    if manifest_matches(&node.manifest_path, &key)? && node.artifact_path.exists() {
        let output = read_partial_output(&node.artifact_path)?;
        return Ok((
            MapDone {
                node_id: node.id,
                key,
                output,
            },
            true,
        ));
    }

    let output = run_chunk_with_kernel(&node.chunk, schema, chunk_size, kernel)?;
    write_partial_output(&node.artifact_path, &output)?;
    write_manifest(
        &node.manifest_path,
        &Manifest::new(
            key.clone(),
            "map",
            read_branch_signature(schema),
            vec![format!(
                "{}:{}-{}",
                node.chunk.source, node.chunk.entry_range.start, node.chunk.entry_range.end
            )],
        ),
    )?;

    Ok((
        MapDone {
            node_id: node.id,
            key,
            output,
        },
        false,
    ))
}

fn run_reduce(
    node: &ReduceNode,
    maps: Vec<MapDone>,
    schema: &BranchSchema,
) -> Result<(ReduceDone, bool)> {
    let keys = maps.iter().map(|map| map.key.clone()).collect::<Vec<_>>();
    let key = reduce_key(&keys, schema);
    if manifest_matches(&node.manifest_path, &key)? && node.artifact_path.exists() {
        let output = read_merged_output(&node.artifact_path)?;
        return Ok((
            ReduceDone {
                node_id: node.id,
                key,
                output,
            },
            true,
        ));
    }

    let output = maps
        .into_iter()
        .map(|map| map.output)
        .fold(PartialOutput::default(), PartialOutput::merge)
        .into();
    write_merged_output(&node.artifact_path, &output)?;
    write_manifest(
        &node.manifest_path,
        &Manifest::new(key.clone(), "reduce", read_branch_signature(schema), keys),
    )?;
    Ok((
        ReduceDone {
            node_id: node.id,
            key,
            output,
        },
        false,
    ))
}

fn run_reduce_parallel(
    node: &ReduceNode,
    maps: Vec<MapDone>,
    schema: &BranchSchema,
) -> Result<(ReduceDone, bool)> {
    let keys = maps.iter().map(|map| map.key.clone()).collect::<Vec<_>>();
    let key = reduce_key(&keys, schema);
    if manifest_matches(&node.manifest_path, &key)? && node.artifact_path.exists() {
        let output = read_merged_output(&node.artifact_path)?;
        return Ok((
            ReduceDone {
                node_id: node.id,
                key,
                output,
            },
            true,
        ));
    }

    let output = maps
        .into_par_iter()
        .map(|map| map.output)
        .reduce(PartialOutput::default, PartialOutput::merge)
        .into();
    write_merged_output(&node.artifact_path, &output)?;
    write_manifest(
        &node.manifest_path,
        &Manifest::new(key.clone(), "reduce", read_branch_signature(schema), keys),
    )?;
    Ok((
        ReduceDone {
            node_id: node.id,
            key,
            output,
        },
        false,
    ))
}

fn run_sink(
    node: &SinkNode,
    reduce: &ReduceDone,
    schema: &BranchSchema,
) -> Result<(MergedOutput, bool)> {
    let key = sink_key(&reduce.key, &node.output_path);
    if manifest_matches(&node.manifest_path, &key)? && node.output_path.exists() {
        return Ok((reduce.output.clone(), true));
    }

    write_muon_skim(&node.output_path, &reduce.output)?;
    write_manifest(
        &node.manifest_path,
        &Manifest::new(
            key,
            "sink",
            read_branch_signature(schema),
            vec![reduce.key.clone()],
        ),
    )?;
    Ok((reduce.output.clone(), false))
}
