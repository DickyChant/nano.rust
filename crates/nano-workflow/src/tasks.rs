use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use nano_core::{BranchSchema, BranchSpec, BranchType, Event};
use nano_producers::{MuonProducer, MuonSkimRow};

use crate::artifacts::{ChunkSpec, Cutflow, EntryRange, MergedOutput, PartialOutput};
use crate::error::{Result, WorkflowError};
use crate::planner::Kernel;

#[derive(Clone)]
pub struct KernelBinding {
    pub id: String,
    pub schema: BranchSchema,
    pub kernel: Kernel,
}

#[derive(Clone, Default)]
pub struct KernelRegistry {
    kernels: HashMap<String, KernelBinding>,
}

impl KernelRegistry {
    pub fn new() -> Self {
        Self {
            kernels: HashMap::new(),
        }
    }

    pub fn with_muon() -> Self {
        let mut registry = Self::new();
        registry.register("muon", muon_schema(), MuonProducer::analyze);
        registry
    }

    pub fn register<K>(&mut self, id: impl Into<String>, schema: BranchSchema, kernel: K)
    where
        K: Fn(&Event) -> nano_core::Result<Option<MuonSkimRow>> + Send + Sync + 'static,
    {
        let id = id.into();
        self.kernels.insert(
            id.clone(),
            KernelBinding {
                id,
                schema,
                kernel: Arc::new(kernel),
            },
        );
    }

    pub fn get(&self, id: &str) -> Result<&KernelBinding> {
        self.kernels
            .get(id)
            .ok_or_else(|| WorkflowError::UnknownKernel(id.to_string()))
    }
}

impl Default for KernelBinding {
    fn default() -> Self {
        Self {
            id: "muon".to_string(),
            schema: muon_schema(),
            kernel: Arc::new(MuonProducer::analyze),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunChunkRequest {
    pub source: String,
    pub entry_range: EntryRange,
    pub kernel_id: String,
}

impl RunChunkRequest {
    pub fn new(
        source: impl Into<String>,
        start: usize,
        stop: usize,
        kernel_id: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            entry_range: EntryRange { start, end: stop },
            kernel_id: kernel_id.into(),
        }
    }

    pub fn chunk(&self) -> ChunkSpec {
        ChunkSpec {
            source: self.source.clone(),
            entry_range: self.entry_range.clone(),
        }
    }
}

pub fn muon_schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
    ])
    .expect("muon workflow schema is valid")
}

pub fn run_chunk(request: &RunChunkRequest, registry: &KernelRegistry) -> Result<PartialOutput> {
    let binding = registry.get(&request.kernel_id)?;
    run_chunk_with_kernel(
        &request.chunk(),
        &binding.schema,
        request.entry_range.len().max(1),
        &binding.kernel,
    )
}

pub fn run_chunk_to_path(
    request: &RunChunkRequest,
    output_path: impl AsRef<Path>,
    registry: &KernelRegistry,
) -> Result<PartialOutput> {
    let output = run_chunk(request, registry)?;
    write_partial_output(output_path, &output)?;
    Ok(output)
}

pub fn run_chunk_with_kernel(
    chunk: &ChunkSpec,
    schema: &BranchSchema,
    chunk_size: usize,
    kernel: &Kernel,
) -> Result<PartialOutput> {
    if is_http_url(&chunk.source) {
        run_remote_chunk(chunk, schema, chunk_size, kernel)
    } else {
        let iterator =
            nano_io::events_chunked(Path::new(&chunk.source), schema, chunk_size.max(1))?;
        collect_chunk(iterator, &chunk.entry_range, kernel)
    }
}

#[cfg(feature = "http")]
fn run_remote_chunk(
    chunk: &ChunkSpec,
    schema: &BranchSchema,
    chunk_size: usize,
    kernel: &Kernel,
) -> Result<PartialOutput> {
    let iterator = nano_io::events_url_chunked(&chunk.source, schema, chunk_size.max(1))?;
    collect_chunk(iterator, &chunk.entry_range, kernel)
}

#[cfg(not(feature = "http"))]
fn run_remote_chunk(
    chunk: &ChunkSpec,
    _schema: &BranchSchema,
    _chunk_size: usize,
    _kernel: &Kernel,
) -> Result<PartialOutput> {
    Err(WorkflowError::UnsupportedSource(format!(
        "HTTP source `{}` requires the nano-workflow `http` feature",
        chunk.source
    )))
}

fn collect_chunk<I>(iterator: I, entry_range: &EntryRange, kernel: &Kernel) -> Result<PartialOutput>
where
    I: Iterator<Item = nano_io::Result<Event>>,
{
    let mut output = PartialOutput {
        rows: Vec::new(),
        cutflow: Cutflow::default(),
        hists: Vec::new(),
    };

    for event in iterator.skip(entry_range.start).take(entry_range.len()) {
        let event = event?;
        output.cutflow.events_seen += 1;
        if let Some(row) = kernel(&event)? {
            output.cutflow.events_selected += 1;
            output.rows.push(row);
        }
    }

    Ok(output)
}

pub fn merge_partials(partials: impl IntoIterator<Item = PartialOutput>) -> MergedOutput {
    partials
        .into_iter()
        .fold(PartialOutput::default(), PartialOutput::merge)
        .into()
}

pub fn merge_partial_files(
    partial_paths: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<MergedOutput> {
    let partials = partial_paths
        .into_iter()
        .map(read_partial_output)
        .collect::<Result<Vec<_>>>()?;
    Ok(merge_partials(partials))
}

pub fn write_partial_output(path: impl AsRef<Path>, output: &PartialOutput) -> Result<()> {
    write_json(path.as_ref(), output)
}

pub fn read_partial_output(path: impl AsRef<Path>) -> Result<PartialOutput> {
    read_json(path.as_ref())
}

pub fn write_merged_output(path: impl AsRef<Path>, output: &MergedOutput) -> Result<()> {
    write_json(path.as_ref(), output)
}

pub fn read_merged_output(path: impl AsRef<Path>) -> Result<MergedOutput> {
    read_json(path.as_ref())
}

fn write_json<T>(path: &Path, value: &T) -> Result<()>
where
    T: serde::Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn read_json<T>(path: &Path) -> Result<T>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn is_http_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}
