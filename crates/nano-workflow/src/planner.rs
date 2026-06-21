use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nano_core::{BranchSchema, Event};
use nano_producers::{MuonProducer, MuonSkimRow};
use serde::{Deserialize, Serialize};

use crate::artifacts::{ChunkSpec, EntryRange, MergedOutput, PartialOutput};
use crate::error::Result;

pub type Kernel =
    Arc<dyn Fn(&Event) -> nano_core::Result<Option<MuonSkimRow>> + Send + Sync + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceNode {
    pub id: usize,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapNode {
    pub id: usize,
    pub chunk: ChunkSpec,
    pub artifact_path: PathBuf,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MapDone {
    pub node_id: usize,
    pub key: String,
    pub output: PartialOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReduceNode {
    pub id: usize,
    pub artifact_path: PathBuf,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReduceDone {
    pub node_id: usize,
    pub key: String,
    pub output: MergedOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkNode {
    pub output_path: PathBuf,
    pub manifest_path: PathBuf,
}

pub struct WorkflowPlan {
    pub sources: Vec<SourceNode>,
    pub maps: Vec<MapNode>,
    pub reduce: ReduceNode,
    pub sink: SinkNode,
    pub schema: BranchSchema,
    pub chunk_size: usize,
    pub kernel: Kernel,
    pub kernel_id: String,
}

impl WorkflowPlan {
    pub fn node_summaries(&self) -> Vec<WorkflowNodeSummary> {
        let mut nodes = Vec::with_capacity(self.sources.len() + self.maps.len() + 2);
        nodes.extend(self.sources.iter().map(|source| WorkflowNodeSummary {
            id: format!("source-{}", source.id),
            kind: WorkflowNodeKind::Source,
            label: source.path.display().to_string(),
            chunk: None,
        }));
        nodes.extend(self.maps.iter().map(|map| WorkflowNodeSummary {
            id: format!("map-{}", map.id),
            kind: WorkflowNodeKind::Map,
            label: format!(
                "{} entries {}-{}",
                map.chunk.source, map.chunk.entry_range.start, map.chunk.entry_range.end
            ),
            chunk: Some(map.chunk.clone()),
        }));
        nodes.push(WorkflowNodeSummary {
            id: format!("reduce-{}", self.reduce.id),
            kind: WorkflowNodeKind::Reduce,
            label: self.reduce.artifact_path.display().to_string(),
            chunk: None,
        });
        nodes.push(WorkflowNodeSummary {
            id: "sink-0".to_string(),
            kind: WorkflowNodeKind::Sink,
            label: self.sink.output_path.display().to_string(),
            chunk: None,
        });
        nodes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowNodeKind {
    Source,
    Map,
    Reduce,
    Sink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeSummary {
    pub id: String,
    pub kind: WorkflowNodeKind,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<ChunkSpec>,
}

pub fn plan_muon_workflow(
    inputs: impl IntoIterator<Item = impl AsRef<Path>>,
    schema: BranchSchema,
    chunk_size: usize,
    cache_dir: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> Result<WorkflowPlan> {
    plan_workflow_with_kernel_id(
        inputs,
        schema,
        chunk_size,
        cache_dir,
        output_path,
        MuonProducer::analyze,
        "muon",
    )
}

pub fn plan_workflow<K>(
    inputs: impl IntoIterator<Item = impl AsRef<Path>>,
    schema: BranchSchema,
    chunk_size: usize,
    cache_dir: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    kernel: K,
) -> Result<WorkflowPlan>
where
    K: Fn(&Event) -> nano_core::Result<Option<MuonSkimRow>> + Send + Sync + 'static,
{
    plan_workflow_with_kernel_id(
        inputs,
        schema,
        chunk_size,
        cache_dir,
        output_path,
        kernel,
        "custom",
    )
}

pub fn plan_workflow_with_kernel_id<K>(
    inputs: impl IntoIterator<Item = impl AsRef<Path>>,
    schema: BranchSchema,
    chunk_size: usize,
    cache_dir: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    kernel: K,
    kernel_id: impl Into<String>,
) -> Result<WorkflowPlan>
where
    K: Fn(&Event) -> nano_core::Result<Option<MuonSkimRow>> + Send + Sync + 'static,
{
    let chunk_size = chunk_size.max(1);
    let cache_dir = cache_dir.as_ref();
    fs::create_dir_all(cache_dir)?;

    let input_paths = inputs
        .into_iter()
        .map(|path| path.as_ref().to_path_buf())
        .collect::<Vec<_>>();
    let sources = input_paths
        .iter()
        .enumerate()
        .map(|(id, path)| SourceNode {
            id,
            path: path.clone(),
        })
        .collect::<Vec<_>>();

    let mut maps = Vec::new();
    for source in &sources {
        let mut start = 0_usize;
        let mut events_in_chunk = 0_usize;
        let source_string = source.path.display().to_string();
        let iterator = source_events(&source.path, &source_string, &schema, chunk_size)?;

        for event in iterator {
            let _ = event?;
            events_in_chunk += 1;
            if events_in_chunk == chunk_size {
                maps.push(map_node(
                    cache_dir,
                    maps.len(),
                    &source.path,
                    start,
                    chunk_size,
                ));
                start += events_in_chunk;
                events_in_chunk = 0;
            }
        }

        if events_in_chunk > 0 {
            maps.push(map_node(
                cache_dir,
                maps.len(),
                &source.path,
                start,
                events_in_chunk,
            ));
        }
    }

    Ok(WorkflowPlan {
        sources,
        maps,
        reduce: ReduceNode {
            id: 0,
            artifact_path: cache_dir.join("reduce.json"),
            manifest_path: cache_dir.join("reduce.manifest.json"),
        },
        sink: SinkNode {
            output_path: output_path.as_ref().to_path_buf(),
            manifest_path: output_path.as_ref().with_extension("root.manifest.json"),
        },
        schema,
        chunk_size,
        kernel: Arc::new(kernel),
        kernel_id: kernel_id.into(),
    })
}

fn map_node(cache_dir: &Path, id: usize, source: &Path, start: usize, len: usize) -> MapNode {
    let source_string = source.display().to_string();
    MapNode {
        id,
        chunk: ChunkSpec {
            source: source_string,
            entry_range: EntryRange {
                start,
                end: start + len,
            },
        },
        artifact_path: cache_dir.join(format!("map-{id}.json")),
        manifest_path: cache_dir.join(format!("map-{id}.manifest.json")),
    }
}

#[cfg(feature = "http")]
fn source_events(
    path: &Path,
    source: &str,
    schema: &BranchSchema,
    chunk_size: usize,
) -> Result<Box<dyn Iterator<Item = nano_io::Result<Event>>>> {
    if is_http_url(source) {
        Ok(Box::new(nano_io::events_url_chunked(
            source, schema, chunk_size,
        )?))
    } else {
        Ok(Box::new(nano_io::events_chunked(path, schema, chunk_size)?))
    }
}

#[cfg(not(feature = "http"))]
fn source_events(
    path: &Path,
    source: &str,
    schema: &BranchSchema,
    chunk_size: usize,
) -> Result<Box<dyn Iterator<Item = nano_io::Result<Event>>>> {
    if is_http_url(source) {
        return Err(crate::error::WorkflowError::UnsupportedSource(format!(
            "HTTP source `{source}` requires the nano-workflow `http` feature"
        )));
    }
    Ok(Box::new(nano_io::events_chunked(path, schema, chunk_size)?))
}

fn is_http_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}
