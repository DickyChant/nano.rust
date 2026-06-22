//! nano-io — ROOT input reading and skim output writing for nano.rust.
//!
//! Reading wraps the owned synchronous `nano-rootio` crate for local files and
//! HTTP(S) byte-range sources behind the `http` feature. Writing uses the owned
//! pure-Rust TTree writer for the fixed skim schema (`bool`, `i32`, `u32`,
//! `u64`, `f32`, `Vec<f32>`) plus filtered `Runs`/`LuminosityBlocks`. See
//! `docs/rust-migration.md`.

use std::fmt;
use std::num::{ParseFloatError, ParseIntError, TryFromIntError};
use std::path::Path;

use nano_core::{BranchSchema, Event};
#[cfg(feature = "http")]
pub use nano_rootio::HttpSourceOptions;

pub type Result<T> = std::result::Result<T, RootError>;

#[derive(Debug)]
pub enum RootError {
    Io(std::io::Error),
    Parse(String),
    Format(fmt::Error),
    Decompression(String),
    UnsupportedCompression(String),
    IntConversion(TryFromIntError),
    ParseFloat(ParseFloatError),
    ParseInt(ParseIntError),
    Other(String),
}

impl RootError {
    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse(message.into())
    }

    pub fn decompression(message: impl Into<String>) -> Self {
        Self::Decompression(message.into())
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }
}

impl fmt::Display for RootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Parse(message) => write!(f, "{message}"),
            Self::Format(err) => write!(f, "{err}"),
            Self::Decompression(message) => write!(f, "{message}"),
            Self::UnsupportedCompression(magic) => {
                write!(f, "unsupported ROOT compression algorithm `{magic}`")
            }
            Self::IntConversion(err) => write!(f, "{err}"),
            Self::ParseFloat(err) => write!(f, "{err}"),
            Self::ParseInt(err) => write!(f, "{err}"),
            Self::Other(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for RootError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Format(err) => Some(err),
            Self::IntConversion(err) => Some(err),
            Self::ParseFloat(err) => Some(err),
            Self::ParseInt(err) => Some(err),
            Self::Parse(_)
            | Self::Decompression(_)
            | Self::UnsupportedCompression(_)
            | Self::Other(_) => None,
        }
    }
}

impl From<std::io::Error> for RootError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<fmt::Error> for RootError {
    fn from(err: fmt::Error) -> Self {
        Self::Format(err)
    }
}

impl From<TryFromIntError> for RootError {
    fn from(err: TryFromIntError) -> Self {
        Self::IntConversion(err)
    }
}

impl From<ParseFloatError> for RootError {
    fn from(err: ParseFloatError) -> Self {
        Self::ParseFloat(err)
    }
}

impl From<ParseIntError> for RootError {
    fn from(err: ParseIntError) -> Self {
        Self::ParseInt(err)
    }
}

impl From<nano_rootio::Error> for RootError {
    fn from(err: nano_rootio::Error) -> Self {
        match err {
            nano_rootio::Error::Io(err) => Self::Io(err),
            nano_rootio::Error::Parse { message, .. } => Self::Parse(message),
            nano_rootio::Error::Decompression(message) => Self::Decompression(message),
            nano_rootio::Error::UnsupportedCompression(magic) => {
                Self::UnsupportedCompression(magic)
            }
            other => Self::Other(other.to_string()),
        }
    }
}

/// Synchronously read the `Events` TTree from a ROOT file into one [`Event`] per entry.
pub fn read_events(path: &Path, schema: BranchSchema) -> Result<Vec<Event>> {
    events(path, &schema)?.collect()
}

/// Synchronously read one named TTree from a ROOT file into one [`Event`] per entry.
pub fn read_events_from_tree(
    path: &Path,
    tree_name: &str,
    schema: BranchSchema,
) -> Result<Vec<Event>> {
    events_from_tree(path, tree_name, &schema)?.collect()
}

/// Stream the `Events` TTree in bounded-memory chunks.
pub fn events(path: &Path, schema: &BranchSchema) -> Result<impl Iterator<Item = Result<Event>>> {
    events_from_tree(path, "Events", schema)
}

/// Stream one named TTree in bounded-memory chunks.
pub fn events_from_tree(
    path: &Path,
    tree_name: &str,
    schema: &BranchSchema,
) -> Result<impl Iterator<Item = Result<Event>>> {
    events_chunked_from_tree(path, tree_name, schema, reader::DEFAULT_CHUNK_SIZE)
}

