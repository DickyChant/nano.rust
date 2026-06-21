use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use nano_core::BranchType;
use nano_io::writer::{write_events, OutputBranch};
use nano_review::{
    repair_spec, semantic_diff, suggest_repairs, RepairOutcome, RepairSuggestion, SemanticDiff,
};
use nano_rootio::RootFile;
use nano_spec::codegen;
use nano_spec::interpret::{interpret, InterpretError, OutputRow, Value};
use nano_spec::{AnalysisSpec, Catalogue, Expr, OutputDef, ParseError, SpecError};
use nano_workflow::{
    plan_workflow_with_kernel_id, ExecutionMode, Executor, KernelBinding, KernelRegistry,
};
use serde::Serialize;

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const DEFAULT_CATALOGUE_VERSION: &str = "v9";

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
    Inspect(InspectReport),
    Codegen(CodegenReport),
    Diff(DiffReport),
    Repair(RepairReport),
    Run(RunReport),
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunReport {
    pub status: Status,
    pub spec: PathBuf,
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
    Codegen,
    Inspect,
    Interpret,
    Kernel,
    Workflow,
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
        Command::Validate { spec } => validate_command(&spec),
        Command::Branches { spec } => branches_command(&spec),
        Command::Inspect { source, insecure } => inspect_command(&source, insecure),
        Command::Codegen { spec } => codegen_command(&spec),
        Command::Diff { spec_a, spec_b } => diff_command(&spec_a, &spec_b),
        Command::Repair { spec, apply } => repair_command(&spec, apply),
        Command::Run(options) => run_workflow(options).map(Output::Run),
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
            "OK run {}\ninputs: {}\nmode: {}\nkernel: {}\nevents_seen: {}\nevents_selected: {}\noutput: {}\nmanifest: {}",
            report.spec.display(),
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
    }
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

fn validate_command(spec_path: &Path) -> Result<Output> {
    let (spec, plan) = load_validated_plan(spec_path)?;
    Ok(Output::Validate(ValidateReport {
        status: Status::Ok,
        spec_path: spec_path.to_path_buf(),
        catalogue_version: DEFAULT_CATALOGUE_VERSION.to_string(),
        analysis: analysis_summary(&spec),
        read_branches: branch_reports(plan.read_branches.specs()),
    }))
}

fn branches_command(spec_path: &Path) -> Result<Output> {
    let (_, plan) = load_validated_plan(spec_path)?;
    Ok(Output::Branches(BranchesReport {
        status: Status::Ok,
        spec_path: spec_path.to_path_buf(),
        catalogue_version: DEFAULT_CATALOGUE_VERSION.to_string(),
        models: analysis_summary(&plan.spec).models,
        branches: branch_reports(plan.read_branches.specs()),
    }))
}

fn codegen_command(spec_path: &Path) -> Result<Output> {
    let (_, plan) = load_validated_plan(spec_path)?;
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

fn diff_command(spec_a_path: &Path, spec_b_path: &Path) -> Result<Output> {
    let spec_a_text = read_spec_text(spec_a_path)?;
    let spec_b_text = read_spec_text(spec_b_path)?;
    let catalogue = load_default_catalogue(None)?;
    Ok(Output::Diff(DiffReport {
        status: Status::Ok,
        spec_a_path: spec_a_path.to_path_buf(),
        spec_b_path: spec_b_path.to_path_buf(),
        catalogue_version: DEFAULT_CATALOGUE_VERSION.to_string(),
        diff: semantic_diff(&spec_a_text, &spec_b_text, &catalogue),
    }))
}

fn repair_command(spec_path: &Path, apply: bool) -> Result<Output> {
    let spec_text = read_spec_text(spec_path)?;
    let catalogue = load_default_catalogue(Some(spec_path))?;
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
            catalogue_version: DEFAULT_CATALOGUE_VERSION.to_string(),
            applied: true,
            suggestions: Vec::new(),
            outcome: Some(outcome),
        }))
    } else {
        Ok(Output::Repair(RepairReport {
            status: Status::Ok,
            spec_path: spec_path.to_path_buf(),
            catalogue_version: DEFAULT_CATALOGUE_VERSION.to_string(),
            applied: false,
            suggestions: suggest_repairs(&spec_text, &catalogue),
            outcome: None,
        }))
    }
}

pub fn run_workflow(options: WorkflowRunOptions) -> Result<RunReport> {
    if options.inputs.is_empty() {
        return Err(usage_error("`nano run` needs at least one input"));
    }

    if options.interpret {
        return run_interpreted(options);
    }

    let (spec, plan) = load_validated_plan(&options.spec_path)?;
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
        inputs: options.inputs,
        mode: "compiled".to_string(),
        kernel: workflow.kernel_id,
        events_seen: cutflow.events_seen,
        events_selected: cutflow.events_selected,
        output: Some(output_path.clone()),
        manifest: Some(manifest_path_for_output(&output_path)),
    })
}

fn run_interpreted(options: WorkflowRunOptions) -> Result<RunReport> {
    let (_, plan) = load_validated_plan(&options.spec_path)?;
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

    let output_names = plan
        .spec
        .outputs
        .iter()
        .map(|output| output.name.clone())
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut events_seen = 0_u64;
    let mut events_selected = 0_u64;

    for input in &options.inputs {
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
            let event = event.map_err(|error| CliError {
                status: ErrorStatus::Error,
                kind: ErrorKind::Workflow,
                message: error.to_string(),
                spec_path: Some(options.spec_path.clone()),
                validation_errors: Vec::new(),
            })?;
            events_seen += 1;
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
        }
    }

    if let Some(output_path) = &options.output {
        let branches = output_branches(&plan.spec.outputs, &rows).map_err(|message| CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Interpret,
            message,
            spec_path: Some(options.spec_path.clone()),
            validation_errors: Vec::new(),
        })?;
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
        inputs: options.inputs,
        mode: "interpret".to_string(),
        kernel: "interpret".to_string(),
        events_seen,
        events_selected,
        output: options.output,
        manifest: None,
    })
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

