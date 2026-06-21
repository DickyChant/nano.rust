use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nano_rootio::RootFile;
use nano_spec::{codegen, AnalysisSpec, Catalogue, ParseError, SpecError};
use nano_workflow::{plan_muon_workflow, ExecutionMode, Executor, MergedOutput};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const DEFAULT_CATALOGUE_VERSION: &str = "v9";
const DEFAULT_CHUNK_SIZE: usize = 10_000;

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SpecSummary {
    pub path: PathBuf,
    pub catalogue_version: String,
    pub analysis_name: String,
    pub year: String,
    pub objects: Vec<ObjectSummary>,
    pub regions: Vec<String>,
    pub outputs: Vec<String>,
    pub read_branches: Vec<BranchSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ObjectSummary {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BranchSummary {
    pub name: String,
    pub branch_type: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RootInspection {
    pub source: String,
    pub trees: Vec<TreeSummary>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TreeSummary {
    pub name: String,
    pub entries: i64,
    pub branches: Vec<RootBranchSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RootBranchSummary {
    pub name: String,
    pub types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RunSummary {
    pub inputs: Vec<PathBuf>,
    pub mode: String,
    pub events_seen: u64,
    pub events_selected: u64,
    pub cutflow: CutflowSummary,
    pub plot_values: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CutflowSummary {
    pub events_seen: u64,
    pub events_selected: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ValidationIssue {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionError {
    Parse {
        path: PathBuf,
        message: String,
    },
    Catalogue {
        message: String,
    },
    Validation {
        path: PathBuf,
        issues: Vec<ValidationIssue>,
    },
    Codegen {
        path: PathBuf,
        message: String,
    },
    Inspect {
        source: String,
        message: String,
    },
    Workflow {
        message: String,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { path, message } => {
                write!(f, "failed to parse spec `{}`: {message}", path.display())
            }
            Self::Catalogue { message } => write!(f, "failed to load catalogue: {message}"),
            Self::Validation { path, issues } => {
                writeln!(f, "spec validation failed for `{}`", path.display())?;
                for issue in issues {
                    writeln!(f, "- {}", issue.message)?;
                }
                Ok(())
            }
            Self::Codegen { path, message } => {
                write!(
                    f,
                    "failed to generate code for `{}`: {message}",
                    path.display()
                )
            }
            Self::Inspect { source, message } => {
                write!(f, "failed to inspect ROOT source `{source}`: {message}")
            }
            Self::Workflow { message } => write!(f, "workflow failed: {message}"),
        }
    }
}

impl std::error::Error for SessionError {}

pub fn open_spec(path: impl AsRef<Path>) -> Result<SpecSummary> {
    let path = path.as_ref();
    let (_, plan) = load_validated_plan(path)?;
    Ok(SpecSummary {
        path: path.to_path_buf(),
        catalogue_version: DEFAULT_CATALOGUE_VERSION.to_string(),
        analysis_name: plan.spec.name.clone(),
        year: format!("{:?}", plan.spec.year),
        objects: plan
            .spec
            .objects
            .iter()
            .map(|object| ObjectSummary {
                name: object.name.clone(),
                source: object.source.clone(),
            })
            .collect(),
        regions: plan
            .spec
            .regions
            .iter()
            .map(|region| region.name.clone())
            .collect(),
        outputs: plan
            .spec
            .outputs
            .iter()
            .map(|output| output.name.clone())
            .collect(),
        read_branches: branch_summaries(plan.read_branches.specs()),
    })
}

pub fn codegen_source(path: impl AsRef<Path>) -> Result<String> {
    let path = path.as_ref();
    let (_, plan) = load_validated_plan(path)?;
    codegen::generate_producer_source(&plan).map_err(|error| SessionError::Codegen {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

pub fn inspect_root(source: &str, insecure: bool) -> Result<RootInspection> {
    let root_file = open_root_file(source, insecure)?;
    let mut trees = Vec::new();
    let mut seen = HashSet::new();

    for object in root_file.objects() {
        if object.class() != "TTree" || !seen.insert(object.name().to_string()) {
            continue;
        }
        let tree = root_file
            .tree(object.name())
            .map_err(|error| SessionError::Inspect {
                source: source.to_string(),
                message: error.to_string(),
            })?;
        let branches = tree
            .branches()
            .into_iter()
            .map(|branch| RootBranchSummary {
                name: branch.name,
                types: branch.types,
            })
            .collect();
        trees.push(TreeSummary {
            name: object.name().to_string(),
            entries: tree.entries(),
            branches,
        });
    }

    Ok(RootInspection {
        source: source.to_string(),
        trees,
    })
}

pub fn run_muon_dag(
    inputs: impl IntoIterator<Item = impl AsRef<Path>>,
    parallel: bool,
) -> Result<RunSummary> {
    let inputs = inputs
        .into_iter()
        .map(|path| path.as_ref().to_path_buf())
        .collect::<Vec<_>>();
    if inputs.is_empty() {
        return Err(SessionError::Workflow {
            message: "run_muon_dag requires at least one input".to_string(),
        });
    }
    if let Some(url) = inputs
        .iter()
        .map(|path| path.to_string_lossy())
        .find(|source| is_http_url(source))
    {
        return Err(SessionError::Workflow {
            message: format!(
                "planning remote workflow input `{url}` is not supported by the current local DAG planner"
            ),
        });
    }

    let root = unique_session_dir("nano-ui-muon-dag")?;
    let cache_dir = root.join("cache");
    let output_path = root.join("skim.root");
    let plan = plan_muon_workflow(
        inputs.iter(),
        nano_workflow::muon_schema(),
        DEFAULT_CHUNK_SIZE,
        &cache_dir,
        &output_path,
    )
    .map_err(|error| SessionError::Workflow {
        message: error.to_string(),
    })?;
    let mode = if parallel {
        ExecutionMode::Parallel
    } else {
        ExecutionMode::Serial
    };
    let report = Executor::new()
        .run(&plan, mode)
        .map_err(|error| SessionError::Workflow {
            message: error.to_string(),
        })?;

    Ok(run_summary(inputs, mode, &report.merged))
}

fn run_summary(inputs: Vec<PathBuf>, mode: ExecutionMode, merged: &MergedOutput) -> RunSummary {
    let cutflow = merged.cutflow;
    RunSummary {
        inputs,
        mode: match mode {
            ExecutionMode::Serial => "serial".to_string(),
            ExecutionMode::Parallel => "parallel".to_string(),
        },
        events_seen: cutflow.events_seen,
        events_selected: cutflow.events_selected,
        cutflow: CutflowSummary {
            events_seen: cutflow.events_seen,
            events_selected: cutflow.events_selected,
        },
        plot_values: merged
            .rows
            .iter()
            .map(|row| f64::from(row.lead_muon_pt))
            .collect(),
    }
}

fn load_validated_plan(path: &Path) -> Result<(AnalysisSpec, nano_spec::ResolvedPlan)> {
    let spec = AnalysisSpec::from_path(path).map_err(|error| parse_error(path, error))?;
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, DEFAULT_CATALOGUE_VERSION)
        .map_err(|error| SessionError::Catalogue {
        message: error.to_string(),
    })?;
    let plan =
        nano_spec::validate(&spec, &catalogue).map_err(|errors| validation_error(path, errors))?;
    Ok((spec, plan))
}

fn parse_error(path: &Path, error: ParseError) -> SessionError {
    SessionError::Parse {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn validation_error(path: &Path, errors: Vec<SpecError>) -> SessionError {
    SessionError::Validation {
        path: path.to_path_buf(),
        issues: errors
            .into_iter()
            .map(|error| ValidationIssue {
                message: error.to_string(),
            })
            .collect(),
    }
}

fn branch_summaries(branches: &[nano_core::BranchSpec]) -> Vec<BranchSummary> {
    branches
        .iter()
        .map(|branch| BranchSummary {
            name: branch.name.clone(),
            branch_type: format!("{:?}", branch.branch_type),
        })
        .collect()
}

fn open_root_file(source: &str, insecure: bool) -> Result<RootFile> {
    if is_http_url(source) {
        return open_url(source, insecure);
    }
    RootFile::open(Path::new(source)).map_err(|error| SessionError::Inspect {
        source: source.to_string(),
        message: error.to_string(),
    })
}

#[cfg(feature = "http")]
fn open_url(source: &str, insecure: bool) -> Result<RootFile> {
    let mut options = nano_rootio::HttpSourceOptions::from_env();
    if insecure {
        options = options.insecure(true);
    }
    RootFile::open_url_with_options(source, options).map_err(|error| SessionError::Inspect {
        source: source.to_string(),
        message: error.to_string(),
    })
}

#[cfg(not(feature = "http"))]
fn open_url(source: &str, _insecure: bool) -> Result<RootFile> {
    Err(SessionError::Inspect {
        source: source.to_string(),
        message: format!("`{source}` requires HTTP support; rebuild with `--features http`"),
    })
}

fn is_http_url(source: impl AsRef<str>) -> bool {
    let source = source.as_ref();
    source.starts_with("http://") || source.starts_with("https://")
}

fn unique_session_dir(prefix: &str) -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| SessionError::Workflow {
            message: error.to_string(),
        })?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{prefix}-{}-{timestamp}", std::process::id()));
    std::fs::create_dir_all(&root).map_err(|error| SessionError::Workflow {
        message: format!("failed to create `{}`: {error}", root.display()),
    })?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use nano_core::{BranchSchema, BranchSpec, BranchType};
    use nano_io::read_events;
    use nano_io::writer::{write_events, OutputBranch};

    use super::*;

    #[test]
    fn open_spec_muon_toml_returns_expected_summary() {
        let summary = open_spec(muon_spec_path()).expect("muon spec validates");

        assert_eq!(summary.analysis_name, "muon_demo");
        assert_eq!(summary.objects[0].name, "good_muon");
        assert_eq!(summary.regions, vec!["signal"]);
        assert_eq!(summary.outputs, vec!["n_good_muon", "lead_muon_pt"]);
        assert_eq!(
            summary
                .read_branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            vec!["nMuon", "Muon_eta", "Muon_pt"]
        );
    }

    #[test]
    fn codegen_source_for_muon_is_non_empty() {
        let source = codegen_source(muon_spec_path()).expect("codegen succeeds");

        assert!(source.contains("GeneratedProducer"));
        assert!(source.contains("lead_muon_pt"));
    }

    #[test]
    fn inspect_root_lists_synthetic_tree() {
        let fixture = Fixture::new("inspect-root");
        let input = fixture.path("input.root");
        write_synthetic_input(&input, vec![vec![(31.0, 0.1)], vec![(29.0, 0.2)]]);

        let inspection = inspect_root(&input.display().to_string(), false).expect("inspect works");

        let events = inspection
            .trees
            .iter()
            .find(|tree| tree.name == "Events")
            .expect("Events tree is listed");
        assert_eq!(events.entries, 2);
        assert!(events
            .branches
            .iter()
            .any(|branch| branch.name == "Muon_pt"));
    }

    #[test]
    fn run_muon_dag_over_synthetic_root_has_sane_cutflow() {
        let fixture = Fixture::new("run-dag");
        let input = fixture.path("input.root");
        write_synthetic_input(
            &input,
            vec![
                vec![(31.0, 0.1)],
                vec![(29.0, 0.2)],
                vec![(45.0, -0.3), (40.0, 2.0)],
            ],
        );

        let summary = run_muon_dag([&input], false).expect("workflow runs");

        assert_eq!(summary.events_seen, 3);
        assert_eq!(summary.events_selected, 2);
        assert_eq!(summary.plot_values, vec![31.0, 45.0]);
    }

    fn muon_spec_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nano-spec/examples/muon.toml")
    }

    fn input_schema() -> BranchSchema {
        BranchSchema::new([
            BranchSpec::new("nMuon", BranchType::U32),
            BranchSpec::new("Muon_pt", BranchType::VecF32),
            BranchSpec::new("Muon_eta", BranchType::VecF32),
        ])
        .expect("schema is valid")
    }

    fn write_synthetic_input(path: &Path, muons: Vec<Vec<(f32, f32)>>) {
        let n_events = muons.len();
        let n_muon = muons
            .iter()
            .map(|event_muons| event_muons.len() as u32)
            .collect::<Vec<_>>();
        let muon_pt = muons
            .iter()
            .map(|event_muons| event_muons.iter().map(|(pt, _)| *pt).collect())
            .collect::<Vec<Vec<_>>>();
        let muon_eta = muons
            .iter()
            .map(|event_muons| event_muons.iter().map(|(_, eta)| *eta).collect())
            .collect::<Vec<Vec<_>>>();

        write_events(
            path,
            &[
                OutputBranch::u32("nMuon", n_muon),
                OutputBranch::vec_f32("Muon_pt", muon_pt),
                OutputBranch::vec_f32("Muon_eta", muon_eta),
            ],
        )
        .expect("synthetic input writes");
        assert_eq!(
            read_events(path, input_schema())
                .expect("synthetic input reads")
                .len(),
            n_events
        );
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after epoch")
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("nano-ui-{}-{timestamp}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).expect("fixture dir is created");
            Self { root }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.root.join(name)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