/// Stream the `Events` TTree with an explicit chunk size.
pub fn events_chunked(
    path: &Path,
    schema: &BranchSchema,
    chunk_size: usize,
) -> Result<impl Iterator<Item = Result<Event>>> {
    events_chunked_from_tree(path, "Events", schema, chunk_size)
}

/// Stream one named TTree with an explicit chunk size.
pub fn events_chunked_from_tree(
    path: &Path,
    tree_name: &str,
    schema: &BranchSchema,
    chunk_size: usize,
) -> Result<impl Iterator<Item = Result<Event>>> {
    reader::EventIterator::new(path, tree_name, schema, chunk_size)
}

/// Stream the `Events` TTree from an HTTP(S) URL using byte-range reads.
#[cfg(feature = "http")]
pub fn events_url(url: &str, schema: &BranchSchema) -> Result<reader::EventIterator> {
    events_url_from_tree(url, "Events", schema)
}

/// Stream one named TTree from an HTTP(S) URL using byte-range reads.
#[cfg(feature = "http")]
pub fn events_url_from_tree(
    url: &str,
    tree_name: &str,
    schema: &BranchSchema,
) -> Result<reader::EventIterator> {
    events_url_chunked_from_tree(url, tree_name, schema, reader::DEFAULT_CHUNK_SIZE)
}

/// Stream the `Events` TTree from an HTTP(S) URL with an explicit chunk size.
#[cfg(feature = "http")]
pub fn events_url_chunked(
    url: &str,
    schema: &BranchSchema,
    chunk_size: usize,
) -> Result<reader::EventIterator> {
    events_url_chunked_from_tree(url, "Events", schema, chunk_size)
}

/// Stream one named TTree from an HTTP(S) URL with explicit chunk size and
/// environment-driven TLS options.
#[cfg(feature = "http")]
pub fn events_url_chunked_from_tree(
    url: &str,
    tree_name: &str,
    schema: &BranchSchema,
    chunk_size: usize,
) -> Result<reader::EventIterator> {
    events_url_chunked_from_tree_with_options(
        url,
        tree_name,
        schema,
        chunk_size,
        HttpSourceOptions::from_env(),
    )
}

/// Stream one named TTree from an HTTP(S) URL with explicit TLS options.
#[cfg(feature = "http")]
pub fn events_url_chunked_from_tree_with_options(
    url: &str,
    tree_name: &str,
    schema: &BranchSchema,
    chunk_size: usize,
    options: HttpSourceOptions,
) -> Result<reader::EventIterator> {
    let file = nano_rootio::RootFile::open_url_with_options(url, options)?;
    reader::EventIterator::new_remote_file(file, url, tree_name, schema, chunk_size)
}

pub mod reader {
    use std::path::Path;
    use std::sync::Arc;

    use nano_core::{
        BranchColumn, BranchSchema, BranchSpec, BranchType, Event, EventColumns, JaggedColumn,
    };
    use nano_rootio::{BasketPayloadCache, RootFile, Tree};

    use crate::{Result, RootError};

    pub const DEFAULT_CHUNK_SIZE: usize = 65_536;

    enum EventIteratorBackend {
        Local {
            tree: Tree,
        },
        #[cfg(feature = "http")]
        Remote {
            file: RootFile,
            tree: Tree,
        },
    }

    pub struct EventIterator {
        file_size: u64,
        backend: EventIteratorBackend,
        schema: Arc<BranchSchema>,
        chunk_size: usize,
        total_entries: usize,
        next_entry: usize,
        chunk_start: usize,
        chunk_len: usize,
        chunk_row: usize,
        columns: Option<Arc<EventColumns>>,
    }

    impl EventIterator {
        pub fn new(
            path: &Path,
            tree_name: &str,
            schema: &BranchSchema,
            chunk_size: usize,
        ) -> Result<Self> {
            let file = RootFile::open(path)?;
            let file_size = file.file_size();
            let tree = tree_by_name_or_first(&file, &path.display().to_string(), tree_name)?;
            let total_entries = usize::try_from(tree.entries())?;
            Ok(Self {
                file_size,
                backend: EventIteratorBackend::Local { tree },
                schema: Arc::new(schema.clone()),
                chunk_size: chunk_size.max(1),
                total_entries,
                next_entry: 0,
                chunk_start: 0,
                chunk_len: 0,
                chunk_row: 0,
                columns: None,
            })
        }

