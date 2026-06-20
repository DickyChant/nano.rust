use std::collections::HashMap;
use std::path::PathBuf;

use nano_core::{BranchSchema, BranchSpec, BranchType};
use serde::{Deserialize, Serialize};

use crate::artifacts::EntryRange;
use crate::error::{Result, WorkflowError};
use crate::planner::{MapNode, ReduceNode, SinkNode, SourceNode, WorkflowPlan};
use crate::tasks::KernelRegistry;

pub const PORTABLE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableGraph {
    pub schema_version: u32,
    pub default_kernel_id: String,
    pub read_schema: Vec<PortableBranchSpec>,
    pub chunk_size: usize,
    pub nodes: Vec<PortableNode>,
    pub edges: Vec<PortableEdge>,
}

impl PortableGraph {
    pub fn from_plan(plan: &WorkflowPlan) -> Self {
        export_portable_graph(plan)
    }

    pub fn into_plan(self) -> Result<WorkflowPlan> {
        import_portable_graph(&self)
    }

    pub fn to_plan(&self) -> Result<WorkflowPlan> {
        import_portable_graph(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableNode {
    pub id: String,
    pub kind: PortableNodeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_range: Option<EntryRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableNodeKind {
    Source,
    Map,
    Reduce,
    Sink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableBranchSpec {
    pub name: String,
    pub branch_type: PortableBranchType,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableBranchType {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    VecBool,
    VecI8,
    VecU8,
    VecI16,
    VecU16,
    VecI32,
    VecU32,
    VecI64,
    VecU64,
    VecF32,
}

pub fn export_portable_graph(plan: &WorkflowPlan) -> PortableGraph {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut source_ids = HashMap::new();

    for source in &plan.sources {
        let id = source_id(source.id);
        let source_path = source.path.display().to_string();
        source_ids
            .entry(source_path.clone())
            .or_insert_with(|| id.clone());
        nodes.push(PortableNode {
            id,
            kind: PortableNodeKind::Source,
            source: Some(source_path),
            entry_range: None,
            kernel_id: None,
            output_path: None,
            manifest_path: None,
        });
    }

    for map in &plan.maps {
        let id = map_id(map.id);
        if let Some(source_id) = source_ids.get(&map.chunk.source) {
            edges.push(PortableEdge {
                from: source_id.clone(),
                to: id.clone(),
            });
        }
        edges.push(PortableEdge {
            from: id.clone(),
            to: reduce_id(plan.reduce.id),
        });
        nodes.push(PortableNode {
            id,
            kind: PortableNodeKind::Map,
            source: Some(map.chunk.source.clone()),
            entry_range: Some(map.chunk.entry_range.clone()),
            kernel_id: Some(plan.kernel_id.clone()),
            output_path: Some(map.artifact_path.display().to_string()),
            manifest_path: Some(map.manifest_path.display().to_string()),
        });
    }

    nodes.push(PortableNode {
        id: reduce_id(plan.reduce.id),
        kind: PortableNodeKind::Reduce,
        source: None,
        entry_range: None,
        kernel_id: None,
        output_path: Some(plan.reduce.artifact_path.display().to_string()),
        manifest_path: Some(plan.reduce.manifest_path.display().to_string()),
    });
    edges.push(PortableEdge {
        from: reduce_id(plan.reduce.id),
        to: "sink-0".to_string(),
    });
    nodes.push(PortableNode {
        id: "sink-0".to_string(),
        kind: PortableNodeKind::Sink,
        source: None,
        entry_range: None,
        kernel_id: None,
        output_path: Some(plan.sink.output_path.display().to_string()),
        manifest_path: Some(plan.sink.manifest_path.display().to_string()),
    });

    PortableGraph {
        schema_version: PORTABLE_SCHEMA_VERSION,
        default_kernel_id: plan.kernel_id.clone(),
        read_schema: plan
            .schema
            .specs()
            .iter()
            .map(PortableBranchSpec::from)
            .collect(),
        chunk_size: plan.chunk_size,
        nodes,
        edges,
    }
}

pub fn import_portable_graph(graph: &PortableGraph) -> Result<WorkflowPlan> {
    import_portable_graph_with_registry(graph, &KernelRegistry::with_muon())
}

pub fn import_portable_graph_with_registry(
    graph: &PortableGraph,
    registry: &KernelRegistry,
) -> Result<WorkflowPlan> {
    if graph.schema_version != PORTABLE_SCHEMA_VERSION {
        return Err(WorkflowError::InvalidGraph(format!(
            "unsupported PortableGraph schema_version {}; expected {}",
            graph.schema_version, PORTABLE_SCHEMA_VERSION
        )));
    }

    let schema = BranchSchema::new(
        graph
            .read_schema
            .iter()
            .map(PortableBranchSpec::to_branch_spec),
    )?;

    let mut sources = Vec::new();
    let mut maps = Vec::new();
    let mut reduce = None;
    let mut sink = None;
    let mut kernel_id = graph.default_kernel_id.clone();

    for node in &graph.nodes {
        match node.kind {
            PortableNodeKind::Source => {
                let source = required(&node.source, &node.id, "source")?;
                sources.push(SourceNode {
                    id: parse_prefixed_id(&node.id, "source-").unwrap_or(sources.len()),
                    path: PathBuf::from(source),
                });
            }
            PortableNodeKind::Map => {
                let node_kernel_id = required(&node.kernel_id, &node.id, "kernel_id")?;
                if maps.is_empty() {
                    kernel_id = node_kernel_id.to_string();
                } else if kernel_id != *node_kernel_id {
                    return Err(WorkflowError::InvalidGraph(
                        "WorkflowPlan import currently expects one kernel id".to_string(),
                    ));
                }
                maps.push(MapNode {
                    id: parse_prefixed_id(&node.id, "map-").unwrap_or(maps.len()),
                    chunk: crate::artifacts::ChunkSpec {
                        source: required(&node.source, &node.id, "source")?.to_string(),
                        entry_range: required(&node.entry_range, &node.id, "entry_range")?.clone(),
                    },
                    artifact_path: PathBuf::from(required(
                        &node.output_path,
                        &node.id,
                        "output_path",
                    )?),
                    manifest_path: PathBuf::from(required(
                        &node.manifest_path,
                        &node.id,
                        "manifest_path",
                    )?),
                });
            }
            PortableNodeKind::Reduce => {
                reduce = Some(ReduceNode {
                    id: parse_prefixed_id(&node.id, "reduce-").unwrap_or(0),
                    artifact_path: PathBuf::from(required(
                        &node.output_path,
                        &node.id,
                        "output_path",
                    )?),
                    manifest_path: PathBuf::from(required(
                        &node.manifest_path,
                        &node.id,
                        "manifest_path",
                    )?),
                });
            }
            PortableNodeKind::Sink => {
                sink = Some(SinkNode {
                    output_path: PathBuf::from(required(
                        &node.output_path,
                        &node.id,
                        "output_path",
                    )?),
                    manifest_path: PathBuf::from(required(
                        &node.manifest_path,
                        &node.id,
                        "manifest_path",
                    )?),
                });
            }
        }
    }

    let binding = registry.get(&kernel_id)?;
    Ok(WorkflowPlan {
        sources,
        maps,
        reduce: reduce.ok_or_else(|| {
            WorkflowError::InvalidGraph("PortableGraph is missing a reduce node".to_string())
        })?,
        sink: sink.ok_or_else(|| {
            WorkflowError::InvalidGraph("PortableGraph is missing a sink node".to_string())
        })?,
        schema,
        chunk_size: graph.chunk_size.max(1),
        kernel: binding.kernel.clone(),
        kernel_id,
    })
}

impl PortableBranchSpec {
    fn to_branch_spec(&self) -> BranchSpec {
        let mut spec = BranchSpec::new(self.name.clone(), BranchType::from(self.branch_type));
        if self.optional {
            spec = spec.optional();
        }
        spec
    }
}

impl From<&BranchSpec> for PortableBranchSpec {
    fn from(value: &BranchSpec) -> Self {
        Self {
            name: value.name.clone(),
            branch_type: PortableBranchType::from(value.branch_type),
            optional: value.optional,
        }
    }
}

impl From<PortableBranchType> for BranchType {
    fn from(value: PortableBranchType) -> Self {
        match value {
            PortableBranchType::Bool => Self::Bool,
            PortableBranchType::I8 => Self::I8,
            PortableBranchType::U8 => Self::U8,
            PortableBranchType::I16 => Self::I16,
            PortableBranchType::U16 => Self::U16,
            PortableBranchType::I32 => Self::I32,
            PortableBranchType::U32 => Self::U32,
            PortableBranchType::I64 => Self::I64,
            PortableBranchType::U64 => Self::U64,
            PortableBranchType::F32 => Self::F32,
            PortableBranchType::VecBool => Self::VecBool,
            PortableBranchType::VecI8 => Self::VecI8,
            PortableBranchType::VecU8 => Self::VecU8,
            PortableBranchType::VecI16 => Self::VecI16,
            PortableBranchType::VecU16 => Self::VecU16,
            PortableBranchType::VecI32 => Self::VecI32,
            PortableBranchType::VecU32 => Self::VecU32,
            PortableBranchType::VecI64 => Self::VecI64,
            PortableBranchType::VecU64 => Self::VecU64,
            PortableBranchType::VecF32 => Self::VecF32,
        }
    }
}

impl From<BranchType> for PortableBranchType {
    fn from(value: BranchType) -> Self {
        match value {
            BranchType::Bool => Self::Bool,
            BranchType::I8 => Self::I8,
            BranchType::U8 => Self::U8,
            BranchType::I16 => Self::I16,
            BranchType::U16 => Self::U16,
            BranchType::I32 => Self::I32,
            BranchType::U32 => Self::U32,
            BranchType::I64 => Self::I64,
            BranchType::U64 => Self::U64,
            BranchType::F32 => Self::F32,
            BranchType::VecBool => Self::VecBool,
            BranchType::VecI8 => Self::VecI8,
            BranchType::VecU8 => Self::VecU8,
            BranchType::VecI16 => Self::VecI16,
            BranchType::VecU16 => Self::VecU16,
            BranchType::VecI32 => Self::VecI32,
            BranchType::VecU32 => Self::VecU32,
            BranchType::VecI64 => Self::VecI64,
            BranchType::VecU64 => Self::VecU64,
            BranchType::VecF32 => Self::VecF32,
        }
    }
}

fn source_id(id: usize) -> String {
    format!("source-{id}")
}

fn map_id(id: usize) -> String {
    format!("map-{id}")
}

fn reduce_id(id: usize) -> String {
    format!("reduce-{id}")
}

fn parse_prefixed_id(id: &str, prefix: &str) -> Option<usize> {
    id.strip_prefix(prefix)?.parse().ok()
}

fn required<'a, T>(value: &'a Option<T>, node_id: &str, field: &str) -> Result<&'a T> {
    value.as_ref().ok_or_else(|| {
        WorkflowError::InvalidGraph(format!(
            "PortableGraph node `{node_id}` is missing `{field}`"
        ))
    })
}

fn is_false(value: &bool) -> bool {
    !*value
}
