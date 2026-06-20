use std::fmt;
use std::path::{Path, PathBuf};

use nano_core::BranchType;
use nano_rootio::RootFile;
use nano_spec::codegen;
use nano_spec::{AnalysisSpec, Catalogue, ParseError, SpecError};
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
pub enum Output {
    Validate(ValidateReport),
    Branches(BranchesReport),
    Inspect(InspectReport),
    Codegen(CodegenReport),
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
        Command::Inspect { file } => inspect_command(&file),
        Command::Codegen { spec } => codegen_command(&spec),
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

fn inspect_command(file: &Path) -> Result<Output> {
    let root_file = RootFile::open(file).map_err(|error| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Inspect,
        message: error.to_string(),
        spec_path: None,
        validation_errors: Vec::new(),
    })?;

    let mut trees = Vec::new();
    for object in root_file.objects() {
        if object.class() != "TTree" {
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
        file: file.to_path_buf(),
        trees,
    }))
}

fn load_validated_plan(spec_path: &Path) -> Result<(AnalysisSpec, nano_spec::ResolvedPlan)> {
    let spec =
        AnalysisSpec::from_path(spec_path).map_err(|error| parse_cli_error(spec_path, error))?;
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, DEFAULT_CATALOGUE_VERSION)
        .map_err(|error| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Catalogue,
        message: error.to_string(),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: Vec::new(),
    })?;
    let plan = nano_spec::validate(&spec, &catalogue).map_err(|errors| CliError {
        status: ErrorStatus::Error,
        kind: ErrorKind::Validation,
        message: "spec validation failed".to_string(),
        spec_path: Some(spec_path.to_path_buf()),
        validation_errors: errors.iter().map(validation_error_report).collect(),
    })?;
    Ok((spec, plan))
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
    Inspect { file: PathBuf },
    Codegen { spec: PathBuf },
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
        let operand = one_operand(command, &positional[1..])?;
        let command = match command {
            "validate" => Command::Validate { spec: operand },
            "branches" => Command::Branches { spec: operand },
            "inspect" => Command::Inspect { file: operand },
            "codegen" => Command::Codegen { spec: operand },
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

fn one_operand(command: &str, args: &[String]) -> Result<PathBuf> {
    match args {
        [operand] => Ok(PathBuf::from(operand)),
        [] => Err(usage_error(format!("`nano {command}` needs one path"))),
        _ => Err(usage_error(format!(
            "`nano {command}` accepts exactly one path"
        ))),
    }
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
