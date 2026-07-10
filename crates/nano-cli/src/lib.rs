use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use nano_core::BranchType;
use nano_io::writer::{write_events, OutputBranch};
use nano_review::{
    repair_spec, semantic_diff, suggest_repairs, RepairOutcome, RepairSuggestion, SemanticDiff,
};
use nano_rootio::RootFile;
use nano_spec::certificate::PlanCertificate;
use nano_spec::codegen;
use nano_spec::interpret::{interpret, interpret_union, InterpretError, OutputRow, Value};
use nano_spec::{AnalysisSpec, Catalogue, Expr, OutputDType, OutputDef, ParseError, SpecError};
use nano_validate::{compare_root_files, CompareOptions, ComparisonReport, FloatTolerance};
use nano_workflow::{
    plan_workflow_with_kernel_id, resolve_eos_dataset_files, EosResolveOptions, ExecutionMode,
    Executor, KernelBinding, KernelRegistry, SourceList,
};
use serde::{Deserialize, Serialize};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const NANOV12_CATALOGUE: &str = include_str!("../../../configs/branches/nanov12.yaml");
const NANOV15_CATALOGUE: &str = include_str!("../../../configs/branches/nanov15.yaml");
const DEFAULT_CATALOGUE_VERSION: CatalogueVersion = CatalogueVersion::V9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogueVersion {
    V9,
    V12,
    V15,
}

impl Default for CatalogueVersion {
    fn default() -> Self {
        DEFAULT_CATALOGUE_VERSION
    }
}

impl CatalogueVersion {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "v9" | "9" => Some(Self::V9),
            "v12" | "12" => Some(Self::V12),
            "v15" | "15" => Some(Self::V15),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::V9 => "v9",
            Self::V12 => "v12",
            Self::V15 => "v15",
        }
    }

    fn yaml(self) -> &'static str {
        match self {
            Self::V9 => NANOV9_CATALOGUE,
            Self::V12 => NANOV12_CATALOGUE,
            Self::V15 => NANOV15_CATALOGUE,
        }
    }
}

