//! nano-io — ROOT input reading and skim output writing for nano.rust.
//!
//! Reading wraps the forked `root-io` crate (TTree reader); writing extends
//! that fork with a native pure-Rust TTree writer for the fixed skim schema
//! (`bool`, `i32`, `u32`, `u64`, `f32`, `Vec<f32>`) plus filtered
//! `Runs`/`LuminosityBlocks`. See `docs/rust-migration.md`.

use std::path::Path;

use futures::executor::block_on;
use nano_core::{BranchSchema, Event};
pub use root_io::{Result, RootError};

/// Synchronously read the `Events` TTree from a ROOT file into one [`Event`] per entry.
pub fn read_events(path: &Path, schema: BranchSchema) -> Result<Vec<Event>> {
    block_on(reader::read_events(path, schema))
}

/// Synchronously read one named TTree from a ROOT file into one [`Event`] per entry.
pub fn read_events_from_tree(
    path: &Path,
    tree_name: &str,
    schema: BranchSchema,
) -> Result<Vec<Event>> {
    block_on(reader::read_events_from_tree(path, tree_name, schema))
}

pub mod reader {
    use std::collections::HashMap;
    use std::path::Path;

    use futures::StreamExt;
    use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
    use nom::number::complete::{
        be_f32, be_i16, be_i32, be_i64, be_i8, be_u16, be_u32, be_u64, be_u8,
    };
    use root_io::tree_reader::Tree;
    use root_io::RootFile;

    use crate::{Result, RootError};

    /// Read the `Events` TTree from a ROOT file into one [`Event`] per entry.
    pub async fn read_events(path: &Path, schema: BranchSchema) -> Result<Vec<Event>> {
        read_events_from_tree(path, "Events", schema).await
    }

    /// Read one named TTree from a ROOT file into one [`Event`] per entry.
    pub async fn read_events_from_tree(
        path: &Path,
        tree_name: &str,
        schema: BranchSchema,
    ) -> Result<Vec<Event>> {
        let file = RootFile::new(path).await?;
        let tree = tree_by_name_or_first(&file, path, tree_name).await?;
        let columns = read_columns(&tree, schema.specs()).await?;
        let n_entries = columns.values().map(BranchColumn::len).max().unwrap_or(0);

        let mut events = Vec::with_capacity(n_entries);
        for entry in 0..n_entries {
            events.push(
                Event::from_columns(schema.clone(), columns.clone(), entry)
                    .map_err(|err| RootError::other(err.to_string()))?,
            );
        }
        Ok(events)
    }