        #[cfg(feature = "http")]
        pub fn new_remote_file(
            file: RootFile,
            source_label: &str,
            tree_name: &str,
            schema: &BranchSchema,
            chunk_size: usize,
        ) -> Result<Self> {
            let file_size = file.file_size();
            let tree = tree_by_name_or_first(&file, source_label, tree_name)?;
            let total_entries = usize::try_from(tree.entries()).map_err(RootError::from)?;
            Ok(Self {
                file_size,
                backend: EventIteratorBackend::Remote { file, tree },
                schema: Arc::new(schema.clone()),
                chunk_size: chunk_size.max(1),
                total_entries,
                next_entry: 0,
                chunk_start: 0,
                chunk_len: 0,
                chunk_row: 0,
                columns: None,
            })
        }

        pub fn bytes_fetched(&self) -> u64 {
            match &self.backend {
                EventIteratorBackend::Local { .. } => 0,
                #[cfg(feature = "http")]
                EventIteratorBackend::Remote { file, .. } => file.bytes_fetched(),
            }
        }

        pub fn file_size(&self) -> u64 {
            self.file_size
        }

        fn load_next_chunk(&mut self) -> Result<bool> {
            if self.next_entry >= self.total_entries {
                self.columns = None;
                self.chunk_len = 0;
                self.chunk_row = 0;
                return Ok(false);
            }

            let start = self.next_entry;
            let len = self.chunk_size.min(self.total_entries - start);
            let columns = match &self.backend {
                EventIteratorBackend::Local { tree } => {
                    read_columns_window(tree, self.schema.specs(), start, len)?
                }
                #[cfg(feature = "http")]
                EventIteratorBackend::Remote { tree, .. } => {
                    read_columns_window(tree, self.schema.specs(), start, len)?
                }
            };
            Event::validate_event_columns(&self.schema, &columns, len - 1)
                .map_err(|err| RootError::other(err.to_string()))?;
            self.columns = Some(Arc::new(columns));
            self.chunk_start = start;
            self.chunk_len = len;
            self.chunk_row = 0;
            self.next_entry += len;
            Ok(true)
        }
    }

    impl Iterator for EventIterator {
        type Item = Result<Event>;

        fn next(&mut self) -> Option<Self::Item> {
            if self.chunk_row >= self.chunk_len {
                match self.load_next_chunk() {
                    Ok(true) => {}
                    Ok(false) => return None,
                    Err(err) => return Some(Err(err)),
                }
            }

            let columns = self.columns.as_ref()?.clone();
            let row_index = self.chunk_row;
            let entry = self.chunk_start + row_index;
            self.chunk_row += 1;

            Some(Ok(Event::from_validated_event_columns_at(
                self.schema.clone(),
                columns,
                entry,
                row_index,
            )))
        }
    }

    fn tree_by_name_or_first(file: &RootFile, source_label: &str, tree_name: &str) -> Result<Tree> {
        if file
            .objects()
            .iter()
            .any(|item| item.name() == tree_name && item.class() == "TTree")
        {
            return file.tree(tree_name).map_err(RootError::from);
        }

        let object = file
            .objects()
            .into_iter()
            .find(|item| item.class() == "TTree")
            .ok_or_else(|| RootError::other(format!("No TTree found in {source_label}")))?;
        file.tree(object.name()).map_err(RootError::from)
    }

    fn read_columns_window(
        tree: &Tree,
        specs: &[BranchSpec],
        start: usize,
        len: usize,
    ) -> Result<EventColumns> {
        let mut columns = Vec::with_capacity(specs.len());
        let mut cache = BasketPayloadCache::new();

        for spec in specs {
            let read_result = if spec.branch_type.is_vector() {
                read_vector_column_window(tree, spec, start, len, &mut cache)
            } else {
                read_scalar_column_window(tree, spec, start, len, &mut cache)
            };
            match read_result {
                Ok(column) => {
                    columns.push((spec.name.clone(), column));
                }
                Err(err) if spec.optional => {
                    let _ = err;
                }
                Err(err) => return Err(err),
            }
        }

        Ok(EventColumns::from_ordered(columns))
    }

