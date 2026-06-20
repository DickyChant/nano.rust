//! nano-io — ROOT input reading and skim output writing for nano.rust.
//!
//! Reading wraps the forked `root-io` crate (TTree reader); writing extends
//! that fork with a native pure-Rust TTree writer for the fixed skim schema
//! (`bool`, `i32`, `u32`, `u64`, `f32`, `Vec<f32>`) plus filtered
//! `Runs`/`LuminosityBlocks`. See `docs/rust-migration.md`.

use std::path::Path;

use nano_core::{BranchSchema, Event};
pub use root_io::{Result, RootError};

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

pub mod reader {
    use std::collections::HashMap;
    use std::path::Path;
    use std::rc::Rc;

    use futures::executor::block_on;
    use nano_core::{BranchColumn, BranchColumns, BranchSchema, BranchSpec, BranchType, Event};
    use nom::multi::count;
    use nom::number::complete::{
        be_f32, be_i16, be_i32, be_i64, be_i8, be_u16, be_u32, be_u64, be_u8,
    };
    use nom::IResult;
    use root_io::tree_reader::TBranch;
    use root_io::tree_reader::Tree;
    use root_io::RootFile;

    use crate::{Result, RootError};

    pub const DEFAULT_CHUNK_SIZE: usize = 65_536;

    pub struct EventIterator {
        tree: Tree,
        schema: Rc<BranchSchema>,
        chunk_size: usize,
        total_entries: usize,
        next_entry: usize,
        chunk_start: usize,
        chunk_len: usize,
        chunk_row: usize,
        columns: Option<Rc<BranchColumns>>,
    }

    impl EventIterator {
        pub fn new(
            path: &Path,
            tree_name: &str,
            schema: &BranchSchema,
            chunk_size: usize,
        ) -> Result<Self> {
            let file = block_on(RootFile::new(path))?;
            let tree = block_on(tree_by_name_or_first(&file, path, tree_name))?;
            let total_entries = usize::try_from(tree.entries()).map_err(RootError::from)?;
            Ok(Self {
                tree,
                schema: Rc::new(schema.clone()),
                chunk_size: chunk_size.max(1),
                total_entries,
                next_entry: 0,
                chunk_start: 0,
                chunk_len: 0,
                chunk_row: 0,
                columns: None,
            })
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
            let columns = read_columns_window(&self.tree, self.schema.specs(), start, len)?;
            self.columns = Some(Rc::new(columns));
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

            Some(
                Event::from_shared_columns_at(self.schema.clone(), columns, entry, row_index)
                    .map_err(|err| RootError::other(err.to_string())),
            )
        }
    }

    async fn tree_by_name_or_first(file: &RootFile, path: &Path, tree_name: &str) -> Result<Tree> {
        if let Some(item) = file
            .items()
            .iter()
            .find(|item| item.name() == format!("`{tree_name}` of type `TTree`"))
        {
            return item.as_tree().await;
        }

        file.items()
            .iter()
            .find(|item| item.verbose_info().contains("TTree"))
            .ok_or_else(|| RootError::other(format!("No TTree found in {}", path.display())))?
            .as_tree()
            .await
    }

    fn read_columns_window(
        tree: &Tree,
        specs: &[BranchSpec],
        start: usize,
        len: usize,
    ) -> Result<BranchColumns> {
        let mut columns = BranchColumns::new();
        let mut count_cache = HashMap::new();

        for spec in specs.iter().filter(|spec| !spec.branch_type.is_vector()) {
            match read_scalar_column_window(tree, spec, start, len) {
                Ok(column) => {
                    columns.insert(spec.name.clone(), column);
                }
                Err(err) if spec.optional => {
                    let _ = err;
                }
                Err(err) => return Err(err),
            }
        }

        for spec in specs.iter().filter(|spec| spec.branch_type.is_vector()) {
            match read_vector_column_window(tree, spec, start, len, &mut count_cache) {
                Ok(column) => {
                    columns.insert(spec.name.clone(), column);
                }
                Err(err) if spec.optional => {
                    let _ = err;
                }
                Err(err) => return Err(err),
            }
        }

        Ok(columns)
    }