    async fn tree_by_name_or_first(
        file: &RootFile,
        path: &Path,
        tree_name: &str,
    ) -> Result<Tree> {
        if let Some(item) = file
            .items()
            .iter()
            .find(|item| item.name() == tree_name && item.verbose_info().contains("TTree"))
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

    async fn read_columns(
        tree: &Tree,
        specs: &[BranchSpec],
    ) -> Result<HashMap<String, BranchColumn>> {
        let mut columns = HashMap::new();

        for spec in specs.iter().filter(|spec| !spec.branch_type.is_vector()) {
            match read_scalar_column(tree, spec).await {
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
            match read_vector_column(tree, spec, &columns).await {
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

    async fn read_scalar_column(tree: &Tree, spec: &BranchSpec) -> Result<BranchColumn> {
        let branch = tree.branch_by_name(&spec.name)?;
        let column = match spec.branch_type {
            BranchType::Bool => BranchColumn::Bool(
                branch
                    .as_fixed_size_iterator(|i| be_u8(i).map(|(i, value)| (i, value != 0)))
                    .collect()
                    .await,
            ),
            BranchType::I8 => {
                BranchColumn::I8(branch.as_fixed_size_iterator(|i| be_i8(i)).collect().await)
            }
            BranchType::U8 => {
                BranchColumn::U8(branch.as_fixed_size_iterator(|i| be_u8(i)).collect().await)
            }
            BranchType::I16 => {
                BranchColumn::I16(branch.as_fixed_size_iterator(|i| be_i16(i)).collect().await)
            }
            BranchType::U16 => {
                BranchColumn::U16(branch.as_fixed_size_iterator(|i| be_u16(i)).collect().await)
            }
            BranchType::I32 => {
                BranchColumn::I32(branch.as_fixed_size_iterator(|i| be_i32(i)).collect().await)
            }
            BranchType::U32 => {
                BranchColumn::U32(branch.as_fixed_size_iterator(|i| be_u32(i)).collect().await)
            }
            BranchType::I64 => {
                BranchColumn::I64(branch.as_fixed_size_iterator(|i| be_i64(i)).collect().await)
            }
            BranchType::U64 => {
                BranchColumn::U64(branch.as_fixed_size_iterator(|i| be_u64(i)).collect().await)
            }
            BranchType::F32 => {
                BranchColumn::F32(branch.as_fixed_size_iterator(|i| be_f32(i)).collect().await)
            }
            branch_type => {
                return Err(RootError::other(format!(
                    "branch `{}` has non-scalar type {:?}",
                    spec.name,
                    branch_type
                )));
            }
        };
        Ok(column)
    }

    async fn read_vector_column(
        tree: &Tree,
        spec: &BranchSpec,
        columns: &HashMap<String, BranchColumn>,
    ) -> Result<BranchColumn> {
        let branch = tree.branch_by_name(&spec.name)?;
        let count_branch = count_branch_name(&spec.name)?;
        let counts = if let Some(BranchColumn::U32(values)) = columns.get(&count_branch) {
            values.clone()
        } else {
            tree.branch_by_name(&count_branch)?
                .as_fixed_size_iterator(|i| be_u32(i))
                .collect()
                .await
        };

        let column = match spec.branch_type {
            BranchType::VecBool => BranchColumn::VecBool(
                branch
                    .as_var_size_iterator(|i| be_u8(i).map(|(i, value)| (i, value != 0)), counts)
                    .collect()
                    .await,
            ),
            BranchType::VecI8 => BranchColumn::VecI8(
                branch
                    .as_var_size_iterator(|i| be_i8(i), counts)
                    .collect()
                    .await,
            ),
            BranchType::VecU8 => BranchColumn::VecU8(
                branch
                    .as_var_size_iterator(|i| be_u8(i), counts)
                    .collect()
                    .await,
            ),
            BranchType::VecI16 => BranchColumn::VecI16(
                branch
                    .as_var_size_iterator(|i| be_i16(i), counts)
                    .collect()
                    .await,
            ),
            BranchType::VecU16 => BranchColumn::VecU16(
                branch
                    .as_var_size_iterator(|i| be_u16(i), counts)
                    .collect()
                    .await,
            ),
            BranchType::VecI32 => BranchColumn::VecI32(
                branch
                    .as_var_size_iterator(|i| be_i32(i), counts)
                    .collect()
                    .await,
            ),
            BranchType::VecU32 => BranchColumn::VecU32(
                branch
                    .as_var_size_iterator(|i| be_u32(i), counts)
                    .collect()
                    .await,
            ),
            BranchType::VecI64 => BranchColumn::VecI64(
                branch
                    .as_var_size_iterator(|i| be_i64(i), counts)
                    .collect()
                    .await,
            ),
            BranchType::VecU64 => BranchColumn::VecU64(
                branch
                    .as_var_size_iterator(|i| be_u64(i), counts)
                    .collect()
                    .await,
            ),
            BranchType::VecF32 => BranchColumn::VecF32(
                branch
                    .as_var_size_iterator(|i| be_f32(i), counts)
                    .collect()
                    .await,
            ),
            branch_type => {
                return Err(RootError::other(format!(
                    "branch `{}` has non-vector type {:?}",
                    spec.name,
                    branch_type
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