pub type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOptions {
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
// Keep the public enum shape stable for CLI library users.
#[allow(clippy::large_enum_variant)]
pub enum Output {
    Validate(ValidateReport),
    Branches(BranchesReport),
    Certify(CertifyReport),
    Inspect(InspectReport),
    Compare(ComparisonReport),
    Codegen(CodegenReport),
    Diff(DiffReport),
    Repair(RepairReport),
    Run(RunReport),
    EosSources(EosSourcesReport),
    Campaign(CampaignReport),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidateReport {
    pub status: Status,
    pub spec_path: PathBuf,
    pub catalogue_version: String,
    pub analysis: AnalysisSummary,
    pub read_branches: Vec<BranchReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalysisSummary {
    pub name: String,
    pub year: String,
    pub objects: Vec<ObjectSummary>,
    pub models: Vec<ModelSummary>,
    pub regions: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ObjectSummary {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelSummary {
    pub name: String,
    pub inputs: Vec<String>,
    pub output: String,
    pub batch: String,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BranchesReport {
    pub status: Status,
    pub spec_path: PathBuf,
    pub catalogue_version: String,
    pub models: Vec<ModelSummary>,
    pub branches: Vec<BranchReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CertifyReport {
    pub status: Status,
    pub spec_path: PathBuf,
    pub catalogue_version: String,
    pub certificate: PlanCertificate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchReport {
    pub name: String,
    pub branch_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InspectReport {
    pub status: Status,
    pub file: PathBuf,
    pub trees: Vec<TreeReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TreeReport {
    pub name: String,
    pub entries: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branches: Option<Vec<RootBranchReport>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RootBranchReport {
    pub name: String,
    pub types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodegenReport {
    pub status: Status,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiffReport {
    pub status: Status,
    pub spec_a_path: PathBuf,
    pub spec_b_path: PathBuf,
    pub catalogue_version: String,
    pub diff: SemanticDiff,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepairReport {
    pub status: Status,
    pub spec_path: PathBuf,
    pub catalogue_version: String,
    pub applied: bool,
    pub suggestions: Vec<RepairSuggestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RepairOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowRunOptions {
    pub spec_path: PathBuf,
    pub inputs: Vec<PathBuf>,
    pub output: Option<PathBuf>,
    pub parallel: bool,
    pub kernel: Option<String>,
    pub interpret: bool,
    pub max_events: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunReport {
    pub status: Status,
    pub spec: PathBuf,
    pub catalogue_version: String,
    pub inputs: Vec<PathBuf>,
    pub mode: String,
    pub kernel: String,
    pub events_seen: u64,
    pub events_selected: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EosSourcesReport {
    pub status: Status,
    pub sample_config: PathBuf,
    pub store_root: PathBuf,
    pub output: PathBuf,
    pub max_files_per_dataset: usize,
    pub max_depth: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x509_proxy: Option<PathBuf>,
    pub samples: Vec<EosSampleReport>,
    pub files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EosSampleReport {
    pub name: String,
    pub datasets: usize,
    pub files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CampaignReport {
    pub status: Status,
    pub campaign_path: PathBuf,
    pub name: String,
    pub analysis_spec: PathBuf,
    pub catalogue_version: String,
    pub purpose: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub demo: Option<CampaignDemoReport>,
    pub sample_slices: Vec<CampaignSampleSliceReport>,
    pub runs: Vec<CampaignRunReport>,
    pub gates: Vec<CampaignGateReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CampaignDemoReport {
    pub objective: String,
    pub thesis_points: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CampaignSampleSliceReport {
    pub name: String,
    pub role: String,
    pub sample_config: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_list: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_files_per_dataset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_events: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CampaignRunReport {
    pub name: String,
    pub spec: PathBuf,
    pub input_list: PathBuf,
    pub output: PathBuf,
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_events: Option<u64>,
    pub omit_channel_index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CampaignGateReport {
    pub name: String,
    pub kind: String,
    pub status: String,
    pub scope: String,
    pub required: bool,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_spec: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_entries: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_entries: Option<i64>,
    pub check_status: String,
    pub stochastic_branches: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CliError {
    pub status: ErrorStatus,
    pub kind: ErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub validation_errors: Vec<ValidationErrorReport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStatus {
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Usage,
    Parse,
    Catalogue,
    Validation,
    Compare,
    Certificate,
    Codegen,
    Inspect,
    Interpret,
    Kernel,
    Workflow,
}

#[derive(Debug, Deserialize)]
struct RawCampaignSpec {
    campaign: RawCampaignMeta,
    #[serde(default)]
    demo: Option<RawCampaignDemo>,
    #[serde(default, rename = "sample_slice")]
    sample_slices: Vec<RawCampaignSampleSlice>,
    #[serde(default, rename = "run")]
    runs: Vec<RawCampaignRun>,
    #[serde(default, rename = "gate")]
    gates: Vec<RawCampaignGate>,
}

#[derive(Debug, Deserialize)]
struct RawCampaignMeta {
    name: String,
    analysis_spec: PathBuf,
    catalogue_version: String,
    purpose: String,
}

#[derive(Debug, Deserialize)]
struct RawCampaignDemo {
    objective: String,
    #[serde(default)]
    thesis_points: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawCampaignSampleSlice {
    name: String,
    role: String,
    sample_config: PathBuf,
    #[serde(default)]
    source_list: Option<PathBuf>,
    #[serde(default)]
    max_files_per_dataset: Option<usize>,
    #[serde(default)]
    max_events: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawCampaignRun {
    name: String,
    spec: PathBuf,
    input_list: PathBuf,
    output: PathBuf,
    mode: String,
    #[serde(default)]
    max_events: Option<u64>,
    #[serde(default)]
    omit_channel_index: bool,
}

#[derive(Debug, Deserialize)]
struct RawCampaignGate {
    name: String,
    kind: String,
    status: String,
    scope: String,
    #[serde(default = "default_true")]
    required: bool,
    description: String,
    #[serde(default)]
    reference: Option<PathBuf>,
    #[serde(default)]
    candidate: Option<PathBuf>,
    #[serde(default)]
    validation_spec: Option<PathBuf>,
    #[serde(default)]
    artifact: Option<PathBuf>,
    #[serde(default)]
    tree: Option<String>,
    #[serde(default)]
    expected_entries: Option<i64>,
    #[serde(default)]
    stochastic_branches: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidationErrorReport {
    pub kind: ValidationErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationErrorKind {
    MissingBranch,
    UnsupportedBranchType,
    WrongBranchType,
    MissingUnit,
    UnitMismatch,
    UndefinedObject,
    UndefinedBatch,
    ModelOutputCollision,
    InvalidModel,
    InvalidProvider,
    InvalidExpression,
    InvalidReadSchema,
}

pub fn run<I, S>(args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let parsed = ParsedArgs::parse(&args)?;
    match parsed.command {
        Command::Validate {
            spec,
            catalogue_version,
        } => validate_command(&spec, catalogue_version),
        Command::Branches {
            spec,
            catalogue_version,
        } => branches_command(&spec, catalogue_version),
        Command::Certify {
            spec,
            catalogue_version,
        } => certify_command(&spec, catalogue_version),
        Command::Inspect { source, insecure } => inspect_command(&source, insecure),
        Command::Compare(options) => compare_command(options).map(Output::Compare),
        Command::Codegen {
            spec,
            catalogue_version,
        } => codegen_command(&spec, catalogue_version),
        Command::Diff {
            spec_a,
            spec_b,
            catalogue_version,
        } => diff_command(&spec_a, &spec_b, catalogue_version),
        Command::Repair {
            spec,
            apply,
            catalogue_version,
        } => repair_command(&spec, apply, catalogue_version),
        Command::Run(options) => run_workflow_with_catalogue_inner(
            options.workflow,
            options.catalogue_version,
            options.omit_channel_index,
        )
        .map(Output::Run),
        Command::EosSources(options) => eos_sources_command(options).map(Output::EosSources),
        Command::Campaign { campaign } => campaign_command(&campaign).map(Output::Campaign),
    }
}

pub fn render_text(output: &Output) -> String {
    match output {
        Output::Validate(report) => {
            let objects = report
                .analysis
                .objects
                .iter()
                .map(|object| format!("{}:{}", object.name, object.source))
                .collect::<Vec<_>>()
                .join(", ");
            let regions = report.analysis.regions.join(", ");
            let outputs = report.analysis.outputs.join(", ");
            let models = format_models(&report.analysis.models);
            format!(
                "OK validate {}\nanalysis: {} ({})\ncatalogue: {}\nobjects: {}\nmodels: {}\nregions: {}\noutputs: {}\nread_branches: {}",
                report.spec_path.display(),
                report.analysis.name,
                report.analysis.year,
                report.catalogue_version,
                objects,
                models,
                regions,
                outputs,
                format_branches(&report.read_branches)
            )
        }
        Output::Branches(report) => {
            let mut lines = report
                .branches
                .iter()
                .map(|branch| format!("{} {}", branch.name, branch.branch_type))
                .collect::<Vec<_>>();
            if !report.models.is_empty() {
                lines.push(format!("models: {}", format_models(&report.models)));
            }
            lines.join("\n")
        }
        Output::Certify(report) => {
            serde_json::to_string_pretty(&report.certificate).expect("serialize certificate")
        }
        Output::Inspect(report) => {
            let mut lines = Vec::new();
            for tree in &report.trees {
                lines.push(format!("TTree {} entries={}", tree.name, tree.entries));
                if let Some(branches) = &tree.branches {
                    for branch in branches {
                        lines.push(format!("  {} {}", branch.name, branch.types.join("|")));
                    }
                }
            }
            lines.join("\n")
        }
        Output::Compare(report) => report.summary(),
        Output::Codegen(report) => report.source.clone(),
        Output::Diff(report) => {
            if !report.diff.ok {
                return format!(
                    "semantic diff unavailable\n{}: {}\n{}: {}",
                    report.spec_a_path.display(),
                    format_validation_state(&report.diff.validation.a),
                    report.spec_b_path.display(),
                    format_validation_state(&report.diff.validation.b)
                );
            }
            let summary = if report.diff.summary.is_empty() {
                "(no semantic changes)".to_string()
            } else {
                report.diff.summary.join("\n")
            };
            format!(
                "OK diff {} {}\ncatalogue: {}\n{}",
                report.spec_a_path.display(),
                report.spec_b_path.display(),
                report.catalogue_version,
                summary
            )
        }
        Output::Repair(report) => {
            if let Some(outcome) = &report.outcome {
                return format!(
                    "OK repair {} converged={}\napplied: {}\nremaining_errors: {}",
                    report.spec_path.display(),
                    outcome.converged,
                    outcome
                        .applied
                        .iter()
                        .map(|repair| format!("{} -> {}", repair.error_message, repair.replacement))
                        .collect::<Vec<_>>()
                        .join(", "),
                    if outcome.remaining_errors.is_empty() {
                        "(none)".to_string()
                    } else {
                        outcome.remaining_errors.join("; ")
                    }
                );
            }
            let suggestions = if report.suggestions.is_empty() {
                "(none)".to_string()
            } else {
                report
                    .suggestions
                    .iter()
                    .map(|suggestion| {
                        format!(
                            "{}: {} replacement={} confidence={:.2}",
                            suggestion.error_message,
                            suggestion.suggestion,
                            suggestion.replacement.as_deref().unwrap_or("(none)"),
                            suggestion.confidence
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            format!(
                "OK repair {} suggestions\n{}",
                report.spec_path.display(),
                suggestions
            )
        }
        Output::Run(report) => format!(
            "OK run {}\ncatalogue: {}\ninputs: {}\nmode: {}\nkernel: {}\nevents_seen: {}\nevents_selected: {}\noutput: {}\nmanifest: {}",
            report.spec.display(),
            report.catalogue_version,
            report
                .inputs
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            report.mode,
            report.kernel,
            report.events_seen,
            report.events_selected,
            report
                .output
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "(not written)".to_string()),
            report
                .manifest
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "(not written)".to_string())
        ),
        Output::EosSources(report) => {
            let samples = report
                .samples
                .iter()
                .map(|sample| {
                    format!(
                        "{}: {} datasets, {} files",
                        sample.name, sample.datasets, sample.files
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "OK eos-sources {}\nstore_root: {}\noutput: {}\nmax_files_per_dataset: {}\nfiles: {}\nsamples: {}",
                report.sample_config.display(),
                report.store_root.display(),
                report.output.display(),
                report.max_files_per_dataset,
                report.files,
                samples
            )
        }
        Output::Campaign(report) => {
            let samples = report
                .sample_slices
                .iter()
                .map(|sample| format!("{}({})", sample.name, sample.role))
                .collect::<Vec<_>>()
                .join(", ");
            let runs = report
                .runs
                .iter()
                .map(|run| format!("{}:{}->{}", run.name, run.mode, run.output.display()))
                .collect::<Vec<_>>()
                .join(", ");
            let gates = report
                .gates
                .iter()
                .map(|gate| {
                    format!(
                        "{}:{}:{}:{}",
                        gate.name, gate.kind, gate.status, gate.check_status
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let demo = report
                .demo
                .as_ref()
                .map(|demo| demo.objective.as_str())
                .unwrap_or("(none)");
            format!(
                "OK campaign {}\ncampaign: {}\nanalysis_spec: {}\ncatalogue: {}\npurpose: {}\nsamples: {}\nruns: {}\ngates: {}\ndemo: {}",
                report.campaign_path.display(),
                report.name,
                report.analysis_spec.display(),
                report.catalogue_version,
                report.purpose,
                samples,
                runs,
                gates,
                demo
            )
        }
    }
}

pub fn output_success(output: &Output) -> bool {
    !matches!(
        output,
        Output::Compare(report) if !report.passed()
    )
}

pub fn render_json_output(output: &Output) -> std::result::Result<String, serde_json::Error> {
    serde_json::to_string_pretty(output)
}

pub fn render_json_error(error: &CliError) -> std::result::Result<String, serde_json::Error> {
    serde_json::to_string_pretty(error)
}

pub fn render_text_error(error: &CliError) -> String {
    if error.validation_errors.is_empty() {
        return format!("{}: {}", error.kind, error.message);
    }

    let mut lines = vec![format!("{}: {}", error.kind, error.message)];
    lines.extend(
        error
            .validation_errors
            .iter()
            .map(|validation_error| format!("  - {}", validation_error.message)),
    );
    lines.join("\n")
}

fn validate_command(spec_path: &Path, catalogue_version: CatalogueVersion) -> Result<Output> {
    let (spec, plan) = load_validated_plan(spec_path, catalogue_version)?;
    Ok(Output::Validate(ValidateReport {
        status: Status::Ok,
        spec_path: spec_path.to_path_buf(),
        catalogue_version: catalogue_version.as_str().to_string(),
        analysis: analysis_summary(&spec),
        read_branches: branch_reports(plan.read_branches.specs()),
    }))
}

fn branches_command(spec_path: &Path, catalogue_version: CatalogueVersion) -> Result<Output> {
    let (_, plan) = load_validated_plan(spec_path, catalogue_version)?;
    Ok(Output::Branches(BranchesReport {
        status: Status::Ok,
        spec_path: spec_path.to_path_buf(),
        catalogue_version: catalogue_version.as_str().to_string(),
        models: analysis_summary(&plan.spec).models,
        branches: branch_reports(plan.read_branches.specs()),
    }))
}

fn certify_command(spec_path: &Path, catalogue_version: CatalogueVersion) -> Result<Output> {
    let (_, plan) = load_validated_plan(spec_path, catalogue_version)?;
    let certificate = nano_spec::certificate::try_certify(&plan).map_err(|error| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Certificate,
        message: error.to_string(),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: Vec::new(),
    })?;
    Ok(Output::Certify(CertifyReport {
        status: Status::Ok,
        spec_path: spec_path.to_path_buf(),
        catalogue_version: catalogue_version.as_str().to_string(),
        certificate,
    }))
}

fn codegen_command(spec_path: &Path, catalogue_version: CatalogueVersion) -> Result<Output> {
    let (_, plan) = load_validated_plan(spec_path, catalogue_version)?;
    let source = codegen::generate_producer_source(&plan).map_err(|error| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Codegen,
        message: error.to_string(),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: Vec::new(),
    })?;
    Ok(Output::Codegen(CodegenReport {
        status: Status::Ok,
        source,
    }))
}

fn diff_command(
    spec_a_path: &Path,
    spec_b_path: &Path,
    catalogue_version: CatalogueVersion,
) -> Result<Output> {
    let spec_a_text = read_spec_text(spec_a_path)?;
    let spec_b_text = read_spec_text(spec_b_path)?;
    let catalogue = load_catalogue(catalogue_version, None)?;
    Ok(Output::Diff(DiffReport {
        status: Status::Ok,
        spec_a_path: spec_a_path.to_path_buf(),
        spec_b_path: spec_b_path.to_path_buf(),
        catalogue_version: catalogue_version.as_str().to_string(),
        diff: semantic_diff(&spec_a_text, &spec_b_text, &catalogue),
    }))
}

fn repair_command(
    spec_path: &Path,
    apply: bool,
    catalogue_version: CatalogueVersion,
) -> Result<Output> {
    let spec_text = read_spec_text(spec_path)?;
    let catalogue = load_catalogue(catalogue_version, Some(spec_path))?;
    if apply {
        let outcome = repair_spec(&spec_text, &catalogue, true);
        if outcome.final_spec_text != spec_text {
            fs::write(spec_path, &outcome.final_spec_text).map_err(|source| CliError {
                status: ErrorStatus::Error,
                kind: ErrorKind::Workflow,
                message: format!(
                    "failed to write repaired spec `{}`: {source}",
                    spec_path.display()
                ),
                spec_path: Some(spec_path.to_path_buf()),
                validation_errors: Vec::new(),
            })?;
        }
        Ok(Output::Repair(RepairReport {
            status: Status::Ok,
            spec_path: spec_path.to_path_buf(),
            catalogue_version: catalogue_version.as_str().to_string(),
            applied: true,
            suggestions: Vec::new(),
            outcome: Some(outcome),
        }))
    } else {
        Ok(Output::Repair(RepairReport {
            status: Status::Ok,
            spec_path: spec_path.to_path_buf(),
            catalogue_version: catalogue_version.as_str().to_string(),
            applied: false,
            suggestions: suggest_repairs(&spec_text, &catalogue),
            outcome: None,
        }))
    }
}

pub fn run_workflow(options: WorkflowRunOptions) -> Result<RunReport> {
    run_workflow_with_catalogue(options, DEFAULT_CATALOGUE_VERSION)
}

fn run_workflow_with_catalogue(
    options: WorkflowRunOptions,
    catalogue_version: CatalogueVersion,
) -> Result<RunReport> {
    run_workflow_with_catalogue_inner(options, catalogue_version, false)
}

fn run_workflow_with_catalogue_inner(
    options: WorkflowRunOptions,
    catalogue_version: CatalogueVersion,
    omit_channel_index: bool,
) -> Result<RunReport> {
    if options.inputs.is_empty() {
        return Err(usage_error("`nano run` needs at least one input"));
    }

    if options.interpret {
        return run_interpreted(options, catalogue_version, omit_channel_index);
    }
    if options.max_events.is_some() {
        return Err(CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Usage,
            message: "`nano run --max-events` currently requires --interpret".to_string(),
            spec_path: Some(options.spec_path),
            validation_errors: Vec::new(),
        });
    }

    let (spec, plan) = load_validated_plan(&options.spec_path, catalogue_version)?;
    let registry = KernelRegistry::with_muon();
    let requested_kernel = options.kernel.clone().unwrap_or_else(|| spec.name.clone());
    let kernel_id =
        resolve_kernel_id(&registry, &requested_kernel, &spec.name, &options.spec_path)?;
    let binding = registry.get(&kernel_id).map_err(|error| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Kernel,
        message: error.to_string(),
        spec_path: Some(options.spec_path.clone()),
        validation_errors: Vec::new(),
    })?;
    validate_kernel_compatibility(&options.spec_path, &spec, &plan, binding)?;

    let output_path = options.output.unwrap_or_else(|| default_output_path(&spec));
    let cache_dir = cache_dir_for_output(&output_path);
    let kernel = binding.kernel.clone();
    let workflow = plan_workflow_with_kernel_id(
        options.inputs.iter(),
        plan.read_branches,
        10_000,
        &cache_dir,
        &output_path,
        move |event| kernel(event),
        binding.id.clone(),
    )
    .map_err(|error| workflow_error(&options.spec_path, error))?;

    let mode = if options.parallel {
        ExecutionMode::Parallel
    } else {
        ExecutionMode::Serial
    };
    let report = Executor::new()
        .run(&workflow, mode)
        .map_err(|error| workflow_error(&options.spec_path, error))?;
    let cutflow = report.merged.cutflow;

    Ok(RunReport {
        status: Status::Ok,
        spec: options.spec_path,
        catalogue_version: catalogue_version.as_str().to_string(),
        inputs: options.inputs,
        mode: "compiled".to_string(),
        kernel: workflow.kernel_id,
        events_seen: cutflow.events_seen,
        events_selected: cutflow.events_selected,
        output: Some(output_path.clone()),
        manifest: Some(manifest_path_for_output(&output_path)),
    })
}

fn run_interpreted(
    options: WorkflowRunOptions,
    catalogue_version: CatalogueVersion,
    omit_channel_index: bool,
) -> Result<RunReport> {
    let (_, plan) = load_validated_plan(&options.spec_path, catalogue_version)?;
    if !plan.spec.models.is_empty() {
        return Err(interpret_cli_error(
            &options.spec_path,
            InterpretError::Unsupported(
                "models not yet interpreted; use the compiled path".to_string(),
            ),
        ));
    }
    if options.parallel {
        return Err(CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Usage,
            message: "`nano run --interpret` does not support --parallel".to_string(),
            spec_path: Some(options.spec_path),
            validation_errors: Vec::new(),
        });
    }
    if options.kernel.is_some() {
        return Err(CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Usage,
            message: "`nano run --interpret` does not use --kernel".to_string(),
            spec_path: Some(options.spec_path),
            validation_errors: Vec::new(),
        });
    }

    let output_defs = interpreted_output_defs(&plan).map_err(|message| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Interpret,
        message,
        spec_path: Some(options.spec_path.clone()),
        validation_errors: Vec::new(),
    })?;
    let output_names = output_defs
        .iter()
        .map(|output| output.name.clone())
        .collect::<Vec<_>>();
    let channel_indices = plan
        .spec
        .channels
        .iter()
        .enumerate()
        .map(|(index, channel)| (channel.name.clone(), index as u32))
        .collect::<BTreeMap<_, _>>();
    let mut rows = Vec::new();
    let mut selected_channel_indices = Vec::new();
    let mut events_seen = 0_u64;
    let mut events_selected = 0_u64;

    'inputs: for input in &options.inputs {
        if options
            .max_events
            .is_some_and(|max_events| events_seen >= max_events)
        {
            break;
        }
        let events =
            nano_io::events_chunked(input, &plan.read_branches, 10_000).map_err(|error| {
                CliError {
                    status: ErrorStatus::Error,
                    kind: ErrorKind::Workflow,
                    message: error.to_string(),
                    spec_path: Some(options.spec_path.clone()),
                    validation_errors: Vec::new(),
                }
            })?;
        for event in events {
            if options
                .max_events
                .is_some_and(|max_events| events_seen >= max_events)
            {
                break 'inputs;
            }
            let event = event.map_err(|error| CliError {
                status: ErrorStatus::Error,
                kind: ErrorKind::Workflow,
                message: error.to_string(),
                spec_path: Some(options.spec_path.clone()),
                validation_errors: Vec::new(),
            })?;
            events_seen += 1;
            if plan.spec.channels.is_empty() {
                if let Some(row) = interpret(&plan, &event)
                    .map_err(|error| interpret_cli_error(&options.spec_path, error))?
                {
                    validate_row_shape(&output_names, &row).map_err(|message| CliError {
                        status: ErrorStatus::Error,
                        kind: ErrorKind::Interpret,
                        message,
                        spec_path: Some(options.spec_path.clone()),
                        validation_errors: Vec::new(),
                    })?;
                    events_selected += 1;
                    rows.push(row);
                }
            } else {
                for channel_row in interpret_union(&plan, &event)
                    .map_err(|error| interpret_cli_error(&options.spec_path, error))?
                {
                    validate_row_shape(&output_names, &channel_row.row).map_err(|message| {
                        CliError {
                            status: ErrorStatus::Error,
                            kind: ErrorKind::Interpret,
                            message,
                            spec_path: Some(options.spec_path.clone()),
                            validation_errors: Vec::new(),
                        }
                    })?;
                    let Some(channel_index) = channel_indices.get(&channel_row.channel) else {
                        return Err(CliError {
                            status: ErrorStatus::Error,
                            kind: ErrorKind::Interpret,
                            message: format!(
                                "interpreter returned unknown channel `{}`",
                                channel_row.channel
                            ),
                            spec_path: Some(options.spec_path.clone()),
                            validation_errors: Vec::new(),
                        });
                    };
                    selected_channel_indices.push(*channel_index);
                    events_selected += 1;
                    rows.push(channel_row.row);
                }
            }
        }
    }

    if let Some(output_path) = &options.output {
        let mut branches = output_branches(output_defs, &rows).map_err(|message| CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Interpret,
            message,
            spec_path: Some(options.spec_path.clone()),
            validation_errors: Vec::new(),
        })?;
        if !omit_channel_index && !plan.spec.channels.is_empty() {
            branches.insert(
                0,
                OutputBranch::u32("channel_index", selected_channel_indices),
            );
        }
        write_events(output_path, &branches).map_err(|error| CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Workflow,
            message: error.to_string(),
            spec_path: Some(options.spec_path.clone()),
            validation_errors: Vec::new(),
        })?;
    }

    Ok(RunReport {
        status: Status::Ok,
        spec: options.spec_path,
        catalogue_version: catalogue_version.as_str().to_string(),
        inputs: options.inputs,
        mode: "interpret".to_string(),
        kernel: "interpret".to_string(),
        events_seen,
        events_selected,
        output: options.output,
        manifest: None,
    })
}

fn eos_sources_command(options: EosSourcesCommandOptions) -> Result<EosSourcesReport> {
    let samples = read_sample_datasets(&options.sample_config)?;
    let resolve_options = EosResolveOptions {
        store_root: options.store_root.clone(),
        max_files_per_dataset: options.max_files_per_dataset,
        max_depth: options.max_depth,
        x509_proxy: options.x509_proxy.clone(),
    };

    let mut lines = Vec::new();
    let mut sample_reports = Vec::new();
    let mut total_files = 0_usize;
    for (sample, datasets) in &samples {
        lines.push(format!("# sample: {sample}"));
        let mut sample_files = 0_usize;
        for dataset in datasets {
            lines.push(format!("# dataset: {dataset}"));
            let resolved =
                resolve_eos_dataset_files(dataset, &resolve_options).map_err(workflow_cli_error)?;
            sample_files += resolved.files.len();
            total_files += resolved.files.len();
            lines.extend(resolved.files.iter().map(|path| path.display().to_string()));
        }
        lines.push(String::new());
        sample_reports.push(EosSampleReport {
            name: sample.clone(),
            datasets: datasets.len(),
            files: sample_files,
        });
    }

    if let Some(parent) = options
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Workflow,
            message: format!(
                "failed to create output directory `{}`: {source}",
                parent.display()
            ),
            spec_path: None,
            validation_errors: Vec::new(),
        })?;
    }
    fs::write(&options.output, lines.join("\n")).map_err(|source| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Workflow,
        message: format!(
            "failed to write EOS source list `{}`: {source}",
            options.output.display()
        ),
        spec_path: None,
        validation_errors: Vec::new(),
    })?;

    Ok(EosSourcesReport {
        status: Status::Ok,
        sample_config: options.sample_config,
        store_root: options.store_root,
        output: options.output,
        max_files_per_dataset: options.max_files_per_dataset,
        max_depth: options.max_depth,
        x509_proxy: options.x509_proxy,
        samples: sample_reports,
        files: total_files,
    })
}

fn read_sample_datasets(path: &Path) -> Result<BTreeMap<String, Vec<String>>> {
    let text = fs::read_to_string(path).map_err(|source| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Parse,
        message: format!(
            "failed to read sample config `{}`: {source}",
            path.display()
        ),
        spec_path: None,
        validation_errors: Vec::new(),
    })?;
    let value = serde_yaml::from_str::<serde_yaml::Value>(&text).map_err(|source| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Parse,
        message: format!(
            "failed to parse sample config `{}` as YAML: {source}",
            path.display()
        ),
        spec_path: None,
        validation_errors: Vec::new(),
    })?;
    let Some(mapping) = value.as_mapping() else {
        return Err(usage_error(format!(
            "sample config `{}` must be a YAML mapping",
            path.display()
        )));
    };

    let mut samples = BTreeMap::new();
    for (key, value) in mapping {
        let Some(name) = key.as_str() else {
            return Err(usage_error(format!(
                "sample config `{}` contains a non-string sample name",
                path.display()
            )));
        };
        let mut datasets = Vec::new();
        collect_dataset_strings(value, &mut datasets);
        datasets.sort();
        datasets.dedup();
        if datasets.is_empty() {
            return Err(usage_error(format!(
                "sample `{name}` in `{}` does not contain any datasets",
                path.display()
            )));
        }
        samples.insert(name.to_string(), datasets);
    }

    if samples.is_empty() {
        return Err(usage_error(format!(
            "sample config `{}` does not contain any samples",
            path.display()
        )));
    }
    Ok(samples)
}

fn collect_dataset_strings(value: &serde_yaml::Value, datasets: &mut Vec<String>) {
    match value {
        serde_yaml::Value::String(value) => {
            let value = value.trim();
            if value.starts_with('/') {
                datasets.push(value.to_string());
            }
        }
        serde_yaml::Value::Sequence(values) => {
            for value in values {
                collect_dataset_strings(value, datasets);
            }
        }
        _ => {}
    }
}

fn campaign_command(path: &Path) -> Result<CampaignReport> {
    let text = fs::read_to_string(path).map_err(|source| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Parse,
        message: format!("failed to read campaign `{}`: {source}", path.display()),
        spec_path: Some(path.to_path_buf()),
        validation_errors: Vec::new(),
    })?;
    let raw = toml::from_str::<RawCampaignSpec>(&text).map_err(|source| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Parse,
        message: format!("failed to parse campaign `{}`: {source}", path.display()),
        spec_path: Some(path.to_path_buf()),
        validation_errors: Vec::new(),
    })?;
    campaign_report_from_raw(path, raw)
}

fn campaign_report_from_raw(path: &Path, raw: RawCampaignSpec) -> Result<CampaignReport> {
    let root = path.parent().unwrap_or_else(|| Path::new(""));
    require_non_empty("campaign.name", &raw.campaign.name)?;
    require_non_empty("campaign.purpose", &raw.campaign.purpose)?;
    let catalogue_version =
        CatalogueVersion::parse(&raw.campaign.catalogue_version).ok_or_else(|| {
            usage_error(format!(
                "campaign `{}` has unsupported catalogue_version `{}`",
                raw.campaign.name, raw.campaign.catalogue_version
            ))
        })?;
    let analysis_spec = resolve_campaign_path(root, raw.campaign.analysis_spec);
    let _analysis = AnalysisSpec::from_path(&analysis_spec)
        .map_err(|error| parse_cli_error(&analysis_spec, error))?;
    load_catalogue(catalogue_version, Some(&analysis_spec))?;

    let demo = raw
        .demo
        .map(|demo| {
            require_non_empty("demo.objective", &demo.objective)?;
            Ok(CampaignDemoReport {
                objective: demo.objective,
                thesis_points: demo.thesis_points,
            })
        })
        .transpose()?;
    let sample_slices = raw
        .sample_slices
        .into_iter()
        .map(|sample| campaign_sample_report(root, sample))
        .collect::<Result<Vec<_>>>()?;
    require_unique_names(
        "sample_slice",
        sample_slices.iter().map(|sample| sample.name.as_str()),
    )?;
    let runs = raw
        .runs
        .into_iter()
        .map(|run| campaign_run_report(root, run))
        .collect::<Result<Vec<_>>>()?;
    require_unique_names("run", runs.iter().map(|run| run.name.as_str()))?;
    let gates = raw
        .gates
        .into_iter()
        .map(|gate| campaign_gate_report(root, gate))
        .collect::<Result<Vec<_>>>()?;
    require_unique_names("gate", gates.iter().map(|gate| gate.name.as_str()))?;

    Ok(CampaignReport {
        status: Status::Ok,
        campaign_path: path.to_path_buf(),
        name: raw.campaign.name,
        analysis_spec,
        catalogue_version: catalogue_version.as_str().to_string(),
        purpose: raw.campaign.purpose,
        demo,
        sample_slices,
        runs,
        gates,
    })
}

fn campaign_sample_report(
    root: &Path,
    raw: RawCampaignSampleSlice,
) -> Result<CampaignSampleSliceReport> {
    require_non_empty("sample_slice.name", &raw.name)?;
    require_non_empty("sample_slice.role", &raw.role)?;
    Ok(CampaignSampleSliceReport {
        name: raw.name,
        role: raw.role,
        sample_config: resolve_campaign_path(root, raw.sample_config),
        source_list: raw
            .source_list
            .map(|path| resolve_campaign_path(root, path)),
        max_files_per_dataset: raw.max_files_per_dataset,
        max_events: raw.max_events,
    })
}

fn campaign_run_report(root: &Path, raw: RawCampaignRun) -> Result<CampaignRunReport> {
    require_non_empty("run.name", &raw.name)?;
    match raw.mode.as_str() {
        "interpret" | "compiled" => {}
        other => {
            return Err(usage_error(format!(
                "run `{}` has unsupported mode `{other}`; expected `interpret` or `compiled`",
                raw.name
            )));
        }
    }
    Ok(CampaignRunReport {
        name: raw.name,
        spec: resolve_campaign_path(root, raw.spec),
        input_list: resolve_campaign_path(root, raw.input_list),
        output: resolve_campaign_path(root, raw.output),
        mode: raw.mode,
        max_events: raw.max_events,
        omit_channel_index: raw.omit_channel_index,
    })
}

fn campaign_gate_report(root: &Path, raw: RawCampaignGate) -> Result<CampaignGateReport> {
    require_non_empty("gate.name", &raw.name)?;
    require_non_empty("gate.scope", &raw.scope)?;
    require_non_empty("gate.description", &raw.description)?;
    match raw.status.as_str() {
        "implemented" | "planned" => {}
        other => {
            return Err(usage_error(format!(
                "gate `{}` has unsupported status `{other}`; expected `implemented` or `planned`",
                raw.name
            )));
        }
    }
    match raw.kind.as_str() {
        "spec_static_validation" | "run_smoke" | "root_compare" | "yield_closure" => {}
        other => {
            return Err(usage_error(format!(
                "gate `{}` has unsupported kind `{other}`",
                raw.name
            )));
        }
    }
    let reference = raw.reference.map(|path| resolve_campaign_path(root, path));
    let candidate = raw.candidate.map(|path| resolve_campaign_path(root, path));
    let validation_spec = raw
        .validation_spec
        .map(|path| resolve_campaign_path(root, path));
    let artifact = raw.artifact.map(|path| resolve_campaign_path(root, path));
    let mut observed_entries = None;
    let mut check_status = if raw.status == "planned" {
        "planned".to_string()
    } else {
        "declared".to_string()
    };

    if raw.status == "implemented" && raw.kind == "root_compare" {
        if reference.is_none() || candidate.is_none() || validation_spec.is_none() {
            return Err(usage_error(format!(
                "implemented root_compare gate `{}` needs reference, candidate, and validation_spec",
                raw.name
            )));
        }
        let validation_spec_path = validation_spec.as_ref().expect("checked above");
        let validation_analysis = AnalysisSpec::from_path(validation_spec_path)
            .map_err(|error| parse_cli_error(validation_spec_path, error))?;
        verify_campaign_stochastic_branches(
            &raw.name,
            &validation_analysis,
            &raw.stochastic_branches,
        )?;
    }
    if raw.status == "implemented" && raw.kind == "yield_closure" {
        let Some(artifact_path) = artifact.as_ref() else {
            return Err(usage_error(format!(
                "implemented yield_closure gate `{}` needs artifact",
                raw.name
            )));
        };
        let Some(expected_entries) = raw.expected_entries else {
            return Err(usage_error(format!(
                "implemented yield_closure gate `{}` needs expected_entries",
                raw.name
            )));
        };
        if expected_entries < 0 {
            return Err(usage_error(format!(
                "yield_closure gate `{}` expected_entries must be >= 0",
                raw.name
            )));
        }
        if artifact_path.exists() {
            let tree_name = raw.tree.as_deref().unwrap_or("Events");
            let entries =
                read_root_tree_entries(artifact_path, tree_name).map_err(|message| CliError {
                    status: ErrorStatus::Error,
                    kind: ErrorKind::Validation,
                    message,
                    spec_path: None,
                    validation_errors: Vec::new(),
                })?;
            observed_entries = Some(entries);
            if entries != expected_entries {
                return Err(CliError {
                    status: ErrorStatus::Error,
                    kind: ErrorKind::Validation,
                    message: format!(
                        "yield_closure gate `{}` expected {} entries in `{}` but observed {}",
                        raw.name,
                        expected_entries,
                        artifact_path.display(),
                        entries
                    ),
                    spec_path: None,
                    validation_errors: Vec::new(),
                });
            }
            check_status = "passed".to_string();
        } else {
            check_status = "not_checked_missing_artifact".to_string();
        }
    }

    Ok(CampaignGateReport {
        name: raw.name,
        kind: raw.kind,
        status: raw.status,
        scope: raw.scope,
        required: raw.required,
        description: raw.description,
        reference,
        candidate,
        validation_spec,
        artifact,
        tree: raw.tree,
        expected_entries: raw.expected_entries,
        observed_entries,
        check_status,
        stochastic_branches: raw.stochastic_branches,
    })
}

fn read_root_tree_entries(path: &Path, tree_name: &str) -> std::result::Result<i64, String> {
    let file = RootFile::open(path).map_err(|error| {
        format!(
            "failed to open campaign artifact `{}`: {error}",
            path.display()
        )
    })?;
    let tree = file.tree(tree_name).map_err(|error| {
        format!(
            "failed to read tree `{tree_name}` from campaign artifact `{}`: {error}",
            path.display()
        )
    })?;
    Ok(tree.entries())
}

fn verify_campaign_stochastic_branches(
    gate_name: &str,
    analysis: &AnalysisSpec,
    stochastic_branches: &[String],
) -> Result<()> {
    let declared = analysis
        .validation
        .as_ref()
        .and_then(|validation| validation.compare.as_ref())
        .ok_or_else(|| {
            usage_error(format!(
                "root_compare gate `{gate_name}` references an analysis spec without [validation.compare]"
            ))
        })?
        .branch_tolerances
        .iter()
        .map(|tolerance| tolerance.branch.as_str())
        .collect::<BTreeSet<_>>();
    let campaign = stochastic_branches
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if declared != campaign {
        return Err(usage_error(format!(
            "root_compare gate `{gate_name}` stochastic_branches must match validation.compare.branch_tolerance branches; campaign={campaign:?} spec={declared:?}"
        )));
    }
    Ok(())
}

fn resolve_campaign_path(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

fn require_non_empty(context: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(usage_error(format!("{context} must not be empty")))
    } else {
        Ok(())
    }
}

fn require_unique_names<'a>(context: &str, names: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(usage_error(format!("duplicate {context} name `{name}`")));
        }
    }
    Ok(())
}

fn inspect_command(source: &str, insecure: bool) -> Result<Output> {
    let root_file = open_root_file_for_inspect(source, insecure).map_err(|error| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Inspect,
        message: error,
        spec_path: None,
        validation_errors: Vec::new(),
    })?;

    let mut trees = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for object in root_file.objects() {
        if object.class() != "TTree" {
            continue;
        }
        // A ROOT file can hold several keys for the same tree name (write
        // cycles, e.g. `Events;1`/`Events;2`); opening by name reads one, so
        // list each tree name once.
        if !seen.insert(object.name().to_string()) {
            continue;
        }
        let tree = root_file.tree(object.name()).map_err(|error| CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Inspect,
            message: error.to_string(),
            spec_path: None,
            validation_errors: Vec::new(),
        })?;
        let branches = if object.name() == "Events" {
            Some(
                tree.branches()
                    .into_iter()
                    .map(|branch| RootBranchReport {
                        name: branch.name,
                        types: branch.types,
                    })
                    .collect(),
            )
        } else {
            None
        };
        trees.push(TreeReport {
            name: object.name().to_string(),
            entries: tree.entries(),
            branches,
        });
    }

    Ok(Output::Inspect(InspectReport {
        status: Status::Ok,
        file: PathBuf::from(source),
        trees,
    }))
}

fn compare_command(options: CompareCommandOptions) -> Result<ComparisonReport> {
    compare_root_files(&options.reference, &options.candidate, &options.options).map_err(|error| {
        CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Compare,
            message: error.to_string(),
            spec_path: None,
            validation_errors: Vec::new(),
        }
    })
}

fn open_root_file_for_inspect(
    source: &str,
    insecure: bool,
) -> std::result::Result<RootFile, String> {
    if is_http_url(source) {
        return open_url_for_inspect(source, insecure);
    }
    RootFile::open(Path::new(source)).map_err(|error| error.to_string())
}

#[cfg(feature = "http")]
fn open_url_for_inspect(source: &str, insecure: bool) -> std::result::Result<RootFile, String> {
    let mut options = nano_rootio::HttpSourceOptions::from_env();
    if insecure {
        options = options.insecure(true);
    }
    RootFile::open_url_with_options(source, options).map_err(|error| error.to_string())
}

#[cfg(not(feature = "http"))]
fn open_url_for_inspect(source: &str, _insecure: bool) -> std::result::Result<RootFile, String> {
    Err(format!(
        "`nano inspect {source}` requires HTTP support; rebuild with `--features http`"
    ))
}

fn is_http_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

fn load_validated_plan(
    spec_path: &Path,
    catalogue_version: CatalogueVersion,
) -> Result<(AnalysisSpec, nano_spec::ResolvedPlan)> {
    let spec =
        AnalysisSpec::from_path(spec_path).map_err(|error| parse_cli_error(spec_path, error))?;
    let catalogue = load_catalogue(catalogue_version, Some(spec_path))?;
    let plan = nano_spec::validate(&spec, &catalogue).map_err(|errors| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Validation,
        message: "spec validation failed".to_string(),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: errors.iter().map(validation_error_report).collect(),
    })?;
    Ok((spec, plan))
}

fn load_catalogue(
    catalogue_version: CatalogueVersion,
    spec_path: Option<&Path>,
) -> Result<Catalogue> {
    Catalogue::from_nanoaod_yaml_str(catalogue_version.yaml(), catalogue_version.as_str()).map_err(
        |error| CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Catalogue,
            message: error.to_string(),
            spec_path: spec_path.map(Path::to_path_buf),
            validation_errors: Vec::new(),
        },
    )
}

fn read_spec_text(spec_path: &Path) -> Result<String> {
    fs::read_to_string(spec_path).map_err(|source| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Parse,
        message: format!("failed to read spec `{}`: {source}", spec_path.display()),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: Vec::new(),
    })
}

fn resolve_kernel_id(
    registry: &KernelRegistry,
    requested_kernel: &str,
    spec_name: &str,
    spec_path: &Path,
) -> Result<String> {
    if registry.get(requested_kernel).is_ok() {
        return Ok(requested_kernel.to_string());
    }

    if requested_kernel.to_ascii_lowercase().starts_with("muon") {
        return Ok("muon".to_string());
    }

    Err(CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Kernel,
        message: format!(
            "no compiled kernel for spec `{spec_name}` (requested `{requested_kernel}`); codegen produces source to compile in - this runtime path uses registered kernels"
        ),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: Vec::new(),
    })
}

fn validate_kernel_compatibility(
    spec_path: &Path,
    spec: &AnalysisSpec,
    plan: &nano_spec::ResolvedPlan,
    binding: &KernelBinding,
) -> Result<()> {
    let expected = sorted_branch_signature(binding.schema.specs());
    let actual = sorted_branch_signature(plan.read_branches.specs());
    if actual != expected {
        return Err(CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Kernel,
            message: format!(
                "spec `{}` is not compatible with registered kernel `{}`: read_branches differ (expected {}, got {})",
                spec.name,
                binding.id,
                expected.join(", "),
                actual.join(", ")
            ),
            spec_path: Some(spec_path.to_path_buf()),
            validation_errors: Vec::new(),
        });
    }

    if binding.id == "muon" {
        let outputs = spec
            .outputs
            .iter()
            .map(|output| output.name.as_str())
            .collect::<Vec<_>>();
        if !same_strings(&outputs, &["lead_muon_pt", "n_good_muon"]) {
            return Err(CliError {
                status: ErrorStatus::Error,
                kind: ErrorKind::Kernel,
                message: format!(
                    "spec `{}` is not compatible with registered kernel `muon`: outputs must be lead_muon_pt and n_good_muon",
                    spec.name
                ),
                spec_path: Some(spec_path.to_path_buf()),
                validation_errors: Vec::new(),
            });
        }
    }

    Ok(())
}

fn sorted_branch_signature(branches: &[nano_core::BranchSpec]) -> Vec<String> {
    let mut signature = branches
        .iter()
        .map(|branch| {
            format!(
                "{}:{:?}:optional={}",
                branch.name, branch.branch_type, branch.optional
            )
        })
        .collect::<Vec<_>>();
    signature.sort();
    signature
}

fn same_strings(left: &[&str], right: &[&str]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

fn default_output_path(spec: &AnalysisSpec) -> PathBuf {
    PathBuf::from(format!("{}.root", spec.name))
}

fn cache_dir_for_output(output_path: &Path) -> PathBuf {
    output_path.with_extension("nano-cache")
}

fn manifest_path_for_output(output_path: &Path) -> PathBuf {
    output_path.with_extension("root.manifest.json")
}

fn interpret_cli_error(spec_path: &Path, error: InterpretError) -> CliError {
    CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Interpret,
        message: error.to_string(),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: Vec::new(),
    }
}

fn interpreted_output_defs(
    plan: &nano_spec::ResolvedPlan,
) -> std::result::Result<&[OutputDef], String> {
    if plan.spec.channels.is_empty() {
        return Ok(&plan.spec.outputs);
    }
    plan.spec
        .channels
        .first()
        .map(|channel| channel.outputs.as_slice())
        .ok_or_else(|| "multi-channel interpreted run has no channels".to_string())
}

fn validate_row_shape(output_names: &[String], row: &OutputRow) -> std::result::Result<(), String> {
    if row.values.len() != output_names.len() {
        return Err(format!(
            "interpreted row has {} fields, expected {}",
            row.values.len(),
            output_names.len()
        ));
    }
    for (index, expected) in output_names.iter().enumerate() {
        let Some((actual, _)) = row.values.get(index) else {
            return Err(format!("interpreted row is missing output `{expected}`"));
        };
        if actual != expected {
            return Err(format!(
                "interpreted row field {} is `{actual}`, expected `{expected}`",
                index + 1
            ));
        }
    }
    Ok(())
}

fn output_branches(
    outputs: &[OutputDef],
    rows: &[OutputRow],
) -> std::result::Result<Vec<OutputBranch>, String> {
    if outputs.is_empty() {
        return Err("interpreted skim needs at least one declared output".to_string());
    }

    outputs
        .iter()
        .enumerate()
        .map(|(index, output)| output_branch(output, rows, index))
        .collect()
}

fn output_branch(
    output: &OutputDef,
    rows: &[OutputRow],
    index: usize,
) -> std::result::Result<OutputBranch, String> {
    let name = output.name.as_str();
    let first_value = rows
        .first()
        .and_then(|row| row.values.get(index))
        .map(|(_, value)| *value)
        .or_else(|| default_output_value(&output.expr));

    match output
        .dtype
        .map(OutputBranchKind::from)
        .or_else(|| first_value.map(OutputBranchKind::from))
    {
        Some(OutputBranchKind::F32) | None => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(value) => value_to_f32(name, value),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::f32(name, values)),
        Some(OutputBranchKind::F64) => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(value) => value_to_f64(name, value),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::f64(name, values)),
        Some(OutputBranchKind::I32) => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(value) => value_to_i32(name, value),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::i32(name, values)),
        Some(OutputBranchKind::U32) => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(value) => value_to_u32(name, value),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::u32(name, values)),
        Some(OutputBranchKind::U64) => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(value) => value_to_u64(name, value),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::u64(name, values)),
        Some(OutputBranchKind::Bool) => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(Value::Bool(value)) => Ok(value),
                Some(other) => Err(format!("output `{name}` changed type to {other:?}")),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::bool(name, values)),
    }
}

#[derive(Debug, Clone, Copy)]
enum OutputBranchKind {
    F32,
    F64,
    I32,
    U32,
    U64,
    Bool,
}

impl From<OutputDType> for OutputBranchKind {
    fn from(dtype: OutputDType) -> Self {
        match dtype {
            OutputDType::F32 => Self::F32,
            OutputDType::F64 => Self::F64,
            OutputDType::I32 => Self::I32,
            OutputDType::U32 => Self::U32,
            OutputDType::U64 => Self::U64,
        }
    }
}

impl From<Value> for OutputBranchKind {
    fn from(value: Value) -> Self {
        match value {
            Value::F64(_) => Self::F32,
            Value::I64(_) => Self::I32,
            Value::U32(_) => Self::U32,
            Value::U64(_) => Self::U64,
            Value::Bool(_) => Self::Bool,
        }
    }
}

fn value_to_f32(name: &str, value: Value) -> std::result::Result<f32, String> {
    match value {
        Value::F64(value) => Ok(value as f32),
        other => Err(format!("output `{name}` changed type to {other:?}")),
    }
}

fn value_to_f64(name: &str, value: Value) -> std::result::Result<f64, String> {
    match value {
        Value::F64(value) => Ok(value),
        Value::I64(value) => Ok(value as f64),
        Value::U32(value) => Ok(f64::from(value)),
        Value::U64(value) => Ok(value as f64),
        other => Err(format!(
            "output `{name}` cannot be written as F64 from {other:?}"
        )),
    }
}

fn value_to_i32(name: &str, value: Value) -> std::result::Result<i32, String> {
    match value {
        Value::I64(value) => i32::try_from(value).map_err(|error| {
            format!("output `{name}` value {value} cannot be written as i32: {error}")
        }),
        Value::U32(value) => i32::try_from(value).map_err(|error| {
            format!("output `{name}` value {value} cannot be written as i32: {error}")
        }),
        Value::U64(value) => i32::try_from(value).map_err(|error| {
            format!("output `{name}` value {value} cannot be written as i32: {error}")
        }),
        Value::F64(value) => f64_to_i32(name, value),
        other => Err(format!(
            "output `{name}` cannot be written as I32 from {other:?}"
        )),
    }
}

fn value_to_u32(name: &str, value: Value) -> std::result::Result<u32, String> {
    match value {
        Value::U32(value) => Ok(value),
        Value::I64(value) => u32::try_from(value).map_err(|error| {
            format!("output `{name}` value {value} cannot be written as u32: {error}")
        }),
        Value::U64(value) => u32::try_from(value).map_err(|error| {
            format!("output `{name}` value {value} cannot be written as u32: {error}")
        }),
        Value::F64(value) => f64_to_u32(name, value),
        other => Err(format!(
            "output `{name}` cannot be written as U32 from {other:?}"
        )),
    }
}

fn value_to_u64(name: &str, value: Value) -> std::result::Result<u64, String> {
    match value {
        Value::U64(value) => Ok(value),
        Value::U32(value) => Ok(u64::from(value)),
        Value::I64(value) => u64::try_from(value).map_err(|error| {
            format!("output `{name}` value {value} cannot be written as u64: {error}")
        }),
        Value::F64(value) => f64_to_u64(name, value),
        other => Err(format!(
            "output `{name}` cannot be written as U64 from {other:?}"
        )),
    }
}

fn f64_to_i32(name: &str, value: f64) -> std::result::Result<i32, String> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(format!(
            "output `{name}` value {value} cannot be written as i32 without truncation"
        ));
    }
    if value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(format!(
            "output `{name}` value {value} is outside i32 range"
        ));
    }
    Ok(value as i32)
}