    fn read_scalar_column_window(
        tree: &Tree,
        spec: &BranchSpec,
        start: usize,
        len: usize,
    ) -> Result<BranchColumn> {
        let branch = tree.branch_by_name(&spec.name)?;
        let column = match spec.branch_type {
            BranchType::Bool => BranchColumn::Bool(read_fixed_window(branch, start, len, |i| {
                be_u8(i).map(|(i, value)| (i, value != 0))
            })?),
            BranchType::I8 => {
                BranchColumn::I8(read_fixed_window(branch, start, len, |i| be_i8(i))?)
            }
            BranchType::U8 => {
                BranchColumn::U8(read_fixed_window(branch, start, len, |i| be_u8(i))?)
            }
            BranchType::I16 => {
                BranchColumn::I16(read_fixed_window(branch, start, len, |i| be_i16(i))?)
            }
            BranchType::U16 => {
                BranchColumn::U16(read_fixed_window(branch, start, len, |i| be_u16(i))?)
            }
            BranchType::I32 => {
                BranchColumn::I32(read_fixed_window(branch, start, len, |i| be_i32(i))?)
            }
            BranchType::U32 => {
                BranchColumn::U32(read_fixed_window(branch, start, len, |i| be_u32(i))?)
            }
            BranchType::I64 => {
                BranchColumn::I64(read_fixed_window(branch, start, len, |i| be_i64(i))?)
            }
            BranchType::U64 => {
                BranchColumn::U64(read_fixed_window(branch, start, len, |i| be_u64(i))?)
            }
            BranchType::F32 => {
                BranchColumn::F32(read_fixed_window(branch, start, len, |i| be_f32(i))?)
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
        count_cache: &mut HashMap<(String, usize, usize), Vec<u32>>,
    ) -> Result<BranchColumn> {
        let branch = tree.branch_by_name(&spec.name)?;
        let count_branch = count_branch_name(&spec.name)?;

        let column = match spec.branch_type {
            BranchType::VecBool => BranchColumn::VecBool(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_u8(i).map(|(i, value)| (i, value != 0)),
            )?),
            BranchType::VecI8 => BranchColumn::VecI8(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_i8(i),
            )?),
            BranchType::VecU8 => BranchColumn::VecU8(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_u8(i),
            )?),
            BranchType::VecI16 => BranchColumn::VecI16(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_i16(i),
            )?),
            BranchType::VecU16 => BranchColumn::VecU16(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_u16(i),
            )?),
            BranchType::VecI32 => BranchColumn::VecI32(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_i32(i),
            )?),
            BranchType::VecU32 => BranchColumn::VecU32(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_u32(i),
            )?),
            BranchType::VecI64 => BranchColumn::VecI64(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_i64(i),
            )?),
            BranchType::VecU64 => BranchColumn::VecU64(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_u64(i),
            )?),
            BranchType::VecF32 => BranchColumn::VecF32(read_vector_window(
                tree,
                branch,
                &count_branch,
                start,
                len,
                count_cache,
                |i| be_f32(i),
            )?),
            branch_type => {
                return Err(RootError::other(format!(
                    "branch `{}` has non-vector type {:?}",
                    spec.name, branch_type
                )));
            }
        };
        Ok(column)
    }

    fn read_fixed_window<T, P>(
        branch: &TBranch,
        start: usize,
        len: usize,
        parser: P,
    ) -> Result<Vec<T>>
    where
        P: for<'a> Fn(&'a [u8]) -> IResult<&'a [u8], T>,
    {
        let mut values = Vec::with_capacity(len);
        let end = start + len;
        for basket_index in basket_indices_for_range(branch, start, end)? {
            let basket_start = basket_first_entry(branch, basket_index)?;
            let (n_entries, buffer) = block_on(branch.read_basket(basket_index))?;
            let basket_len = n_entries as usize;
            let basket_end = basket_start + basket_len;
            let parsed = parse_fixed_basket(&buffer, basket_len, &parser, &branch.name)?;
            let take_start = start.saturating_sub(basket_start);
            let take_end = end.min(basket_end) - basket_start;
            values.extend(
                parsed
                    .into_iter()
                    .skip(take_start)
                    .take(take_end - take_start),
            );
        }
        Ok(values)
    }

    fn read_vector_window<T, P>(
        tree: &Tree,
        branch: &TBranch,
        count_branch: &str,
        start: usize,
        len: usize,
        count_cache: &mut HashMap<(String, usize, usize), Vec<u32>>,
        parser: P,
    ) -> Result<Vec<Vec<T>>>
    where
        P: for<'a> Fn(&'a [u8]) -> IResult<&'a [u8], T>,
    {
        let mut values = Vec::with_capacity(len);
        let end = start + len;
        for basket_index in basket_indices_for_range(branch, start, end)? {
            let basket_start = basket_first_entry(branch, basket_index)?;
            let (n_entries, buffer) = block_on(branch.read_basket(basket_index))?;
            let basket_len = n_entries as usize;
            let basket_end = basket_start + basket_len;
            let counts =
                read_count_range(tree, count_branch, basket_start, basket_len, count_cache)?;
            let parsed = parse_vector_basket(&buffer, &counts, &parser, &branch.name)?;
            let take_start = start.saturating_sub(basket_start);
            let take_end = end.min(basket_end) - basket_start;
            values.extend(
                parsed
                    .into_iter()
                    .skip(take_start)
                    .take(take_end - take_start),
            );
        }
        Ok(values)
    }

    fn read_count_range(
        tree: &Tree,
        branch_name: &str,
        start: usize,
        len: usize,
        count_cache: &mut HashMap<(String, usize, usize), Vec<u32>>,
    ) -> Result<Vec<u32>> {
        let key = (branch_name.to_string(), start, len);
        if let Some(values) = count_cache.get(&key) {
            return Ok(values.clone());
        }
        let branch = tree.branch_by_name(branch_name)?;
        let values = read_fixed_window(branch, start, len, |i| be_u32(i))?;
        count_cache.insert(key, values.clone());
        Ok(values)
    }

    fn parse_fixed_basket<T, P>(
        buffer: &[u8],
        n_entries: usize,
        parser: &P,
        branch_name: &str,
    ) -> Result<Vec<T>>
    where
        P: for<'a> Fn(&'a [u8]) -> IResult<&'a [u8], T>,
    {
        count(parser, n_entries)(buffer)
            .map(|(_, values)| values)
            .map_err(|err| {
                RootError::parse(format!(
                    "failed to parse fixed-size basket for branch `{branch_name}`: {err:?}"
                ))
            })
    }

    fn parse_vector_basket<T, P>(
        buffer: &[u8],
        counts: &[u32],
        parser: &P,
        branch_name: &str,
    ) -> Result<Vec<Vec<T>>>
    where
        P: for<'a> Fn(&'a [u8]) -> IResult<&'a [u8], T>,
    {
        let mut rest = buffer;
        let mut rows = Vec::with_capacity(counts.len());
        for n_elements in counts {
            let parsed = count(parser, *n_elements as usize)(rest).map_err(|err| {
                RootError::parse(format!(
                    "failed to parse jagged basket for branch `{branch_name}`: {err:?}"
                ))
            })?;
            rest = parsed.0;
            rows.push(parsed.1);
        }
        Ok(rows)
    }

    fn basket_indices_for_range(branch: &TBranch, start: usize, end: usize) -> Result<Vec<usize>> {
        if start >= end {
            return Ok(Vec::new());
        }

        let mut indices = Vec::new();
        for index in 0..branch.basket_count() {
            let basket_start = basket_first_entry(branch, index)?;
            let basket_end = match branch.basket_first_entry(index + 1) {
                Some(entry) => usize::try_from(entry).map_err(RootError::from)?,
                None => usize::try_from(branch.total_entries()).map_err(RootError::from)?,
            };
            if basket_start < end && basket_end > start {
                indices.push(index);
            }
        }
        Ok(indices)
    }

    fn basket_first_entry(branch: &TBranch, index: usize) -> Result<usize> {
        branch
            .basket_first_entry(index)
            .ok_or_else(|| RootError::other(format!("basket {index} has no first-entry marker")))
            .and_then(|entry| usize::try_from(entry).map_err(RootError::from))
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
    use std::path::Path;

    use root_io::write::{write_tree, Branch};

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
        write_tree(path, "Events", &root_branches)
    }
}

pub mod read {
    use std::path::Path;

    use futures::executor::block_on;
    use futures::StreamExt;
    use nom::number::complete::be_i32;
    use root_io::RootFile;

    use crate::{Result, RootError};

    /// Read an `i32` branch from the first TTree in a local ROOT file.
    pub fn read_i32_branch(path: &Path, branch_name: &str) -> Result<Vec<i32>> {
        block_on(read_i32_branch_async(path, branch_name))
    }

    /// Asynchronously read an `i32` branch from the first TTree in a local ROOT file.
    pub async fn read_i32_branch_async(path: &Path, branch_name: &str) -> Result<Vec<i32>> {
        let file = RootFile::new(path).await?;
        let tree = file
            .items()
            .iter()
            .find(|item| item.verbose_info().contains("TTree"))
            .ok_or_else(|| RootError::other(format!("No TTree found in {}", path.display())))?
            .as_tree()
            .await?;
        let values = tree
            .branch_by_name(branch_name)?
            .as_fixed_size_iterator(|i| be_i32(i))
            .collect::<Vec<_>>()
            .await;
        Ok(values)
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