    fn read_scalar_column_window(
        tree: &Tree,
        spec: &BranchSpec,
        start: usize,
        len: usize,
        cache: &mut BasketPayloadCache,
    ) -> Result<BranchColumn> {
        let start = i64::try_from(start)?;
        let column = match spec.branch_type {
            BranchType::Bool => {
                BranchColumn::Bool(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            BranchType::I8 => {
                BranchColumn::I8(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            BranchType::U8 => {
                BranchColumn::U8(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            BranchType::I16 => {
                BranchColumn::I16(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            BranchType::U16 => {
                BranchColumn::U16(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            BranchType::I32 => {
                BranchColumn::I32(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            BranchType::U32 => {
                BranchColumn::U32(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            BranchType::I64 => {
                BranchColumn::I64(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            BranchType::U64 => {
                BranchColumn::U64(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            BranchType::F32 => {
                BranchColumn::F32(tree.read_scalar_range_cached(&spec.name, start, len, cache)?)
            }
            branch_type => {
                return Err(RootError::other(format!(
                    "branch `{}` has non-scalar type {:?}",
                    spec.name, branch_type
                )));
            }
        };
        Ok(column)
    }

    fn read_vector_column_window(
        tree: &Tree,
        spec: &BranchSpec,
        start: usize,
        len: usize,
        cache: &mut BasketPayloadCache,
    ) -> Result<BranchColumn> {
        let count_branch = count_branch_name(&spec.name)?;
        let start = i64::try_from(start)?;

        let column = match spec.branch_type {
            BranchType::VecBool => BranchColumn::VecBool(tree.read_jagged_range_cached(
                &spec.name,
                &count_branch,
                start,
                len,
                cache,
            )?),
            BranchType::VecI8 => BranchColumn::VecI8(tree.read_jagged_range_cached(
                &spec.name,
                &count_branch,
                start,
                len,
                cache,
            )?),
            BranchType::VecU8 => BranchColumn::VecU8(tree.read_jagged_range_cached(
                &spec.name,
                &count_branch,
                start,
                len,
                cache,
            )?),
            BranchType::VecI16 => BranchColumn::VecI16(tree.read_jagged_range_cached(
                &spec.name,
                &count_branch,
                start,
                len,
                cache,
            )?),
            BranchType::VecU16 => BranchColumn::VecU16(tree.read_jagged_range_cached(
                &spec.name,
                &count_branch,
                start,
                len,
                cache,
            )?),
            BranchType::VecI32 => BranchColumn::VecI32(tree.read_jagged_range_cached(
                &spec.name,
                &count_branch,
                start,
                len,
                cache,
            )?),
            BranchType::VecU32 => BranchColumn::VecU32(tree.read_jagged_range_cached(
                &spec.name,
                &count_branch,
                start,
                len,
                cache,
            )?),
            BranchType::VecI64 => BranchColumn::VecI64(tree.read_jagged_range_cached(
                &spec.name,
                &count_branch,
                start,
                len,
                cache,
            )?),
            BranchType::VecU64 => BranchColumn::VecU64(tree.read_jagged_range_cached(
                &spec.name,
                &count_branch,
                start,
                len,
                cache,
            )?),
            BranchType::VecF32 => {
                let values = tree.read_jagged_flat_range_cached(
                    &spec.name,
                    &count_branch,
                    start,
                    len,
                    cache,
                )?;
                BranchColumn::FlatVecF32(JaggedColumn::new(values.offsets, values.values))
            }
            branch_type => {
                return Err(RootError::other(format!(
                    "branch `{}` has non-vector type {:?}",
                    spec.name, branch_type
                )));
            }
        };
        Ok(column)
    }

    fn count_branch_name(branch_name: &str) -> Result<String> {
        let (object_name, _) = branch_name.split_once('_').ok_or_else(|| {
            RootError::other(format!(
                "cannot infer NanoAOD count branch for vector branch `{branch_name}`"
            ))
        })?;
        Ok(format!("n{object_name}"))
    }
}

pub mod writer {
    use std::fmt::Display;
    use std::path::Path;

    use nano_analysis::{Hist1D, HistSet1D};
    use nano_rootio::write::{
        write_histograms as write_root_histograms, write_tree, Branch, HistogramAxis, Th1F,
    };

    use crate::Result;

    /// One selected output column for the `Events` skim tree.
    #[derive(Debug, Clone, PartialEq)]
    pub enum OutputBranch {
        Bool(String, Vec<bool>),
        I32(String, Vec<i32>),
        U32(String, Vec<u32>),
        U64(String, Vec<u64>),
        F32(String, Vec<f32>),
        VecF32(String, Vec<Vec<f32>>),
    }

    impl OutputBranch {
        pub fn bool(name: impl Into<String>, values: Vec<bool>) -> Self {
            Self::Bool(name.into(), values)
        }

        pub fn i32(name: impl Into<String>, values: Vec<i32>) -> Self {
            Self::I32(name.into(), values)
        }

        pub fn u32(name: impl Into<String>, values: Vec<u32>) -> Self {
            Self::U32(name.into(), values)
        }

        pub fn u64(name: impl Into<String>, values: Vec<u64>) -> Self {
            Self::U64(name.into(), values)
        }

        pub fn f32(name: impl Into<String>, values: Vec<f32>) -> Self {
            Self::F32(name.into(), values)
        }

        pub fn vec_f32(name: impl Into<String>, values: Vec<Vec<f32>>) -> Self {
            Self::VecF32(name.into(), values)
        }

        fn to_root_branch(&self) -> Branch {
            match self {
                Self::Bool(name, values) => Branch::bool(name, values.clone()),
                Self::I32(name, values) => Branch::i32(name, values.clone()),
                Self::U32(name, values) => Branch::u32(name, values.clone()),
                Self::U64(name, values) => Branch::u64(name, values.clone()),
                Self::F32(name, values) => Branch::f32(name, values.clone()),
                Self::VecF32(name, values) => Branch::vec_f32(name, values.clone()),
            }
        }
    }

    /// Write selected rows to a skim TTree named `Events`.
    pub fn write_events(path: &Path, branches: &[OutputBranch]) -> Result<()> {
        let root_branches = branches
            .iter()
            .map(OutputBranch::to_root_branch)
            .collect::<Vec<_>>();
        Ok(write_tree(path, "Events", &root_branches)?)
    }

    /// Write named analysis histograms as top-level ROOT `TH1F` objects.
    pub fn write_histograms(path: &Path, histograms: &[(&str, &Hist1D)]) -> Result<()> {
        let root_histograms = histograms
            .iter()
            .map(|(name, hist)| to_root_histogram(name, hist))
            .collect::<Vec<_>>();
        Ok(write_root_histograms(path, &root_histograms)?)
    }

    /// Write systematic histogram sets as top-level ROOT `TH1F` objects.
    ///
    /// Each variation is named `{base}_{variation}`.
    pub fn write_histogram_sets<S>(path: &Path, histograms: &[(&str, &HistSet1D<S>)]) -> Result<()>
    where
        S: Ord + Display,
    {
        let root_histograms = histograms
            .iter()
            .flat_map(|(base_name, set)| {
                set.iter().map(move |(variation, hist)| {
                    let name = format!("{base_name}_{variation}");
                    to_root_histogram(&name, hist)
                })
            })
            .collect::<Vec<_>>();
        Ok(write_root_histograms(path, &root_histograms)?)
    }

    fn to_root_histogram(name: &str, hist: &Hist1D) -> Th1F {
        let contents = std::iter::once(hist.underflow())
            .chain(hist.bins().iter().copied())
            .chain(std::iter::once(hist.overflow()))
            .collect::<Vec<_>>();
        let sumw2 = std::iter::once(hist.underflow_sumw2())
            .chain(hist.bin_sumw2().iter().copied())
            .chain(std::iter::once(hist.overflow_sumw2()))
            .collect::<Vec<_>>();
        Th1F::new(
            name,
            name,
            HistogramAxis::Fixed {
                bins: hist.nbins(),
                low: hist.low(),
                high: hist.high(),
            },
            contents,
            sumw2,
            hist.entries(),
        )
        .with_weighted_x_stats(hist.sumwx(), hist.sumwx2())
    }
}

pub mod read {
    use std::path::Path;

    use nano_rootio::RootFile;

    use crate::{Result, RootError};

    /// Read an `i32` branch from the first TTree in a local ROOT file.
    pub fn read_i32_branch(path: &Path, branch_name: &str) -> Result<Vec<i32>> {
        let file = RootFile::open(path)?;
        let tree_name = file
            .objects()
            .into_iter()
            .find(|item| item.class() == "TTree")
            .map(|item| item.name().to_string())
            .ok_or_else(|| RootError::other(format!("No TTree found in {}", path.display())))?;
        let tree = file.tree(&tree_name)?;
        Ok(tree.read_scalar(branch_name)?)
    }

    /// Read an `i32` branch from the first TTree in a local ROOT file.
    pub async fn read_i32_branch_async(path: &Path, branch_name: &str) -> Result<Vec<i32>> {
        read_i32_branch(path, branch_name)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::read::read_i32_branch;

    #[test]
    fn reads_simple_root_i32_branch() {
        let path = Path::new("../root-io/src/test_data/simple.root");
        let values = read_i32_branch(path, "one").unwrap();
        assert_eq!(values, vec![1, 2, 3, 4]);
    }
}
