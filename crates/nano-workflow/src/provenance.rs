use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::UNIX_EPOCH;

use nano_core::BranchSchema;
use serde::{Deserialize, Serialize};

use crate::artifacts::ChunkSpec;
use crate::error::Result;

pub const CODE_SPEC_VERSION: &str = "nano-workflow.first-slice.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub key: String,
    pub artifact_kind: String,
    pub code_spec_version: String,
    pub read_branches: Vec<String>,
    pub inputs: Vec<String>,
}

impl Manifest {
    pub fn new(
        key: String,
        artifact_kind: impl Into<String>,
        read_branches: Vec<String>,
        inputs: Vec<String>,
    ) -> Self {
        Self {
            key,
            artifact_kind: artifact_kind.into(),
            code_spec_version: CODE_SPEC_VERSION.to_string(),
            read_branches,
            inputs,
        }
    }
}

pub(crate) fn read_branch_signature(schema: &BranchSchema) -> Vec<String> {
    schema
        .specs()
        .iter()
        .map(|spec| {
            format!(
                "{}:{:?}:optional={}",
                spec.name, spec.branch_type, spec.optional
            )
        })
        .collect()
}

pub(crate) fn map_key(chunk: &ChunkSpec, schema: &BranchSchema) -> Result<String> {
    let metadata = fs::metadata(&chunk.source)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    Ok(hash_parts(&[
        CODE_SPEC_VERSION.to_string(),
        "map".to_string(),
        chunk.source.clone(),
        metadata.len().to_string(),
        modified.to_string(),
        chunk.entry_range.start.to_string(),
        chunk.entry_range.end.to_string(),
        read_branch_signature(schema).join("|"),
    ]))
}

pub(crate) fn reduce_key(map_keys: &[String], schema: &BranchSchema) -> String {
    let mut parts = vec![
        CODE_SPEC_VERSION.to_string(),
        "reduce".to_string(),
        read_branch_signature(schema).join("|"),
    ];
    parts.extend(map_keys.iter().cloned());
    hash_parts(&parts)
}

pub(crate) fn sink_key(reduce_key: &str, output_path: &Path) -> String {
    hash_parts(&[
        CODE_SPEC_VERSION.to_string(),
        "sink".to_string(),
        output_path.display().to_string(),
        reduce_key.to_string(),
    ])
}

pub(crate) fn hash_parts(parts: &[String]) -> String {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        part.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

pub(crate) fn manifest_matches(path: &Path, expected_key: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let bytes = fs::read(path)?;
    let manifest = serde_json::from_slice::<Manifest>(&bytes)?;
    Ok(manifest.key == expected_key && manifest.code_spec_version == CODE_SPEC_VERSION)
}

pub(crate) fn write_manifest(path: &Path, manifest: &Manifest) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(manifest)?;
    fs::write(path, bytes)?;
    Ok(())
}