fn f64_to_u32(name: &str, value: f64) -> std::result::Result<u32, String> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(format!(
            "output `{name}` value {value} cannot be written as u32 without truncation"
        ));
    }
    if value < 0.0 || value > f64::from(u32::MAX) {
        return Err(format!(
            "output `{name}` value {value} is outside u32 range"
        ));
    }
    Ok(value as u32)
}

fn f64_to_u64(name: &str, value: f64) -> std::result::Result<u64, String> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(format!(
            "output `{name}` value {value} cannot be written as u64 without truncation"
        ));
    }
    if value < 0.0 || value > u64::MAX as f64 {
        return Err(format!(
            "output `{name}` value {value} is outside u64 range"
        ));
    }
    Ok(value as u64)
}

fn default_output_value(expr: &Expr) -> Option<Value> {
    match expr {
        Expr::Count(_) => Some(Value::U32(0)),
        Expr::LeadingAttr { .. } => Some(Value::F64(0.0)),
        _ => None,
    }
}

fn workflow_error(spec_path: &Path, error: nano_workflow::WorkflowError) -> CliError {
    CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Workflow,
        message: error.to_string(),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: Vec::new(),
    }
}

fn workflow_cli_error(error: nano_workflow::WorkflowError) -> CliError {
    CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Workflow,
        message: error.to_string(),
        spec_path: None,
        validation_errors: Vec::new(),
    }
}

