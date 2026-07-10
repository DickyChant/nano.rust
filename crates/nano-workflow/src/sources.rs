use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Result, WorkflowError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Local,
    Http,
    XRootD,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSource {
    value: String,
}

impl InputSource {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        let value = value.trim();
        if value.is_empty() {
            return Err(WorkflowError::InvalidSourceList(
                "input source cannot be empty".to_string(),
            ));
        }
        Ok(Self {
            value: value.to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub fn kind(&self) -> SourceKind {
        if is_http_source(&self.value) {
            SourceKind::Http
        } else if is_xrootd_source(&self.value) {
            SourceKind::XRootD
        } else {
            SourceKind::Local
        }
    }

    pub fn is_eos_path(&self) -> bool {
        is_eos_path(&self.value)
    }

    pub fn to_path_buf(&self) -> PathBuf {
        PathBuf::from(&self.value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceList {
    sources: Vec<InputSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EosResolveOptions {
    pub store_root: PathBuf,
    pub max_files_per_dataset: usize,
    pub max_depth: usize,
    pub x509_proxy: Option<PathBuf>,
}

impl Default for EosResolveOptions {
    fn default() -> Self {
        Self {
            store_root: PathBuf::from("/eos/cms/store"),
            max_files_per_dataset: 1,
            max_depth: 8,
            x509_proxy: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EosDatasetFiles {
    pub dataset: String,
    pub base_path: PathBuf,
    pub files: Vec<PathBuf>,
}

impl SourceList {
    pub fn from_csv(value: &str) -> Result<Self> {
        Self::from_values(value.split(','))
    }

    pub fn from_text(value: &str) -> Result<Self> {
        Self::from_values(
            value
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#')),
        )
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|source| {
            WorkflowError::InvalidSourceList(format!(
                "failed to read source list `{}`: {source}",
                path.display()
            ))
        })?;
        Self::from_text(&text).map_err(|error| {
            WorkflowError::InvalidSourceList(format!(
                "invalid source list `{}`: {error}",
                path.display()
            ))
        })
    }

    pub fn sources(&self) -> &[InputSource] {
        &self.sources
    }

    pub fn contains_eos_paths(&self) -> bool {
        self.sources.iter().any(InputSource::is_eos_path)
    }

    pub fn into_paths(self) -> Vec<PathBuf> {
        self.sources
            .into_iter()
            .map(|source| source.to_path_buf())
            .collect()
    }

    fn from_values<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<Self> {
        let sources = values
            .into_iter()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(InputSource::new)
            .collect::<Result<Vec<_>>>()?;
        if sources.is_empty() {
            return Err(WorkflowError::InvalidSourceList(
                "source list did not contain any input sources".to_string(),
            ));
        }
        Ok(Self { sources })
    }
}

pub(crate) fn is_http_source(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

pub(crate) fn is_xrootd_source(source: &str) -> bool {
    source.starts_with("root://")
}

pub fn validate_x509_proxy(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let metadata = fs::metadata(path).map_err(|source| {
        WorkflowError::InvalidSourceList(format!(
            "failed to access X509 proxy `{}`: {source}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(WorkflowError::InvalidSourceList(format!(
            "X509 proxy `{}` is not a regular file",
            path.display()
        )));
    }
    if metadata.len() == 0 {
        return Err(WorkflowError::InvalidSourceList(format!(
            "X509 proxy `{}` is empty",
            path.display()
        )));
    }
    Ok(())
}

pub fn eos_dataset_base_path(dataset: &str, store_root: impl AsRef<Path>) -> Result<PathBuf> {
    let parsed = ParsedEosDataset::parse(dataset)?;
    Ok(parsed.base_path(store_root.as_ref()))
}

pub fn resolve_eos_dataset_files(
    dataset: &str,
    options: &EosResolveOptions,
) -> Result<EosDatasetFiles> {
    if let Some(proxy) = &options.x509_proxy {
        validate_x509_proxy(proxy)?;
    }
    if options.max_files_per_dataset == 0 {
        return Err(WorkflowError::InvalidSourceList(
            "max_files_per_dataset must be greater than zero".to_string(),
        ));
    }

    let base_path = eos_dataset_base_path(dataset, &options.store_root)?;
    let files = collect_root_files(&base_path, options.max_files_per_dataset, options.max_depth)?;
    if files.is_empty() {
        return Err(WorkflowError::InvalidSourceList(format!(
            "EOS dataset `{dataset}` did not contain ROOT files under `{}`",
            base_path.display()
        )));
    }

    Ok(EosDatasetFiles {
        dataset: dataset.to_string(),
        base_path,
        files,
    })
}

fn is_eos_path(source: &str) -> bool {
    source == "/eos" || source.starts_with("/eos/")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedEosDataset {
    primary: String,
    processing_era: String,
    processing_suffix: String,
    tier: String,
    kind: EosDatasetKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EosDatasetKind {
    Data,
    Mc,
}

impl ParsedEosDataset {
    fn parse(dataset: &str) -> Result<Self> {
        let dataset = dataset.trim();
        let parts = dataset
            .trim_matches('/')
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let [primary, processing, tier] = parts.as_slice() else {
            return Err(WorkflowError::InvalidSourceList(format!(
                "invalid CMS dataset `{dataset}`; expected /Primary/Processing/Tier"
            )));
        };
        let Some((processing_era, processing_suffix)) = processing.split_once('-') else {
            return Err(WorkflowError::InvalidSourceList(format!(
                "invalid CMS dataset `{dataset}`; processing campaign must contain `-`"
            )));
        };
        let kind = match *tier {
            "NANOAOD" => EosDatasetKind::Data,
            "NANOAODSIM" => EosDatasetKind::Mc,
            _ => {
                return Err(WorkflowError::InvalidSourceList(format!(
                    "invalid CMS dataset `{dataset}`; expected NANOAOD or NANOAODSIM tier"
                )));
            }
        };

        Ok(Self {
            primary: (*primary).to_string(),
            processing_era: processing_era.to_string(),
            processing_suffix: processing_suffix.to_string(),
            tier: (*tier).to_string(),
            kind,
        })
    }

    fn base_path(&self, store_root: &Path) -> PathBuf {
        let store_kind = match self.kind {
            EosDatasetKind::Data => "data",
            EosDatasetKind::Mc => "mc",
        };
        store_root
            .join(store_kind)
            .join(&self.processing_era)
            .join(&self.primary)
            .join(&self.tier)
            .join(&self.processing_suffix)
    }
}

fn collect_root_files(path: &Path, limit: usize, max_depth: usize) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_root_files_rec(path, 0, max_depth, limit, &mut files)?;
    Ok(files)
}

fn collect_root_files_rec(
    path: &Path,
    depth: usize,
    max_depth: usize,
    limit: usize,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    if files.len() >= limit {
        return Ok(());
    }

    let mut entries = fs::read_dir(path)
        .map_err(|source| {
            WorkflowError::InvalidSourceList(format!(
                "failed to list EOS path `{}`: {source}",
                path.display()
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|source| {
            WorkflowError::InvalidSourceList(format!(
                "failed to read EOS path `{}`: {source}",
                path.display()
            ))
        })?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        if files.len() >= limit {
            break;
        }
        let file_type = entry.file_type().map_err(|source| {
            WorkflowError::InvalidSourceList(format!(
                "failed to inspect EOS path `{}`: {source}",
                entry.path().display()
            ))
        })?;
        let entry_path = entry.path();
        if file_type.is_file() && entry_path.extension().is_some_and(|ext| ext == "root") {
            files.push(entry_path);
        } else if file_type.is_dir() && depth < max_depth {
            collect_root_files_rec(&entry_path, depth + 1, max_depth, limit, files)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn source_list_classifies_mounted_eos_as_local() {
        let source = InputSource::new("/eos/cms/store/mc/sample.root").unwrap();

        assert_eq!(source.kind(), SourceKind::Local);
        assert!(source.is_eos_path());
    }

    #[test]
    fn source_list_reports_when_any_source_is_eos_mounted() {
        let list = SourceList::from_text(
            "\
# Direct mounted EOS NanoAOD inputs
/tmp/local.root
/eos/cms/store/data/Run2024C/file.root
",
        )
        .unwrap();

        assert!(list.contains_eos_paths());
        assert_eq!(list.sources().len(), 2);
    }

    #[test]
    fn proxy_validation_rejects_missing_and_empty_files() {
        let root = temp_test_dir("proxy-validation");
        let missing = root.join("missing.proxy");
        let empty = root.join("empty.proxy");
        fs::write(&empty, "").unwrap();

        assert!(validate_x509_proxy(&missing).is_err());
        assert!(validate_x509_proxy(&empty).is_err());
    }

    #[test]
    fn proxy_validation_accepts_non_empty_regular_file() {
        let root = temp_test_dir("proxy-validation-ok");
        let proxy = root.join("x509.proxy");
        fs::write(&proxy, "proxy payload").unwrap();

        validate_x509_proxy(&proxy).unwrap();
    }

    #[test]
    fn cms_dataset_paths_map_to_mounted_eos_layout() {
        let store = Path::new("/eos/cms/store");

        assert_eq!(
            eos_dataset_base_path(
                "/WZJJto3LNu-EWK_TuneCP5_13p6TeV_madgraph-pythia8/RunIII2024Summer24NanoAODv15-150X_mcRun3_2024_realistic_v2-v2/NANOAODSIM",
                store,
            )
            .unwrap(),
            store
                .join("mc")
                .join("RunIII2024Summer24NanoAODv15")
                .join("WZJJto3LNu-EWK_TuneCP5_13p6TeV_madgraph-pythia8")
                .join("NANOAODSIM")
                .join("150X_mcRun3_2024_realistic_v2-v2")
        );
        assert_eq!(
            eos_dataset_base_path("/Muon0/Run2024C-MINIv6NANOv15-v1/NANOAOD", store,).unwrap(),
            store
                .join("data")
                .join("Run2024C")
                .join("Muon0")
                .join("NANOAOD")
                .join("MINIv6NANOv15-v1")
        );
    }

    #[test]
    fn eos_resolver_finds_root_files_with_dataset_bound() {
        let root = temp_test_dir("eos-resolver");
        let store = root.join("store");
        let dataset = "/Muon0/Run2024C-MINIv6NANOv15-v1/NANOAOD";
        let base = store
            .join("data")
            .join("Run2024C")
            .join("Muon0")
            .join("NANOAOD")
            .join("MINIv6NANOv15-v1")
            .join("2530000");
        fs::create_dir_all(&base).unwrap();
        fs::write(base.join("a.root"), "").unwrap();
        fs::write(base.join("b.root"), "").unwrap();

        let resolved = resolve_eos_dataset_files(
            dataset,
            &EosResolveOptions {
                store_root: store,
                max_files_per_dataset: 1,
                max_depth: 4,
                x509_proxy: None,
            },
        )
        .unwrap();

        assert_eq!(resolved.files.len(), 1);
        assert_eq!(resolved.files[0].file_name().unwrap(), "a.root");
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nano-workflow-{}-{timestamp}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