fn load_validated_plan(spec_path: &Path) -> Result<(AnalysisSpec, nano_spec::ResolvedPlan)> {
    let spec =
        AnalysisSpec::from_path(spec_path).map_err(|error| parse_cli_error(spec_path, error))?;
    let catalogue = load_default_catalogue(Some(spec_path))?;
    let plan = nano_spec::validate(&spec, &catalogue).map_err(|errors| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Validation,
        message: "spec validation failed".to_string(),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: errors.iter().map(validation_error_report).collect(),
    })?;
    Ok((spec, plan))
}

fn load_default_catalogue(spec_path: Option<&Path>) -> Result<Catalogue> {
    Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, DEFAULT_CATALOGUE_VERSION).map_err(|error| {
        CliError {
            status: ErrorStatus::Error,
            kind: ErrorKind::Catalogue,
            message: error.to_string(),
            spec_path: spec_path.map(Path::to_path_buf),
            validation_errors: Vec::new(),
        }
    })
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

    match first_value {
        Some(Value::F64(_)) | None => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(Value::F64(value)) => Ok(value as f32),
                Some(other) => Err(format!(
                    "output `{name}` changed type from F64 to {other:?}"
                )),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::f32(name, values)),
        Some(Value::I64(_)) => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(Value::I64(value)) => i32::try_from(value).map_err(|error| {
                    format!("output `{name}` value {value} cannot be written as i32: {error}")
                }),
                Some(other) => Err(format!(
                    "output `{name}` changed type from I64 to {other:?}"
                )),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::i32(name, values)),
        Some(Value::U32(_)) => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(Value::U32(value)) => Ok(value),
                Some(other) => Err(format!(
                    "output `{name}` changed type from U32 to {other:?}"
                )),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::u32(name, values)),
        Some(Value::Bool(_)) => rows
            .iter()
            .map(|row| match row.values.get(index).map(|(_, value)| *value) {
                Some(Value::Bool(value)) => Ok(value),
                Some(other) => Err(format!(
                    "output `{name}` changed type from Bool to {other:?}"
                )),
                None => Err(format!("row is missing output `{name}`")),
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .map(|values| OutputBranch::bool(name, values)),
    }
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
    Validate { spec: PathBuf },
    Branches { spec: PathBuf },
    Inspect { source: String, insecure: bool },
    Codegen { spec: PathBuf },
    Diff { spec_a: PathBuf, spec_b: PathBuf },
    Repair { spec: PathBuf, apply: bool },
    Run(WorkflowRunOptions),
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
            "validate" => Command::Validate {
                spec: one_operand(command, &positional[1..])?,
            },
            "branches" => Command::Branches {
                spec: one_operand(command, &positional[1..])?,
            },
            "inspect" => parse_inspect_args(&positional[1..])?,
            "codegen" => Command::Codegen {
                spec: one_operand(command, &positional[1..])?,
            },
            "diff" => {
                let (spec_a, spec_b) = two_operands(command, &positional[1..])?;
                Command::Diff { spec_a, spec_b }
            }
            "repair" => parse_repair_args(&positional[1..])?,
            "run" => Command::Run(parse_run_args(&positional[1..])?),
            _ => return Err(usage_error(format!("unknown command `{command}`"))),
        };
        Ok(Self { command })
    }
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
    match args {
        [operand] => Ok(PathBuf::from(operand)),
        [] => Err(usage_error(format!("`nano {command}` needs one path"))),
        _ => Err(usage_error(format!(
            "`nano {command}` accepts exactly one path"
        ))),
    }
}

fn two_operands(command: &str, args: &[String]) -> Result<(PathBuf, PathBuf)> {
    match args {
        [left, right] => Ok((PathBuf::from(left), PathBuf::from(right))),
        [] | [_] => Err(usage_error(format!("`nano {command}` needs two paths"))),
        _ => Err(usage_error(format!(
            "`nano {command}` accepts exactly two paths"
        ))),
    }
}

fn parse_repair_args(args: &[String]) -> Result<Command> {
    let mut apply = false;
    let mut spec = None;
    for arg in args {
        match arg.as_str() {
            "--apply" => apply = true,
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
            }
        }
    }
    let Some(spec) = spec else {
        return Err(usage_error("`nano repair` needs one spec path"));
    };
    Ok(Command::Repair { spec, apply })
}

fn parse_run_args(args: &[String]) -> Result<WorkflowRunOptions> {
    let mut spec = None;
    let mut inputs = None;
    let mut output = None;
    let mut parallel = false;
    let mut kernel = None;
    let mut interpret = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--interpret" => {
                interpret = true;
                index += 1;
            }
            "--inputs" => {
                let value = flag_value(args, index, "--inputs")?;
                inputs = Some(parse_input_list(value)?);
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

    let Some(inputs) = inputs else {
        return Err(usage_error("`nano run` needs --inputs <f1,f2,...>"));
    };

    Ok(WorkflowRunOptions {
        spec_path,
        inputs,
        output,
        parallel,
        kernel,
        interpret,
    })
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

fn parse_input_list(value: &str) -> Result<Vec<PathBuf>> {
    let inputs = value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if inputs.is_empty() {
        return Err(usage_error("`nano run --inputs` needs at least one path"));
    }
    Ok(inputs)
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