fn parse_cli_error(spec_path: &Path, error: ParseError) -> CliError {
    CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Parse,
        message: error.to_string(),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: Vec::new(),
    }
}

fn analysis_summary(spec: &AnalysisSpec) -> AnalysisSummary {
    AnalysisSummary {
        name: spec.name.clone(),
        year: format!("{:?}", spec.year),
        objects: spec
            .objects
            .iter()
            .map(|object| ObjectSummary {
                name: object.name.clone(),
                source: object.source.clone(),
            })
            .collect(),
        models: spec
            .models
            .iter()
            .map(|model| ModelSummary {
                name: model.name.clone(),
                inputs: model.inputs.clone(),
                output: model.output.clone(),
                batch: model.batch.clone(),
                provider: format!("{:?}", model.provider.kind),
            })
            .collect(),
        regions: spec
            .regions
            .iter()
            .map(|region| region.name.clone())
            .collect(),
        outputs: spec
            .outputs
            .iter()
            .map(|output| output.name.clone())
            .collect(),
    }
}

fn format_models(models: &[ModelSummary]) -> String {
    if models.is_empty() {
        return "(none)".to_string();
    }
    models
        .iter()
        .map(|model| {
            format!(
                "{}:{} -> {} [{}]",
                model.name, model.batch, model.output, model.provider
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn branch_reports(branches: &[nano_core::BranchSpec]) -> Vec<BranchReport> {
    branches
        .iter()
        .map(|branch| BranchReport {
            name: branch.name.clone(),
            branch_type: branch_type_name(branch.branch_type),
        })
        .collect()
}

fn format_branches(branches: &[BranchReport]) -> String {
    branches
        .iter()
        .map(|branch| format!("{} {}", branch.name, branch.branch_type))
        .collect::<Vec<_>>()
        .join(", ")
}

fn branch_type_name(branch_type: BranchType) -> String {
    format!("{branch_type:?}")
}

fn validation_error_report(error: &SpecError) -> ValidationErrorReport {
    match error {
        SpecError::MissingBranch { context, branch } => ValidationErrorReport {
            kind: ValidationErrorKind::MissingBranch,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: Some(branch.clone()),
            object: None,
            expr: None,
            expected: None,
            actual: None,
            detail: None,
        },
        SpecError::UnsupportedBranchType {
            context,
            branch,
            raw_type,
        } => ValidationErrorReport {
            kind: ValidationErrorKind::UnsupportedBranchType,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: Some(branch.clone()),
            object: None,
            expr: None,
            expected: Some("supported NanoAOD branch type".to_string()),
            actual: Some(raw_type.clone()),
            detail: None,
        },
        SpecError::WrongBranchType {
            context,
            branch,
            expected,
            actual,
        } => ValidationErrorReport {
            kind: ValidationErrorKind::WrongBranchType,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: Some(branch.clone()),
            object: None,
            expr: None,
            expected: Some(expected.clone()),
            actual: Some(branch_type_name(*actual)),
            detail: None,
        },
        SpecError::MissingUnit {
            context,
            expr,
            expected,
        } => ValidationErrorReport {
            kind: ValidationErrorKind::MissingUnit,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: None,
            object: None,
            expr: Some(expr.clone()),
            expected: Some(expected.to_string()),
            actual: Some("dimensionless".to_string()),
            detail: None,
        },
        SpecError::UnitMismatch {
            context,
            expr,
            expected,
            actual,
        } => ValidationErrorReport {
            kind: ValidationErrorKind::UnitMismatch,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: None,
            object: None,
            expr: Some(expr.clone()),
            expected: Some(format!("{expected:?}")),
            actual: Some(actual.to_string()),
            detail: None,
        },
        SpecError::UndefinedObject { context, object } => ValidationErrorReport {
            kind: ValidationErrorKind::UndefinedObject,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: None,
            object: Some(object.clone()),
            expr: None,
            expected: Some("defined object".to_string()),
            actual: None,
            detail: None,
        },
        SpecError::UndefinedBatch { context, batch } => ValidationErrorReport {
            kind: ValidationErrorKind::UndefinedBatch,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: None,
            object: Some(batch.clone()),
            expr: None,
            expected: Some("defined object or collection".to_string()),
            actual: None,
            detail: None,
        },
        SpecError::ModelOutputCollision { context, output } => ValidationErrorReport {
            kind: ValidationErrorKind::ModelOutputCollision,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: Some(output.clone()),
            object: None,
            expr: None,
            expected: Some("fresh model output column".to_string()),
            actual: Some("existing column".to_string()),
            detail: None,
        },
        SpecError::InvalidModel { context, detail } => ValidationErrorReport {
            kind: ValidationErrorKind::InvalidModel,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: None,
            object: None,
            expr: None,
            expected: None,
            actual: None,
            detail: Some(detail.clone()),
        },
        SpecError::InvalidProvider { context, detail } => ValidationErrorReport {
            kind: ValidationErrorKind::InvalidProvider,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: None,
            object: None,
            expr: None,
            expected: None,
            actual: None,
            detail: Some(detail.clone()),
        },
        SpecError::InvalidExpression { context, detail } => ValidationErrorReport {
            kind: ValidationErrorKind::InvalidExpression,
            message: error.to_string(),
            context: Some(context.clone()),
            branch: None,
            object: None,
            expr: None,
            expected: None,
            actual: None,
            detail: Some(detail.clone()),
        },
        SpecError::InvalidReadSchema { detail } => ValidationErrorReport {
            kind: ValidationErrorKind::InvalidReadSchema,
            message: error.to_string(),
            context: None,
            branch: None,
            object: None,
            expr: None,
            expected: None,
            actual: None,
            detail: Some(detail.clone()),
        },
    }
}

struct ParsedArgs {
    command: Command,
}

enum Command {
    Validate {
        spec: PathBuf,
        catalogue_version: CatalogueVersion,
    },
    Branches {
        spec: PathBuf,
        catalogue_version: CatalogueVersion,
    },
    Certify {
        spec: PathBuf,
        catalogue_version: CatalogueVersion,
    },
    Inspect {
        source: String,
        insecure: bool,
    },
    Compare(CompareCommandOptions),
    Codegen {
        spec: PathBuf,
        catalogue_version: CatalogueVersion,
    },
    Diff {
        spec_a: PathBuf,
        spec_b: PathBuf,
        catalogue_version: CatalogueVersion,
    },
    Repair {
        spec: PathBuf,
        apply: bool,
        catalogue_version: CatalogueVersion,
    },
    Run(RunCommandOptions),
    EosSources(EosSourcesCommandOptions),
    Campaign {
        campaign: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct CompareCommandOptions {
    reference: PathBuf,
    candidate: PathBuf,
    options: CompareOptions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunCommandOptions {
    workflow: WorkflowRunOptions,
    catalogue_version: CatalogueVersion,
    omit_channel_index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EosSourcesCommandOptions {
    sample_config: PathBuf,
    output: PathBuf,
    store_root: PathBuf,
    max_files_per_dataset: usize,
    max_depth: usize,
    x509_proxy: Option<PathBuf>,
}

impl ParsedArgs {
    fn parse(args: &[String]) -> Result<Self> {
        let positional = args
            .iter()
            .filter(|arg| arg.as_str() != "--json")
            .cloned()
            .collect::<Vec<_>>();
        let Some(command) = positional.first().map(String::as_str) else {
            return Err(usage_error("missing command"));
        };
        let command = match command {
            "validate" => {
                let (spec, catalogue_version) =
                    one_operand_with_catalogue(command, &positional[1..])?;
                Command::Validate {
                    spec,
                    catalogue_version,
                }
            }
            "branches" => {
                let (spec, catalogue_version) =
                    one_operand_with_catalogue(command, &positional[1..])?;
                Command::Branches {
                    spec,
                    catalogue_version,
                }
            }
            "certify" => {
                let (spec, catalogue_version) =
                    one_operand_with_catalogue(command, &positional[1..])?;
                Command::Certify {
                    spec,
                    catalogue_version,
                }
            }
            "inspect" => parse_inspect_args(&positional[1..])?,
            "compare" => Command::Compare(parse_compare_args(&positional[1..])?),
            "codegen" => {
                let (spec, catalogue_version) =
                    one_operand_with_catalogue(command, &positional[1..])?;
                Command::Codegen {
                    spec,
                    catalogue_version,
                }
            }
            "diff" => {
                let (spec_a, spec_b, catalogue_version) =
                    two_operands_with_catalogue(command, &positional[1..])?;
                Command::Diff {
                    spec_a,
                    spec_b,
                    catalogue_version,
                }
            }
            "repair" => parse_repair_args(&positional[1..])?,
            "run" => Command::Run(parse_run_args(&positional[1..])?),
            "eos-sources" => Command::EosSources(parse_eos_sources_args(&positional[1..])?),
            "campaign" => {
                let campaign = one_operand(command, &positional[1..])?;
                Command::Campaign { campaign }
            }
            _ => return Err(usage_error(format!("unknown command `{command}`"))),
        };
        Ok(Self { command })
    }
}

fn parse_compare_args(args: &[String]) -> Result<CompareCommandOptions> {
    let mut tree = "Events".to_string();
    let mut rtol = 1e-6;
    let mut atol = 1e-6;
    let mut branch_tolerances = BTreeMap::new();
    let mut validation_spec = None;
    let mut saw_tree = false;
    let mut saw_rtol = false;
    let mut saw_atol = false;
    let mut operands = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--tree" => {
                tree = compare_flag_value(args, index, "--tree")?.to_string();
                saw_tree = true;
                index += 2;
            }
            "--rtol" => {
                rtol = compare_flag_value(args, index, "--rtol")?
                    .parse::<f64>()
                    .map_err(|error| {
                        usage_error(format!("invalid `nano compare --rtol`: {error}"))
                    })?;
                saw_rtol = true;
                index += 2;
            }
            "--atol" => {
                atol = compare_flag_value(args, index, "--atol")?
                    .parse::<f64>()
                    .map_err(|error| {
                        usage_error(format!("invalid `nano compare --atol`: {error}"))
                    })?;
                saw_atol = true;
                index += 2;
            }
            "--branch-tolerance" => {
                let (branch, tolerance) =
                    parse_branch_tolerance(compare_flag_value(args, index, "--branch-tolerance")?)?;
                branch_tolerances.insert(branch, tolerance);
                index += 2;
            }
            "--validation-spec" => {
                validation_spec = Some(PathBuf::from(compare_flag_value(
                    args,
                    index,
                    "--validation-spec",
                )?));
                index += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(usage_error(format!("unknown `nano compare` flag `{flag}`")));
            }
            operand => {
                operands.push(PathBuf::from(operand));
                index += 1;
            }
        }
    }

    let [reference, candidate] = operands.as_slice() else {
        return Err(usage_error(
            "`nano compare` needs <reference.root> <candidate.root>",
        ));
    };
    if !rtol.is_finite() || rtol < 0.0 {
        return Err(usage_error("`nano compare --rtol` must be finite and >= 0"));
    }
    if !atol.is_finite() || atol < 0.0 {
        return Err(usage_error("`nano compare --atol` must be finite and >= 0"));
    }
    if let Some(spec_path) = validation_spec {
        apply_compare_validation_spec(
            &spec_path,
            &mut tree,
            &mut rtol,
            &mut atol,
            &mut branch_tolerances,
            CompareValidationCliOverrides {
                tree: saw_tree,
                rtol: saw_rtol,
                atol: saw_atol,
            },
        )?;
    }
    Ok(CompareCommandOptions {
        reference: reference.clone(),
        candidate: candidate.clone(),
        options: CompareOptions {
            tree,
            rtol,
            atol,
            branch_tolerances,
            ..CompareOptions::default()
        },
    })
}

#[derive(Debug, Clone, Copy)]
struct CompareValidationCliOverrides {
    tree: bool,
    rtol: bool,
    atol: bool,
}

fn apply_compare_validation_spec(
    spec_path: &Path,
    tree: &mut String,
    rtol: &mut f64,
    atol: &mut f64,
    branch_tolerances: &mut BTreeMap<String, FloatTolerance>,
    cli_overrides: CompareValidationCliOverrides,
) -> Result<()> {
    let spec =
        AnalysisSpec::from_path(spec_path).map_err(|error| parse_cli_error(spec_path, error))?;
    let Some(compare) = spec
        .validation
        .as_ref()
        .and_then(|validation| validation.compare.as_ref())
    else {
        return Err(usage_error(format!(
            "`nano compare --validation-spec {}` has no [validation.compare] section",
            spec_path.display()
        )));
    };
    if !cli_overrides.tree {
        if let Some(spec_tree) = &compare.tree {
            *tree = spec_tree.clone();
        }
    }
    if !cli_overrides.rtol {
        if let Some(spec_rtol) = compare.rtol {
            *rtol = spec_rtol;
        }
    }
    if !cli_overrides.atol {
        if let Some(spec_atol) = compare.atol {
            *atol = spec_atol;
        }
    }
    for tolerance in &compare.branch_tolerances {
        branch_tolerances
            .entry(tolerance.branch.clone())
            .or_insert(FloatTolerance {
                rtol: tolerance.rtol,
                atol: tolerance.atol,
            });
    }
    Ok(())
}

fn parse_branch_tolerance(value: &str) -> Result<(String, FloatTolerance)> {
    let parts = value.split(':').collect::<Vec<_>>();
    let [branch, rtol, atol] = parts.as_slice() else {
        return Err(usage_error(
            "`nano compare --branch-tolerance` expects <branch>:<rtol>:<atol>",
        ));
    };
    if branch.is_empty() {
        return Err(usage_error(
            "`nano compare --branch-tolerance` needs a non-empty branch name",
        ));
    }
    let rtol = rtol.parse::<f64>().map_err(|error| {
        usage_error(format!(
            "invalid `nano compare --branch-tolerance` rtol: {error}"
        ))
    })?;
    let atol = atol.parse::<f64>().map_err(|error| {
        usage_error(format!(
            "invalid `nano compare --branch-tolerance` atol: {error}"
        ))
    })?;
    if !rtol.is_finite() || rtol < 0.0 {
        return Err(usage_error(
            "`nano compare --branch-tolerance` rtol must be finite and >= 0",
        ));
    }
    if !atol.is_finite() || atol < 0.0 {
        return Err(usage_error(
            "`nano compare --branch-tolerance` atol must be finite and >= 0",
        ));
    }
    Ok((branch.to_string(), FloatTolerance { rtol, atol }))
}

fn compare_flag_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str> {
    let Some(value) = args.get(index + 1) else {
        return Err(usage_error(format!("`nano compare {flag}` needs a value")));
    };
    if value.starts_with("--") {
        return Err(usage_error(format!("`nano compare {flag}` needs a value")));
    }
    Ok(value)
}

pub fn parse_options<I, S>(args: I) -> RunOptions
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    RunOptions {
        json: args.into_iter().any(|arg| arg.as_ref() == "--json"),
    }
}

fn parse_inspect_args(args: &[String]) -> Result<Command> {
    let mut source = None;
    let mut insecure = false;
    for arg in args {
        if arg == "--insecure" {
            insecure = true;
        } else if source.is_none() {
            source = Some(arg.clone());
        } else {
            return Err(usage_error("`nano inspect` accepts one path or URL"));
        }
    }
    let source = source.ok_or_else(|| usage_error("`nano inspect` needs one path or URL"))?;
    Ok(Command::Inspect { source, insecure })
}

fn one_operand(command: &str, args: &[String]) -> Result<PathBuf> {
    let mut operands = Vec::new();
    for arg in args {
        if arg.starts_with("--") {
            return Err(usage_error(format!(
                "unknown `nano {command}` flag `{arg}`"
            )));
        }
        operands.push(PathBuf::from(arg));
    }

    match operands.as_slice() {
        [operand] => Ok(operand.clone()),
        [] => Err(usage_error(format!("`nano {command}` needs one path"))),
        _ => Err(usage_error(format!(
            "`nano {command}` accepts exactly one path"
        ))),
    }
}

fn one_operand_with_catalogue(
    command: &str,
    args: &[String],
) -> Result<(PathBuf, CatalogueVersion)> {
    let mut operands = Vec::new();
    let mut catalogue_version = CatalogueVersion::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--catalogue-version" => {
                let value = command_flag_value(command, args, index, "--catalogue-version")?;
                catalogue_version = parse_catalogue_version(command, value)?;
                index += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(usage_error(format!(
                    "unknown `nano {command}` flag `{flag}`"
                )));
            }
            operand => {
                operands.push(PathBuf::from(operand));
                index += 1;
            }
        }
    }

    match operands.as_slice() {
        [operand] => Ok((operand.clone(), catalogue_version)),
        [] => Err(usage_error(format!("`nano {command}` needs one path"))),
        _ => Err(usage_error(format!(
            "`nano {command}` accepts exactly one path"
        ))),
    }
}

fn two_operands_with_catalogue(
    command: &str,
    args: &[String],
) -> Result<(PathBuf, PathBuf, CatalogueVersion)> {
    let mut operands = Vec::new();
    let mut catalogue_version = CatalogueVersion::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--catalogue-version" => {
                let value = command_flag_value(command, args, index, "--catalogue-version")?;
                catalogue_version = parse_catalogue_version(command, value)?;
                index += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(usage_error(format!(
                    "unknown `nano {command}` flag `{flag}`"
                )));
            }
            operand => {
                operands.push(PathBuf::from(operand));
                index += 1;
            }
        }
    }

    match operands.as_slice() {
        [left, right] => Ok((left.clone(), right.clone(), catalogue_version)),
        [] | [_] => Err(usage_error(format!("`nano {command}` needs two paths"))),
        _ => Err(usage_error(format!(
            "`nano {command}` accepts exactly two paths"
        ))),
    }
}

