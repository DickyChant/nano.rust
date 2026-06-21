use std::fs;
use std::path::{Path, PathBuf};

use nano_cli::{RunReport, WorkflowRunOptions};
use nano_core::BranchType;
use nano_review::{
    repair_spec as review_repair_spec, semantic_diff as review_semantic_diff,
    suggest_repairs as review_suggest_repairs, RepairOutcome, RepairSuggestion, SemanticDiff,
};
use nano_rootio::RootFile;
use nano_spec::codegen;
use nano_spec::{AnalysisSpec, Catalogue, ParseError, SpecError, SpecFormat};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const DEFAULT_CATALOGUE_VERSION: &str = "v9";
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputFormat {
    Toml,
    Yaml,
    Json,
}

impl InputFormat {
    fn as_spec_format(&self) -> SpecFormat {
        match self {
            Self::Toml => SpecFormat::Toml,
            Self::Yaml => SpecFormat::Yaml,
            Self::Json => SpecFormat::Json,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Yaml => "yaml",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecInput {
    pub spec_path: Option<PathBuf>,
    pub spec_text: Option<String>,
    pub format: Option<InputFormat>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectFileInput {
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunWorkflowInput {
    pub spec_path: PathBuf,
    pub inputs: Vec<PathBuf>,
    pub output: Option<PathBuf>,
    #[serde(default)]
    pub parallel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticDiffInput {
    pub spec_a: SpecInput,
    pub spec_b: SpecInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairSpecInput {
    pub spec_path: Option<PathBuf>,
    pub spec_text: Option<String>,
    pub format: Option<InputFormat>,
    #[serde(default)]
    pub apply: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValidateSpecResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis: Option<AnalysisInfo>,
    #[serde(default)]
    pub objects: Vec<ObjectInfo>,
    #[serde(default)]
    pub regions: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub errors: Vec<ToolError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeriveReadBranchesResult {
    pub ok: bool,
    #[serde(default)]
    pub branches: Vec<BranchInfo>,
    #[serde(default)]
    pub errors: Vec<ToolError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectFileResult {
    pub ok: bool,
    #[serde(default)]
    pub trees: Vec<TreeInfo>,
    #[serde(default)]
    pub events_branches: Vec<EventsBranchInfo>,
    #[serde(default)]
    pub errors: Vec<ToolError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerateKernelResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub errors: Vec<ToolError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunWorkflowResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec: Option<PathBuf>,
    #[serde(default)]
    pub inputs: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events_seen: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events_selected: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<PathBuf>,
    #[serde(default)]
    pub errors: Vec<ToolError>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SemanticDiffResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<SemanticDiff>,
    #[serde(default)]
    pub errors: Vec<ToolError>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SuggestRepairsResult {
    pub ok: bool,
    #[serde(default)]
    pub suggestions: Vec<RepairSuggestion>,
    #[serde(default)]
    pub errors: Vec<ToolError>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepairSpecResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RepairOutcome>,
    #[serde(default)]
    pub errors: Vec<ToolError>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalysisInfo {
    pub name: String,
    pub year: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObjectInfo {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub branch_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TreeInfo {
    pub name: String,
    pub entries: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventsBranchInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub branch_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolError {
    pub kind: ToolErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub validation_errors: Vec<ValidationErrorInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorKind {
    Usage,
    Parse,
    Catalogue,
    Validation,
    Compare,
    Codegen,
    Inspect,
    Interpret,
    Kernel,
    Workflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationErrorInfo {
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

struct LoadedSpec {
    spec: AnalysisSpec,
    spec_path: Option<PathBuf>,
}

pub fn validate_spec(input: SpecInput) -> ValidateSpecResult {
    match load_validated_plan(&input) {
        Ok(loaded) => ValidateSpecResult {
            ok: true,
            analysis: Some(analysis_info(&loaded.plan.spec)),
            objects: object_infos(&loaded.plan.spec),
            regions: loaded
                .plan
                .spec
                .regions
                .iter()
                .map(|region| region.name.clone())
                .collect(),
            outputs: loaded
                .plan
                .spec
                .outputs
                .iter()
                .map(|output| output.name.clone())
                .collect(),
            errors: Vec::new(),
        },
        Err(error) => ValidateSpecResult {
            ok: false,
            analysis: None,
            objects: Vec::new(),
            regions: Vec::new(),
            outputs: Vec::new(),
            errors: vec![error],
        },
    }
}

pub fn derive_read_branches(input: SpecInput) -> DeriveReadBranchesResult {
    match load_validated_plan(&input) {
        Ok(loaded) => DeriveReadBranchesResult {
            ok: true,
            branches: branch_infos(loaded.plan.read_branches.specs()),
            errors: Vec::new(),
        },
        Err(error) => DeriveReadBranchesResult {
            ok: false,
            branches: Vec::new(),
            errors: vec![error],
        },
    }
}

pub fn inspect_file(input: InspectFileInput) -> InspectFileResult {
    let root_file = match RootFile::open(&input.path) {
        Ok(root_file) => root_file,
        Err(error) => {
            return InspectFileResult {
                ok: false,
                trees: Vec::new(),
                events_branches: Vec::new(),
                errors: vec![ToolError {
                    kind: ToolErrorKind::Inspect,
                    message: error.to_string(),
                    spec_path: None,
                    path: Some(input.path),
                    validation_errors: Vec::new(),
                }],
            }
        }
    };

    let mut trees = Vec::new();
    let mut events_branches = Vec::new();
    for object in root_file.objects() {
        if object.class() != "TTree" {
            continue;
        }

        let tree = match root_file.tree(object.name()) {
            Ok(tree) => tree,
            Err(error) => {
                return InspectFileResult {
                    ok: false,
                    trees: Vec::new(),
                    events_branches: Vec::new(),
                    errors: vec![ToolError {
                        kind: ToolErrorKind::Inspect,
                        message: error.to_string(),
                        spec_path: None,
                        path: Some(input.path),
                        validation_errors: Vec::new(),
                    }],
                }
            }
        };

        trees.push(TreeInfo {
            name: object.name().to_string(),
            entries: tree.entries(),
        });

        if object.name() == "Events" {
            events_branches = tree
                .branches()
                .into_iter()
                .map(|branch| EventsBranchInfo {
                    name: branch.name,
                    branch_type: branch.types.join("|"),
                })
                .collect();
        }
    }

    InspectFileResult {
        ok: true,
        trees,
        events_branches,
        errors: Vec::new(),
    }
}

pub fn generate_kernel(input: SpecInput) -> GenerateKernelResult {
    match load_validated_plan(&input) {
        Ok(loaded) => match codegen::generate_producer_source(&loaded.plan) {
            Ok(source) => GenerateKernelResult {
                ok: true,
                source: Some(source),
                errors: Vec::new(),
            },
            Err(error) => GenerateKernelResult {
                ok: false,
                source: None,
                errors: vec![ToolError {
                    kind: ToolErrorKind::Codegen,
                    message: error.to_string(),
                    spec_path: loaded.spec_path,
                    path: None,
                    validation_errors: Vec::new(),
                }],
            },
        },
        Err(error) => GenerateKernelResult {
            ok: false,
            source: None,
            errors: vec![error],
        },
    }
}

pub fn run_workflow(input: RunWorkflowInput) -> RunWorkflowResult {
    match nano_cli::run_workflow(WorkflowRunOptions {
        spec_path: input.spec_path,
        inputs: input.inputs,
        output: input.output,
        parallel: input.parallel,
        kernel: None,
        interpret: false,
    }) {
        Ok(report) => run_workflow_success(report),
        Err(error) => RunWorkflowResult {
            ok: false,
            command: Some("run".to_string()),
            status: None,
            spec: error.spec_path.clone(),
            inputs: Vec::new(),
            kernel: None,
            events_seen: None,
            events_selected: None,
            output: None,
            manifest: None,
            errors: vec![tool_error_from_cli(error)],
        },
    }
}

pub fn semantic_diff(input: SemanticDiffInput) -> SemanticDiffResult {
    let spec_a = match load_spec_text(&input.spec_a) {
        Ok(text) => text,
        Err(error) => {
            return SemanticDiffResult {
                ok: false,
                diff: None,
                errors: vec![error],
            }
        }
    };
    let spec_b = match load_spec_text(&input.spec_b) {
        Ok(text) => text,
        Err(error) => {
            return SemanticDiffResult {
                ok: false,
                diff: None,
                errors: vec![error],
            }
        }
    };
    let catalogue = match load_default_catalogue(None) {
        Ok(catalogue) => catalogue,
        Err(error) => {
            return SemanticDiffResult {
                ok: false,
                diff: None,
                errors: vec![error],
            }
        }
    };
    let diff = review_semantic_diff(&spec_a, &spec_b, &catalogue);
    SemanticDiffResult {
        ok: diff.ok,
        diff: Some(diff),
        errors: Vec::new(),
    }
}

pub fn suggest_repairs(input: SpecInput) -> SuggestRepairsResult {
    let spec_text = match load_spec_text(&input) {
        Ok(text) => text,
        Err(error) => {
            return SuggestRepairsResult {
                ok: false,
                suggestions: Vec::new(),
                errors: vec![error],
            }
        }
    };
    let catalogue = match load_default_catalogue(None) {
        Ok(catalogue) => catalogue,
        Err(error) => {
            return SuggestRepairsResult {
                ok: false,
                suggestions: Vec::new(),
                errors: vec![error],
            }
        }
    };

    SuggestRepairsResult {
        ok: true,
        suggestions: review_suggest_repairs(&spec_text, &catalogue),
        errors: Vec::new(),
    }
}

pub fn repair_spec(input: RepairSpecInput) -> RepairSpecResult {
    let spec_input = SpecInput {
        spec_path: input.spec_path,
        spec_text: input.spec_text,
        format: input.format,
    };
    let spec_text = match load_spec_text(&spec_input) {
        Ok(text) => text,
        Err(error) => {
            return RepairSpecResult {
                ok: false,
                outcome: None,
                errors: vec![error],
            }
        }
    };
    let catalogue = match load_default_catalogue(spec_input.spec_path.as_deref()) {
        Ok(catalogue) => catalogue,
        Err(error) => {
            return RepairSpecResult {
                ok: false,
                outcome: None,
                errors: vec![error],
            }
        }
    };

    let outcome = review_repair_spec(&spec_text, &catalogue, input.apply);
    RepairSpecResult {
        ok: outcome.converged || !input.apply,
        outcome: Some(outcome),
        errors: Vec::new(),
    }
}

pub fn handle_json_rpc_line(line: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(line) {
        Ok(request) => handle_json_rpc(request),
        Err(error) => Some(json_rpc_error(
            Value::Null,
            -32700,
            "Parse error",
            Some(json!({ "message": error.to_string() })),
        )),
    }
}

pub fn handle_json_rpc(request: Value) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Some(json_rpc_error(id, -32600, "Invalid Request", None));
    };

    match method {
        "initialize" => Some(json_rpc_success(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "nano-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )),
        "notifications/initialized" => None,
        "tools/list" => Some(json_rpc_success(
            id,
            json!({ "tools": tool_descriptions() }),
        )),
        "tools/call" => Some(handle_tools_call(id, request.get("params").cloned())),
        _ => Some(json_rpc_error(id, -32601, "Method not found", None)),
    }
}

fn handle_tools_call(id: Value, params: Option<Value>) -> Value {
    let Some(params) = params else {
        return json_rpc_error(
            id,
            -32602,
            "Invalid params",
            Some(json!({"message": "missing params"})),
        );
    };
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return json_rpc_error(
            id,
            -32602,
            "Invalid params",
            Some(json!({"message": "missing tool name"})),
        );
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match name {
        "validate_spec" => decode_and_call(id, arguments, validate_spec),
        "derive_read_branches" => decode_and_call(id, arguments, derive_read_branches),
        "inspect_file" => decode_and_call(id, arguments, inspect_file),
        "generate_kernel" => decode_and_call(id, arguments, generate_kernel),
        "run_workflow" => decode_and_call(id, arguments, run_workflow),
        "semantic_diff" => decode_and_call(id, arguments, semantic_diff),
        "suggest_repairs" => decode_and_call(id, arguments, suggest_repairs),
        "repair_spec" => decode_and_call(id, arguments, repair_spec),
        _ => json_rpc_error(
            id,
            -32602,
            "Invalid params",
            Some(json!({ "message": format!("unknown tool `{name}`") })),
        ),
    }
}

fn decode_and_call<I, O>(id: Value, arguments: Value, handler: impl FnOnce(I) -> O) -> Value
where
    I: for<'de> Deserialize<'de>,
    O: Serialize,
{
    match serde_json::from_value::<I>(arguments) {
        Ok(input) => tool_response(id, handler(input)),
        Err(error) => json_rpc_error(
            id,
            -32602,
            "Invalid params",
            Some(json!({ "message": error.to_string() })),
        ),
    }
}

fn tool_response<T: Serialize>(id: Value, result: T) -> Value {
    let structured = serde_json::to_value(result).expect("serialize tool result");
    let ok = structured
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    json_rpc_success(
        id,
        json!({
            "content": [
                {
                    "type": "text",
                    "text": if ok { "ok" } else { "error" }
                }
            ],
            "structuredContent": structured,
            "isError": !ok
        }),
    )
}

fn json_rpc_success(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn json_rpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({
        "code": code,
        "message": message
    });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": error
    })
}

struct ValidatedPlan {
    plan: nano_spec::ResolvedPlan,
    spec_path: Option<PathBuf>,
}

fn load_validated_plan(input: &SpecInput) -> Result<ValidatedPlan, ToolError> {
    let loaded = load_spec(input)?;
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, DEFAULT_CATALOGUE_VERSION)
        .map_err(|error| ToolError {
        kind: ToolErrorKind::Catalogue,
        message: error.to_string(),
        spec_path: loaded.spec_path.clone(),
        path: None,
        validation_errors: Vec::new(),
    })?;
    let plan = nano_spec::validate(&loaded.spec, &catalogue).map_err(|errors| ToolError {
        kind: ToolErrorKind::Validation,
        message: "spec validation failed".to_string(),
        spec_path: loaded.spec_path.clone(),
        path: None,
        validation_errors: errors.iter().map(validation_error_info).collect(),
    })?;

    Ok(ValidatedPlan {
        plan,
        spec_path: loaded.spec_path,
    })
}

fn load_default_catalogue(spec_path: Option<&Path>) -> Result<Catalogue, ToolError> {
    Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, DEFAULT_CATALOGUE_VERSION).map_err(|error| {
        ToolError {
            kind: ToolErrorKind::Catalogue,
            message: error.to_string(),
            spec_path: spec_path.map(Path::to_path_buf),
            path: None,
            validation_errors: Vec::new(),
        }
    })
}

fn load_spec_text(input: &SpecInput) -> Result<String, ToolError> {
    match (&input.spec_path, &input.spec_text) {
        (Some(_), Some(_)) => Err(usage_error(
            "provide exactly one of `spec_path` or `spec_text`, not both",
        )),
        (None, None) => Err(usage_error("provide one of `spec_path` or `spec_text`")),
        (Some(path), None) => fs::read_to_string(path).map_err(|source| ToolError {
            kind: ToolErrorKind::Parse,
            message: format!("failed to read spec `{}`: {source}", path.display()),
            spec_path: Some(path.clone()),
            path: None,
            validation_errors: Vec::new(),
        }),
        (None, Some(text)) => Ok(text.clone()),
    }
}

fn load_spec(input: &SpecInput) -> Result<LoadedSpec, ToolError> {
    match (&input.spec_path, &input.spec_text) {
        (Some(_), Some(_)) => Err(usage_error(
            "provide exactly one of `spec_path` or `spec_text`, not both",
        )),
        (None, None) => Err(usage_error("provide one of `spec_path` or `spec_text`")),
        (Some(path), None) => {
            let spec = if let Some(format) = &input.format {
                let text = fs::read_to_string(path).map_err(|source| ToolError {
                    kind: ToolErrorKind::Parse,
                    message: format!("failed to read spec `{}`: {source}", path.display()),
                    spec_path: Some(path.clone()),
                    path: None,
                    validation_errors: Vec::new(),
                })?;
                nano_spec::parse_analysis_spec_with_format(&text, format.as_spec_format())
            } else {
                AnalysisSpec::from_path(path)
            }
            .map_err(|error| parse_error(path, error))?;
            Ok(LoadedSpec {
                spec,
                spec_path: Some(path.clone()),
            })
        }
        (None, Some(text)) => {
            let spec = parse_spec_text(text, input.format.as_ref())?;
            Ok(LoadedSpec {
                spec,
                spec_path: None,
            })
        }
    }
}

fn parse_spec_text(text: &str, format: Option<&InputFormat>) -> Result<AnalysisSpec, ToolError> {
    if let Some(format) = format {
        return nano_spec::parse_analysis_spec_with_format(text, format.as_spec_format()).map_err(
            |error| ToolError {
                kind: ToolErrorKind::Parse,
                message: format!("failed to parse {} spec text: {error}", format.label()),
                spec_path: None,
                path: None,
                validation_errors: Vec::new(),
            },
        );
    }

    let mut messages = Vec::new();
    for format in [InputFormat::Toml, InputFormat::Yaml, InputFormat::Json] {
        match nano_spec::parse_analysis_spec_with_format(text, format.as_spec_format()) {
            Ok(spec) => return Ok(spec),
            Err(error) => messages.push(format!("{}: {error}", format.label())),
        }
    }

    Err(ToolError {
        kind: ToolErrorKind::Parse,
        message: format!(
            "failed to parse spec_text as TOML, YAML, or JSON ({})",
            messages.join("; ")
        ),
        spec_path: None,
        path: None,
        validation_errors: Vec::new(),
    })
}

fn parse_error(path: &Path, error: ParseError) -> ToolError {
    ToolError {
        kind: ToolErrorKind::Parse,
        message: error.to_string(),
        spec_path: Some(path.to_path_buf()),
        path: None,
        validation_errors: Vec::new(),
    }
}

fn usage_error(message: impl Into<String>) -> ToolError {
    ToolError {
        kind: ToolErrorKind::Usage,
        message: message.into(),
        spec_path: None,
        path: None,
        validation_errors: Vec::new(),
    }
}

fn run_workflow_success(report: RunReport) -> RunWorkflowResult {
    RunWorkflowResult {
        ok: true,
        command: Some("run".to_string()),
        status: Some("ok".to_string()),
        spec: Some(report.spec),
        inputs: report.inputs,
        kernel: Some(report.kernel),
        events_seen: Some(report.events_seen),
        events_selected: Some(report.events_selected),
        output: report.output,
        manifest: report.manifest,
        errors: Vec::new(),
    }
}

fn tool_error_from_cli(error: nano_cli::CliError) -> ToolError {
    ToolError {
        kind: match error.kind {
            nano_cli::ErrorKind::Usage => ToolErrorKind::Usage,
            nano_cli::ErrorKind::Parse => ToolErrorKind::Parse,
            nano_cli::ErrorKind::Catalogue => ToolErrorKind::Catalogue,
            nano_cli::ErrorKind::Validation => ToolErrorKind::Validation,
            nano_cli::ErrorKind::Compare => ToolErrorKind::Compare,
            nano_cli::ErrorKind::Codegen => ToolErrorKind::Codegen,
            nano_cli::ErrorKind::Inspect => ToolErrorKind::Inspect,
            nano_cli::ErrorKind::Interpret => ToolErrorKind::Interpret,
            nano_cli::ErrorKind::Kernel => ToolErrorKind::Kernel,
            nano_cli::ErrorKind::Workflow => ToolErrorKind::Workflow,
        },
        message: error.message,
        spec_path: error.spec_path,
        path: None,
        validation_errors: error
            .validation_errors
            .into_iter()
            .map(validation_error_from_cli)
            .collect(),
    }
}

fn validation_error_from_cli(error: nano_cli::ValidationErrorReport) -> ValidationErrorInfo {
    ValidationErrorInfo {
        kind: match error.kind {
            nano_cli::ValidationErrorKind::MissingBranch => ValidationErrorKind::MissingBranch,
            nano_cli::ValidationErrorKind::UnsupportedBranchType => {
                ValidationErrorKind::UnsupportedBranchType
            }
            nano_cli::ValidationErrorKind::WrongBranchType => ValidationErrorKind::WrongBranchType,
            nano_cli::ValidationErrorKind::MissingUnit => ValidationErrorKind::MissingUnit,
            nano_cli::ValidationErrorKind::UnitMismatch => ValidationErrorKind::UnitMismatch,
            nano_cli::ValidationErrorKind::UndefinedObject => ValidationErrorKind::UndefinedObject,
            nano_cli::ValidationErrorKind::UndefinedBatch => ValidationErrorKind::UndefinedBatch,
            nano_cli::ValidationErrorKind::ModelOutputCollision => {
                ValidationErrorKind::ModelOutputCollision
            }
            nano_cli::ValidationErrorKind::InvalidModel => ValidationErrorKind::InvalidModel,
            nano_cli::ValidationErrorKind::InvalidProvider => ValidationErrorKind::InvalidProvider,
            nano_cli::ValidationErrorKind::InvalidExpression => {
                ValidationErrorKind::InvalidExpression
            }
            nano_cli::ValidationErrorKind::InvalidReadSchema => {
                ValidationErrorKind::InvalidReadSchema
            }
        },
        message: error.message,
        context: error.context,
        branch: error.branch,
        object: error.object,
        expr: error.expr,
        expected: error.expected,
        actual: error.actual,
        detail: error.detail,
    }
}

fn analysis_info(spec: &AnalysisSpec) -> AnalysisInfo {
    AnalysisInfo {
        name: spec.name.clone(),
        year: format!("{:?}", spec.year),
    }
}

fn object_infos(spec: &AnalysisSpec) -> Vec<ObjectInfo> {
    spec.objects
        .iter()
        .map(|object| ObjectInfo {
            name: object.name.clone(),
            source: object.source.clone(),
        })
        .collect()
}

fn branch_infos(branches: &[nano_core::BranchSpec]) -> Vec<BranchInfo> {
    branches
        .iter()
        .map(|branch| BranchInfo {
            name: branch.name.clone(),
            branch_type: branch_type_name(branch.branch_type),
        })
        .collect()
}

fn branch_type_name(branch_type: BranchType) -> String {
    format!("{branch_type:?}")
}

fn validation_error_info(error: &SpecError) -> ValidationErrorInfo {
    match error {
        SpecError::MissingBranch { context, branch } => ValidationErrorInfo {
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
        } => ValidationErrorInfo {
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
        } => ValidationErrorInfo {
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
        } => ValidationErrorInfo {
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
        } => ValidationErrorInfo {
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
        SpecError::UndefinedObject { context, object } => ValidationErrorInfo {
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
        SpecError::UndefinedBatch { context, batch } => ValidationErrorInfo {
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
        SpecError::ModelOutputCollision { context, output } => ValidationErrorInfo {
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
        SpecError::InvalidModel { context, detail } => ValidationErrorInfo {
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
        SpecError::InvalidProvider { context, detail } => ValidationErrorInfo {
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
        SpecError::InvalidExpression { context, detail } => ValidationErrorInfo {
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
        SpecError::InvalidReadSchema { detail } => ValidationErrorInfo {
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

fn tool_descriptions() -> Vec<Value> {
    vec![
        json!({
            "name": "semantic_diff",
            "description": "Parse, validate, and semantically diff two nano.rust analysis specs, including object cuts, regions, outputs, models, and read-branch deltas.",
            "inputSchema": {
                "type": "object",
                "required": ["spec_a", "spec_b"],
                "additionalProperties": false,
                "properties": {
                    "spec_a": spec_input_schema(),
                    "spec_b": spec_input_schema()
                }
            },
            "outputSchema": review_output_schema("diff")
        }),
        json!({
            "name": "suggest_repairs",
            "description": "Validate a nano.rust analysis spec and return typed repair suggestions for compiler-gated validation errors.",
            "inputSchema": spec_input_schema(),
            "outputSchema": {
                "type": "object",
                "required": ["ok", "suggestions", "errors"],
                "properties": {
                    "ok": { "type": "boolean" },
                    "suggestions": { "type": "array", "items": { "type": "object" } },
                    "errors": errors_schema()
                }
            }
        }),
        json!({
            "name": "repair_spec",
            "description": "Run a bounded validation-repair loop over a nano.rust spec and return the repaired text plus remaining errors.",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "oneOf": [
                    { "required": ["spec_path"] },
                    { "required": ["spec_text"] }
                ],
                "properties": {
                    "spec_path": { "type": "string", "description": "Path to a TOML, YAML, or JSON analysis spec." },
                    "spec_text": { "type": "string", "description": "Inline TOML, YAML, or JSON analysis spec text." },
                    "format": {
                        "type": "string",
                        "enum": ["toml", "yaml", "json"],
                        "description": "Optional explicit format."
                    },
                    "apply": { "type": "boolean", "description": "Apply high-confidence replacements in a bounded loop when true." }
                }
            },
            "outputSchema": review_output_schema("outcome")
        }),
        json!({
            "name": "validate_spec",
            "description": "Parse and validate a nano.rust analysis spec against the NanoAOD branch catalogue.",
            "inputSchema": spec_input_schema(),
            "outputSchema": {
                "type": "object",
                "required": ["ok", "objects", "regions", "outputs", "errors"],
                "properties": {
                    "ok": { "type": "boolean" },
                    "analysis": analysis_schema(),
                    "objects": { "type": "array", "items": object_schema() },
                    "regions": { "type": "array", "items": { "type": "string" } },
                    "outputs": { "type": "array", "items": { "type": "string" } },
                    "errors": errors_schema()
                }
            }
        }),
        json!({
            "name": "derive_read_branches",
            "description": "Validate a nano.rust analysis spec and return the exact NanoAOD branches the event reader must bind.",
            "inputSchema": spec_input_schema(),
            "outputSchema": {
                "type": "object",
                "required": ["ok", "branches", "errors"],
                "properties": {
                    "ok": { "type": "boolean" },
                    "branches": { "type": "array", "items": branch_schema() },
                    "errors": errors_schema()
                }
            }
        }),
        json!({
            "name": "inspect_file",
            "description": "Inspect a local ROOT file and list TTrees plus Events branch metadata when present.",
            "inputSchema": {
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "description": "Path to a local ROOT file." }
                }
            },
            "outputSchema": {
                "type": "object",
                "required": ["ok", "trees", "events_branches", "errors"],
                "properties": {
                    "ok": { "type": "boolean" },
                    "trees": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["name", "entries"],
                            "properties": {
                                "name": { "type": "string" },
                                "entries": { "type": "integer" }
                            }
                        }
                    },
                    "events_branches": { "type": "array", "items": branch_schema() },
                    "errors": errors_schema()
                }
            }
        }),
        json!({
            "name": "generate_kernel",
            "description": "Validate a nano.rust analysis spec and generate the Rust producer source for the supported semantic slice.",
            "inputSchema": spec_input_schema(),
            "outputSchema": {
                "type": "object",
                "required": ["ok", "errors"],
                "properties": {
                    "ok": { "type": "boolean" },
                    "source": { "type": "string" },
                    "errors": errors_schema()
                }
            }
        }),
        json!({
            "name": "run_workflow",
            "description": "Validate a spec, resolve a registered runtime kernel, execute the local workflow DAG over ROOT inputs, and write the skim plus provenance manifest.",
            "inputSchema": {
                "type": "object",
                "required": ["spec_path", "inputs"],
                "additionalProperties": false,
                "properties": {
                    "spec_path": { "type": "string", "description": "Path to a TOML, YAML, or JSON analysis spec." },
                    "inputs": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Local ROOT input paths."
                    },
                    "output": { "type": "string", "description": "Optional output skim path." },
                    "parallel": { "type": "boolean", "description": "Run map/reduce nodes in parallel when true." }
                }
            },
            "outputSchema": {
                "type": "object",
                "required": ["ok", "inputs", "errors"],
                "properties": {
                    "ok": { "type": "boolean" },
                    "command": { "type": "string" },
                    "status": { "type": "string" },
                    "spec": { "type": "string" },
                    "inputs": { "type": "array", "items": { "type": "string" } },
                    "kernel": { "type": "string" },
                    "events_seen": { "type": "integer" },
                    "events_selected": { "type": "integer" },
                    "output": { "type": "string" },
                    "manifest": { "type": "string" },
                    "errors": errors_schema()
                }
            }
        }),
    ]
}

fn review_output_schema(payload_name: &str) -> Value {
    json!({
        "type": "object",
        "required": ["ok", "errors"],
        "properties": {
            "ok": { "type": "boolean" },
            payload_name: { "type": "object" },
            "errors": errors_schema()
        }
    })
}

fn spec_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "oneOf": [
            { "required": ["spec_path"] },
            { "required": ["spec_text"] }
        ],
        "properties": {
            "spec_path": { "type": "string", "description": "Path to a TOML, YAML, or JSON analysis spec." },
            "spec_text": { "type": "string", "description": "Inline TOML, YAML, or JSON analysis spec text." },
            "format": {
                "type": "string",
                "enum": ["toml", "yaml", "json"],
                "description": "Optional explicit format. Paths infer by extension when omitted; inline text auto-detects TOML, then YAML, then JSON."
            }
        }
    })
}

fn analysis_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name", "year"],
        "properties": {
            "name": { "type": "string" },
            "year": { "type": "string" }
        }
    })
}

fn object_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name", "source"],
        "properties": {
            "name": { "type": "string" },
            "source": { "type": "string" }
        }
    })
}

fn branch_schema() -> Value {
    json!({
        "type": "object",
        "required": ["name", "type"],
        "properties": {
            "name": { "type": "string" },
            "type": { "type": "string" }
        }
    })
}

fn errors_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "required": ["kind", "message"],
            "properties": {
                "kind": { "type": "string" },
                "message": { "type": "string" },
                "spec_path": { "type": "string" },
                "path": { "type": "string" },
                "validation_errors": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["kind", "message"],
                        "properties": {
                            "kind": { "type": "string" },
                            "message": { "type": "string" },
                            "context": { "type": "string" },
                            "branch": { "type": "string" },
                            "object": { "type": "string" },
                            "expr": { "type": "string" },
                            "expected": { "type": "string" },
                            "actual": { "type": "string" },
                            "detail": { "type": "string" }
                        }
                    }
                }
            }
        }
    })
}
