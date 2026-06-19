//! nano-io — ROOT input reading and skim output writing for nano.rust.
//!
//! Reading wraps the forked `root-io` crate (TTree reader); writing extends
//! that fork with a native pure-Rust TTree writer for the fixed skim schema
//! (`bool`, `i32`, `u32`, `u64`, `f32`, `Vec<f32>`) plus filtered
//! `Runs`/`LuminosityBlocks`. See `docs/rust-migration.md`.
//!
//! This is the green skeleton; the reader/writer land here as the fork's
//! standalone build and write path come online.

pub mod read {
    use std::path::Path;

    use failure::Error;
    use futures::StreamExt;
    use nom::number::complete::be_i32;
    use root_io::RootFile;

    /// Read an `i32` branch from the first TTree in a local ROOT file.
    pub async fn read_i32_branch(path: &Path, branch_name: &str) -> Result<Vec<i32>, Error> {
        let file = RootFile::new(path).await?;
        let tree = file
            .items()
            .iter()
            .find(|item| item.verbose_info().contains("TTree"))
            .ok_or_else(|| failure::format_err!("No TTree found in {}", path.display()))?
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

    #[tokio::test]
    async fn reads_simple_root_i32_branch() {
        let path = Path::new("../root-io/src/test_data/simple.root");
        let values = read_i32_branch(path, "one").await.unwrap();
        assert_eq!(values, vec![1, 2, 3, 4]);
    }
}