fn parse_repair_args(args: &[String]) -> Result<Command> {
    let mut apply = false;
    let mut spec = None;
    let mut catalogue_version = CatalogueVersion::default();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--apply" => {
                apply = true;
                index += 1;
            }
            "--catalogue-version" => {
                let value = command_flag_value("repair", args, index, "--catalogue-version")?;
                catalogue_version = parse_catalogue_version("repair", value)?;
                index += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(usage_error(format!("unknown `nano repair` flag `{flag}`")));
            }
            operand => {
                if spec.is_some() {
                    return Err(usage_error(format!(
                        "unexpected `nano repair` argument `{operand}`"
                    )));
                }
                spec = Some(PathBuf::from(operand));
                index += 1;
            }
        }
    }
    let Some(spec) = spec else {
        return Err(usage_error("`nano repair` needs one spec path"));
    };
    Ok(Command::Repair {
        spec,
        apply,
        catalogue_version,
    })
}

fn parse_run_args(args: &[String]) -> Result<RunCommandOptions> {
    let mut spec = None;
    let mut inputs = Vec::new();
    let mut output = None;
    let mut parallel = false;
    let mut kernel = None;
    let mut interpret = false;
    let mut catalogue_version = CatalogueVersion::default();
    let mut max_events = None;
    let mut omit_channel_index = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--interpret" => {
                interpret = true;
                index += 1;
            }
            "--inputs" => {
                let value = flag_value(args, index, "--inputs")?;
                inputs.extend(source_list_from_csv(value)?.into_paths());
                index += 2;
            }
            "--input-list" => {
                let value = flag_value(args, index, "--input-list")?;
                inputs.extend(source_list_from_path(Path::new(value))?.into_paths());
                index += 2;
            }
            "--output" => {
                let value = flag_value(args, index, "--output")?;
                output = Some(PathBuf::from(value));
                index += 2;
            }
            "--parallel" => {
                parallel = true;
                index += 1;
            }
            "--kernel" => {
                let value = flag_value(args, index, "--kernel")?;
                kernel = Some(value.to_string());
                index += 2;
            }
            "--catalogue-version" => {
                let value = flag_value(args, index, "--catalogue-version")?;
                catalogue_version = parse_catalogue_version("run", value)?;
                index += 2;
            }
            "--max-events" => {
                let value = flag_value(args, index, "--max-events")?;
                max_events = Some(parse_positive_u64_flag("run", "--max-events", value)?);
                index += 2;
            }
            "--omit-channel-index" => {
                omit_channel_index = true;
                index += 1;
            }
            flag if flag.starts_with("--") => {
                return Err(usage_error(format!("unknown `nano run` flag `{flag}`")));
            }
            operand => {
                if spec.is_some() {
                    return Err(usage_error(format!(
                        "unexpected `nano run` argument `{operand}`"
                    )));
                }
                spec = Some(PathBuf::from(operand));
                index += 1;
            }
        }
    }

    let Some(spec_path) = spec else {
        return Err(usage_error("`nano run` needs one spec path"));
    };

    if inputs.is_empty() {
        return Err(usage_error(
            "`nano run` needs --inputs <f1,f2,...> or --input-list <file>",
        ));
    }

    Ok(RunCommandOptions {
        workflow: WorkflowRunOptions {
            spec_path,
            inputs,
            output,
            parallel,
            kernel,
            interpret,
            max_events,
        },
        catalogue_version,
        omit_channel_index,
    })
}

fn parse_eos_sources_args(args: &[String]) -> Result<EosSourcesCommandOptions> {
    let mut sample_config = None;
    let mut output = None;
    let mut store_root = PathBuf::from("/eos/cms/store");
    let mut max_files_per_dataset = 1_usize;
    let mut max_depth = 8_usize;
    let mut x509_proxy = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--output" => {
                output = Some(PathBuf::from(command_flag_value(
                    "eos-sources",
                    args,
                    index,
                    "--output",
                )?));
                index += 2;
            }
            "--store-root" => {
                store_root = PathBuf::from(command_flag_value(
                    "eos-sources",
                    args,
                    index,
                    "--store-root",
                )?);
                index += 2;
            }
            "--max-files-per-dataset" => {
                max_files_per_dataset = parse_positive_usize_flag(
                    "eos-sources",
                    "--max-files-per-dataset",
                    command_flag_value("eos-sources", args, index, "--max-files-per-dataset")?,
                )?;
                index += 2;
            }
            "--max-depth" => {
                max_depth = parse_positive_usize_flag(
                    "eos-sources",
                    "--max-depth",
                    command_flag_value("eos-sources", args, index, "--max-depth")?,
                )?;
                index += 2;
            }
            "--x509-proxy" => {
                x509_proxy = Some(PathBuf::from(command_flag_value(
                    "eos-sources",
                    args,
                    index,
                    "--x509-proxy",
                )?));
                index += 2;
            }
            flag if flag.starts_with("--") => {
                return Err(usage_error(format!(
                    "unknown `nano eos-sources` flag `{flag}`"
                )));
            }
            operand => {
                if sample_config.is_some() {
                    return Err(usage_error(format!(
                        "unexpected `nano eos-sources` argument `{operand}`"
                    )));
                }
                sample_config = Some(PathBuf::from(operand));
                index += 1;
            }
        }
    }

    let Some(sample_config) = sample_config else {
        return Err(usage_error("`nano eos-sources` needs one sample YAML path"));
    };
    let Some(output) = output else {
        return Err(usage_error(
            "`nano eos-sources` needs --output <sources.txt>",
        ));
    };

    Ok(EosSourcesCommandOptions {
        sample_config,
        output,
        store_root,
        max_files_per_dataset,
        max_depth,
        x509_proxy,
    })
}

fn parse_catalogue_version(command: &str, value: &str) -> Result<CatalogueVersion> {
    CatalogueVersion::parse(value).ok_or_else(|| {
        usage_error(format!(
            "invalid `nano {command} --catalogue-version {value}`; expected v9, v12, or v15"
        ))
    })
}

fn parse_positive_usize_flag(command: &str, flag: &str, value: &str) -> Result<usize> {
    let parsed = value
        .parse::<usize>()
        .map_err(|error| usage_error(format!("invalid `nano {command} {flag}`: {error}")))?;
    if parsed == 0 {
        return Err(usage_error(format!(
            "`nano {command} {flag}` must be greater than zero"
        )));
    }
    Ok(parsed)
}

fn parse_positive_u64_flag(command: &str, flag: &str, value: &str) -> Result<u64> {
    let parsed = value
        .parse::<u64>()
        .map_err(|error| usage_error(format!("invalid `nano {command} {flag}`: {error}")))?;
    if parsed == 0 {
        return Err(usage_error(format!(
            "`nano {command} {flag}` must be greater than zero"
        )));
    }
    Ok(parsed)
}

fn command_flag_value<'a>(
    command: &str,
    args: &'a [String],
    index: usize,
    flag: &str,
) -> Result<&'a str> {
    let Some(value) = args.get(index + 1) else {
        return Err(usage_error(format!(
            "`nano {command} {flag}` needs a value"
        )));
    };
    if value.starts_with("--") {
        return Err(usage_error(format!(
            "`nano {command} {flag}` needs a value"
        )));
    }
    Ok(value)
}

fn flag_value<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str> {
    let Some(value) = args.get(index + 1) else {
        return Err(usage_error(format!("`nano run {flag}` needs a value")));
    };
    if value.starts_with("--") {
        return Err(usage_error(format!("`nano run {flag}` needs a value")));
    }
    Ok(value)
}

fn usage_error(message: impl Into<String>) -> CliError {
    CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Usage,
        message: message.into(),
        spec_path: None,
        validation_errors: Vec::new(),
    }
}

fn source_list_from_csv(value: &str) -> Result<SourceList> {
    SourceList::from_csv(value)
        .map_err(|error| usage_error(format!("invalid `nano run --inputs`: {error}")))
}

fn source_list_from_path(path: &Path) -> Result<SourceList> {
    SourceList::from_path(path).map_err(|error| {
        usage_error(format!(
            "invalid `nano run --input-list {}`: {error}",
            path.display()
        ))
    })
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

fn format_validation_state(state: &nano_review::ValidationState) -> String {
    if state.valid {
        return "valid".to_string();
    }
    if let Some(error) = &state.parse_error {
        return error.clone();
    }
    state.validation_errors.join("; ")
}
