//! Semantic analysis specifications for nano.rust.
//!
//! This crate implements the first semantic-IR slice: parse a physics-facing
//! TOML/YAML/JSON specification, validate it against a NanoAOD branch catalogue, and
//! derive the exact [`nano_core::BranchSchema`] needed by the streaming reader.

use nano_core::{BranchSchema, BranchSpec, BranchType};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub mod codegen;
pub mod interpret;

/// Typed semantic analysis specification.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AnalysisSpec {
    pub name: String,
    pub year: Year,
    pub objects: Vec<ObjectDef>,
    pub models: Vec<ModelDef>,
    pub regions: Vec<RegionDef>,
    pub outputs: Vec<OutputDef>,
}

impl AnalysisSpec {
    /// Parse an analysis specification from the physics-facing YAML form.
    pub fn from_yaml_str(input: &str) -> Result<Self, ParseError> {
        parse_analysis_spec_with_format(input, SpecFormat::Yaml)
    }

    /// Parse an analysis specification from the physics-facing TOML form.
    pub fn from_toml_str(input: &str) -> Result<Self, ParseError> {
        parse_analysis_spec_with_format(input, SpecFormat::Toml)
    }

    /// Parse an analysis specification from the physics-facing JSON form.
    pub fn from_json_str(input: &str) -> Result<Self, ParseError> {
        parse_analysis_spec_with_format(input, SpecFormat::Json)
    }

    /// Load an analysis specification from a file, dispatching by extension.
    ///
    /// Supported extensions are `.toml`, `.yaml`, `.yml`, and `.json`.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ParseError> {
        load_analysis_spec(path)
    }
}

impl<'de> serde::Deserialize<'de> for AnalysisSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = <RawAnalysisSpec as serde::Deserialize>::deserialize(deserializer)?;
        analysis_spec_from_raw(raw).map_err(serde::de::Error::custom)
    }
}

/// Physics-facing spec serialization format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecFormat {
    Toml,
    Yaml,
    Json,
}

impl SpecFormat {
    /// Infer the spec format from a path extension.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ParseError> {
        let path = path.as_ref();
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);

        match extension.as_deref() {
            Some("toml") => Ok(Self::Toml),
            Some("yaml" | "yml") => Ok(Self::Yaml),
            Some("json") => Ok(Self::Json),
            _ => Err(ParseError::UnsupportedFormat {
                path: path.to_path_buf(),
            }),
        }
    }
}

impl fmt::Display for SpecFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml => f.write_str("TOML"),
            Self::Yaml => f.write_str("YAML"),
            Self::Json => f.write_str("JSON"),
        }
    }
}

/// Data-taking year label from the analysis spec.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Year {
    Run2016,
    Run2017,
    Run2018,
    Other(String),
}

impl Year {
    fn parse(value: &str) -> Self {
        match value {
            "Run2016" => Self::Run2016,
            "Run2017" => Self::Run2017,
            "Run2018" => Self::Run2018,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Object definition, such as `good_muon` sourced from NanoAOD `Muon`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ObjectDef {
    pub name: String,
    pub source: String,
    pub cuts: Vec<Cut>,
}

/// A numeric comparison cut.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cut {
    pub lhs: Expr,
    pub op: CmpOp,
    pub rhs: Quantity,
}

/// A model binding declared by a `[[model]]` spec table.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelDef {
    pub name: String,
    pub inputs: Vec<String>,
    pub output: String,
    pub output_dtype: ModelOutputDType,
    pub batch: String,
    pub provider: ModelProviderSpec,
}

/// Model output dtype. Layer 3 only accepts one F32 score column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelOutputDType {
    F32,
}

/// Inference provider binding from the spec.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelProviderSpec {
    pub kind: ModelProviderKind,
    pub endpoint: Option<String>,
    pub launch: Option<String>,
    pub onnx_path: Option<String>,
}

impl ModelProviderSpec {
    fn mock() -> Self {
        Self {
            kind: ModelProviderKind::Mock,
            endpoint: None,
            launch: None,
            onnx_path: None,
        }
    }
}

/// Supported provider kinds. `Other` is retained so validation can report a
/// structured provider error instead of failing during serde conversion.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelProviderKind {
    Mock,
    InProcess,
    Remote,
    Managed,
    Other(String),
}

/// A value with an explicit or dimensionless unit.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Quantity {
    pub value: f64,
    pub unit: Unit,
}

/// Units currently understood by the semantic validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Unit {
    GeV,
    Dimensionless,
}

/// Expression nodes for the first semantic slice.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Expr {
    Attr { object: String, attr: String },
    Abs(Box<Expr>),
    Count(String),
    LeadingAttr { object: String, attr: String },
}

/// Comparison operators supported in cuts and region requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CmpOp {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

/// A region-level requirement, such as `count(good_muon) >= 1`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Requirement {
    pub lhs: Expr,
    pub op: CmpOp,
    pub rhs: f64,
}

/// Named region definition.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RegionDef {
    pub name: String,
    pub require: Vec<Requirement>,
}

/// Named output expression.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutputDef {
    pub name: String,
    pub expr: Expr,
}

/// A parsed NanoAOD branch catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalogue {
    branches: BTreeMap<String, CatalogueBranch>,
}

impl Catalogue {
    /// Parse the `configs/branches/nanov*.yaml` catalogue format.
    pub fn from_nanoaod_yaml_str(input: &str, version: &str) -> Result<Self, ParseError> {
        parse_catalogue(input, version)
    }

    /// Return catalogue metadata for a branch.
    pub fn branch(&self, name: &str) -> Option<&CatalogueBranch> {
        self.branches.get(name)
    }

    /// Return branch names known to the catalogue in stable sorted order.
    pub fn branch_names(&self) -> impl Iterator<Item = &str> {
        self.branches.keys().map(String::as_str)
    }
}

/// Metadata for one catalogue branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueBranch {
    pub branch_type: Option<BranchType>,
    pub raw_type: String,
}

/// Validated analysis plan.
#[derive(Debug, Clone)]
pub struct ResolvedPlan {
    pub spec: AnalysisSpec,
    pub read_branches: BranchSchema,
}

/// Load a physics-facing spec from a file, dispatching by extension.
pub fn load_analysis_spec(path: impl AsRef<Path>) -> Result<AnalysisSpec, ParseError> {
    let path = path.as_ref();
    let format = SpecFormat::from_path(path)?;
    let input = fs::read_to_string(path).map_err(|source| ParseError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_analysis_spec_with_format(&input, format)
}

/// Parse the physics-facing YAML spec into typed IR.
pub fn parse_analysis_spec(input: &str) -> Result<AnalysisSpec, ParseError> {
    parse_analysis_spec_with_format(input, SpecFormat::Yaml)
}

/// Parse the physics-facing spec into typed IR using the requested serde format.
pub fn parse_analysis_spec_with_format(
    input: &str,
    format: SpecFormat,
) -> Result<AnalysisSpec, ParseError> {
    match format {
        SpecFormat::Toml => toml::from_str(input).map_err(|error| {
            ParseError::InvalidSpec(format!("failed to parse TOML spec: {error}"))
        }),
        SpecFormat::Yaml => serde_yaml::from_str(input).map_err(|error| {
            ParseError::InvalidSpec(format!("failed to parse YAML spec: {error}"))
        }),
        SpecFormat::Json => serde_json::from_str(input).map_err(|error| {
            ParseError::InvalidSpec(format!("failed to parse JSON spec: {error}"))
        }),
    }
}

fn analysis_spec_from_raw(raw: RawAnalysisSpec) -> Result<AnalysisSpec, ParseError> {
    validate_raw_analysis_spec(&raw)?;
    let mut objects = Vec::with_capacity(raw.objects.len());
    for (name, object) in raw.objects {
        let cuts = object
            .cuts
            .iter()
            .map(|cut| parse_cut(cut, &name))
            .collect::<Result<Vec<_>, _>>()?;
        objects.push(ObjectDef {
            name,
            source: object.source,
            cuts,
        });
    }

    let models = raw
        .models
        .into_iter()
        .map(model_def_from_raw)
        .collect::<Result<Vec<_>, _>>()?;

    let mut regions = Vec::with_capacity(raw.regions.len());
    for (name, region) in raw.regions {
        let require = region
            .require
            .iter()
            .map(|requirement| parse_requirement(requirement))
            .collect::<Result<Vec<_>, _>>()?;
        regions.push(RegionDef { name, require });
    }

    let outputs = raw
        .outputs
        .iter()
        .map(|output| {
            Ok(OutputDef {
                name: output.name.clone(),
                expr: parse_expr(&output.expr, None)?,
            })
        })
        .collect::<Result<Vec<_>, ParseError>>()?;

    Ok(AnalysisSpec {
        name: raw.analysis.name,
        year: Year::parse(&raw.analysis.year),
        objects,
        models,
        regions,
        outputs,
    })
}

/// Validate a typed spec against a branch catalogue and derive the read schema.
pub fn validate(
    spec: &AnalysisSpec,
    catalogue: &Catalogue,
) -> Result<ResolvedPlan, Vec<SpecError>> {
    let object_sources = spec
        .objects
        .iter()
        .map(|object| (object.name.as_str(), object.source.as_str()))
        .collect::<HashMap<_, _>>();
    let mut errors = Vec::new();
    let mut required = RequiredBranches::default();
    let model_outputs =
        validate_models(spec, catalogue, &object_sources, &mut required, &mut errors);

    {
        let mut ctx = ValidationContext {
            catalogue,
            object_sources: &object_sources,
            model_outputs: &model_outputs,
            required: &mut required,
            errors: &mut errors,
        };

        for object in &spec.objects {
            ctx.required.require_counter(&object.source);
            for (index, cut) in object.cuts.iter().enumerate() {
                validate_cut(object, index, cut, &mut ctx);
            }
        }

        for region in &spec.regions {
            for (index, requirement) in region.require.iter().enumerate() {
                validate_requirement(region, index, requirement, &mut ctx);
            }
        }

        for output in &spec.outputs {
            validate_expr(&output.expr, &format!("output `{}`", output.name), &mut ctx);
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let branch_specs = required
        .to_branch_specs(catalogue)
        .map_err(|error| vec![error])?;
    let read_branches = BranchSchema::new(branch_specs).map_err(|error| {
        vec![SpecError::InvalidReadSchema {
            detail: error.to_string(),
        }]
    })?;

    Ok(ResolvedPlan {
        spec: spec.clone(),
        read_branches,
    })
}

struct ValidationContext<'a> {
    catalogue: &'a Catalogue,
    object_sources: &'a HashMap<&'a str, &'a str>,
    model_outputs: &'a ModelOutputs,
    required: &'a mut RequiredBranches,
    errors: &'a mut Vec<SpecError>,
}

fn validate_cut(object: &ObjectDef, index: usize, cut: &Cut, ctx: &mut ValidationContext<'_>) {
    let context = format!("object `{}` cut {}", object.name, index + 1);
    let lhs_type = validate_expr(&cut.lhs, &context, ctx);

    match lhs_type {
        Some(ExprType::Numeric(dimension)) => {
            validate_quantity_unit(&context, &cut.lhs, dimension, cut, ctx.errors)
        }
        Some(ExprType::Count) => ctx.errors.push(SpecError::InvalidExpression {
            context,
            detail: "object cuts must compare branch attributes, not counts".to_string(),
        }),
        None => {}
    }
}

fn validate_requirement(
    region: &RegionDef,
    index: usize,
    requirement: &Requirement,
    ctx: &mut ValidationContext<'_>,
) {
    let context = format!("region `{}` requirement {}", region.name, index + 1);
    validate_expr(&requirement.lhs, &context, ctx);
}

fn validate_expr(expr: &Expr, context: &str, ctx: &mut ValidationContext<'_>) -> Option<ExprType> {
    match expr {
        Expr::Attr { object, attr } => validate_attr(object, attr, context, ctx),
        Expr::Abs(inner) => match validate_expr(inner, context, ctx) {
            Some(ExprType::Numeric(dimension)) => Some(ExprType::Numeric(dimension)),
            Some(ExprType::Count) => {
                ctx.errors.push(SpecError::InvalidExpression {
                    context: context.to_string(),
                    detail: "abs(...) requires a numeric attribute".to_string(),
                });
                None
            }
            None => None,
        },
        Expr::Count(object) => {
            let Some(source) = ctx.object_sources.get(object.as_str()) else {
                ctx.errors.push(SpecError::UndefinedObject {
                    context: context.to_string(),
                    object: object.clone(),
                });
                return None;
            };
            ctx.required.require_counter(source);
            Some(ExprType::Count)
        }
        Expr::LeadingAttr { object, attr } => validate_attr(object, attr, context, ctx),
    }
}

fn validate_attr(
    object: &str,
    attr: &str,
    context: &str,
    ctx: &mut ValidationContext<'_>,
) -> Option<ExprType> {
    let Some(source) = ctx.object_sources.get(object) else {
        ctx.errors.push(SpecError::UndefinedObject {
            context: context.to_string(),
            object: object.to_string(),
        });
        return None;
    };

    let branch = format!("{source}_{attr}");
    if let Some(output) = ctx.model_outputs.by_branch.get(&branch) {
        ctx.required.require_counter(source);
        return Some(ExprType::Numeric(output.dimension));
    }

    let Some(entry) = ctx.catalogue.branch(&branch) else {
        ctx.errors.push(SpecError::MissingBranch {
            context: context.to_string(),
            branch,
        });
        return None;
    };

    let Some(branch_type) = entry.branch_type else {
        ctx.errors.push(SpecError::UnsupportedBranchType {
            context: context.to_string(),
            branch,
            raw_type: entry.raw_type.clone(),
        });
        return None;
    };

    if !is_numeric_vector(branch_type) {
        ctx.errors.push(SpecError::WrongBranchType {
            context: context.to_string(),
            branch,
            expected: "numeric vector branch".to_string(),
            actual: branch_type,
        });
        return None;
    }

    ctx.required.require_counter(source);
    ctx.required.require_attr(source, attr);
    Some(ExprType::Numeric(attribute_dimension(attr)))
}

fn validate_quantity_unit(
    context: &str,
    lhs: &Expr,
    dimension: Dimension,
    cut: &Cut,
    errors: &mut Vec<SpecError>,
) {
    match (dimension, cut.rhs.unit) {
        (Dimension::Momentum, Unit::GeV) | (Dimension::Dimensionless, Unit::Dimensionless) => {}
        (Dimension::Momentum, Unit::Dimensionless) => errors.push(SpecError::MissingUnit {
            context: context.to_string(),
            expr: format!("{lhs}"),
            expected: Unit::GeV,
        }),
        (expected, actual) => errors.push(SpecError::UnitMismatch {
            context: context.to_string(),
            expr: format!("{lhs}"),
            expected,
            actual,
        }),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModelOutputInfo {
    dimension: Dimension,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ModelOutputs {
    by_branch: BTreeMap<String, ModelOutputInfo>,
}

fn validate_models(
    spec: &AnalysisSpec,
    catalogue: &Catalogue,
    object_sources: &HashMap<&str, &str>,
    required: &mut RequiredBranches,
    errors: &mut Vec<SpecError>,
) -> ModelOutputs {
    let mut outputs = ModelOutputs::default();
    let mut seen_names = BTreeSet::new();
    let mut seen_outputs = BTreeSet::new();

    for model in &spec.models {
        let context = format!("model `{}`", model.name);
        if !seen_names.insert(model.name.as_str()) {
            errors.push(SpecError::InvalidModel {
                context: context.clone(),
                detail: format!("duplicate model name `{}`", model.name),
            });
        }

        let batch_source = validate_model_batch(model, &context, object_sources, errors);
        validate_model_inputs(model, &context, catalogue, required, errors);
        validate_model_output(
            model,
            &context,
            batch_source,
            catalogue,
            &mut seen_outputs,
            &mut outputs,
            errors,
        );
        validate_model_provider(model, &context, errors);
    }

    outputs
}

fn validate_model_batch(
    model: &ModelDef,
    context: &str,
    object_sources: &HashMap<&str, &str>,
    errors: &mut Vec<SpecError>,
) -> Option<String> {
    if let Some(source) = object_sources.get(model.batch.as_str()) {
        return Some((*source).to_string());
    }
    if object_sources
        .values()
        .any(|source| *source == model.batch.as_str())
    {
        return Some(model.batch.clone());
    }

    errors.push(SpecError::UndefinedBatch {
        context: context.to_string(),
        batch: model.batch.clone(),
    });
    None
}

fn validate_model_inputs(
    model: &ModelDef,
    context: &str,
    catalogue: &Catalogue,
    required: &mut RequiredBranches,
    errors: &mut Vec<SpecError>,
) {
    if model.inputs.is_empty() {
        errors.push(SpecError::InvalidModel {
            context: context.to_string(),
            detail: "model inputs must not be empty".to_string(),
        });
    }

    for input in &model.inputs {
        let input_context = format!("{context} input `{input}`");
        let Some(entry) = catalogue.branch(input) else {
            errors.push(SpecError::MissingBranch {
                context: input_context,
                branch: input.clone(),
            });
            continue;
        };
        let Some(branch_type) = entry.branch_type else {
            errors.push(SpecError::UnsupportedBranchType {
                context: input_context,
                branch: input.clone(),
                raw_type: entry.raw_type.clone(),
            });
            continue;
        };
        if !is_numeric_branch(branch_type) {
            errors.push(SpecError::WrongBranchType {
                context: input_context,
                branch: input.clone(),
                expected: "numeric branch".to_string(),
                actual: branch_type,
            });
            continue;
        }
        required.require_branch(input);
    }
}

fn validate_model_output(
    model: &ModelDef,
    context: &str,
    batch_source: Option<String>,
    catalogue: &Catalogue,
    seen_outputs: &mut BTreeSet<String>,
    outputs: &mut ModelOutputs,
    errors: &mut Vec<SpecError>,
) {
    if model.output_dtype != ModelOutputDType::F32 {
        errors.push(SpecError::InvalidModel {
            context: context.to_string(),
            detail: "model output dtype must be F32".to_string(),
        });
    }

    let Some((output_source, output_attr)) = model.output.split_once('_') else {
        errors.push(SpecError::InvalidModel {
            context: context.to_string(),
            detail: format!(
                "output `{}` must be a per-object column like `Collection_score`",
                model.output
            ),
        });
        return;
    };

    if output_source.is_empty() || output_attr.is_empty() {
        errors.push(SpecError::InvalidModel {
            context: context.to_string(),
            detail: format!(
                "output `{}` must be a per-object column like `Collection_score`",
                model.output
            ),
        });
        return;
    }

    if let Some(batch_source) = batch_source.as_deref() {
        if output_source != batch_source {
            errors.push(SpecError::InvalidModel {
                context: context.to_string(),
                detail: format!(
                    "output `{}` belongs to `{output_source}`, but batch `{}` resolves to `{batch_source}`",
                    model.output, model.batch
                ),
            });
        }
    }

    if catalogue.branch(&model.output).is_some() {
        errors.push(SpecError::ModelOutputCollision {
            context: context.to_string(),
            output: model.output.clone(),
        });
    }

    if !seen_outputs.insert(model.output.clone()) {
        errors.push(SpecError::ModelOutputCollision {
            context: context.to_string(),
            output: model.output.clone(),
        });
    }

    outputs.by_branch.insert(
        model.output.clone(),
        ModelOutputInfo {
            dimension: Dimension::Dimensionless,
        },
    );
}

fn validate_model_provider(model: &ModelDef, context: &str, errors: &mut Vec<SpecError>) {
    match &model.provider.kind {
        ModelProviderKind::Mock => {}
        ModelProviderKind::InProcess => {
            if model
                .provider
                .onnx_path
                .as_deref()
                .is_none_or(str::is_empty)
            {
                errors.push(SpecError::InvalidProvider {
                    context: context.to_string(),
                    detail: "inproc provider requires `onnx_path`".to_string(),
                });
            }
        }
        ModelProviderKind::Remote => match model.provider.endpoint.as_deref() {
            Some(endpoint) if url::Url::parse(endpoint).is_ok() => {}
            Some(endpoint) => errors.push(SpecError::InvalidProvider {
                context: context.to_string(),
                detail: format!("remote provider endpoint `{endpoint}` is not a valid URL"),
            }),
            None => errors.push(SpecError::InvalidProvider {
                context: context.to_string(),
                detail: "remote provider requires `endpoint`".to_string(),
            }),
        },
        ModelProviderKind::Managed => {
            if model.provider.launch.as_deref().is_none_or(str::is_empty) {
                errors.push(SpecError::InvalidProvider {
                    context: context.to_string(),
                    detail: "managed provider requires `launch`".to_string(),
                });
            }
        }
        ModelProviderKind::Other(kind) => {
            let detail = if kind.is_empty() {
                "provider requires `kind`".to_string()
            } else {
                format!("unsupported provider kind `{kind}`")
            };
            errors.push(SpecError::InvalidProvider {
                context: context.to_string(),
                detail,
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExprType {
    Numeric(Dimension),
    Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Dimension {
    Momentum,
    Dimensionless,
}

fn attribute_dimension(attr: &str) -> Dimension {
    match attr {
        "pt" | "mass" | "energy" | "msoftdrop" | "rawFactor" => Dimension::Momentum,
        value if value.ends_with("Pt") || value.ends_with("Mass") => Dimension::Momentum,
        _ => Dimension::Dimensionless,
    }
}

#[derive(Debug, Default)]
struct RequiredBranches {
    counters: BTreeSet<String>,
    attrs: BTreeSet<(String, String)>,
    branches: BTreeSet<String>,
}

impl RequiredBranches {
    fn require_counter(&mut self, source: &str) {
        self.counters.insert(source.to_string());
    }

    fn require_attr(&mut self, source: &str, attr: &str) {
        self.attrs.insert((source.to_string(), attr.to_string()));
    }

    fn require_branch(&mut self, branch: &str) {
        self.branches.insert(branch.to_string());
    }

    fn to_branch_specs(&self, catalogue: &Catalogue) -> Result<Vec<BranchSpec>, SpecError> {
        let mut specs =
            Vec::with_capacity(self.counters.len() + self.attrs.len() + self.branches.len());
        let mut seen = BTreeSet::new();

        for source in &self.counters {
            let branch = format!("n{source}");
            let branch_type = catalogue_branch_type(catalogue, &branch, "derived read_branches")?;
            if seen.insert(branch.clone()) {
                specs.push(BranchSpec::new(branch, branch_type));
            }
        }

        for (source, attr) in &self.attrs {
            let branch = format!("{source}_{attr}");
            let branch_type = catalogue_branch_type(catalogue, &branch, "derived read_branches")?;
            if seen.insert(branch.clone()) {
                specs.push(BranchSpec::new(branch, branch_type));
            }
        }

        for branch in &self.branches {
            let branch_type = catalogue_branch_type(catalogue, branch, "derived read_branches")?;
            if seen.insert(branch.clone()) {
                specs.push(BranchSpec::new(branch.clone(), branch_type));
            }
        }

        Ok(specs)
    }
}

fn catalogue_branch_type(
    catalogue: &Catalogue,
    branch: &str,
    context: &str,
) -> Result<BranchType, SpecError> {
    let Some(entry) = catalogue.branch(branch) else {
        return Err(SpecError::MissingBranch {
            context: context.to_string(),
            branch: branch.to_string(),
        });
    };
    entry
        .branch_type
        .ok_or_else(|| SpecError::UnsupportedBranchType {
            context: context.to_string(),
            branch: branch.to_string(),
            raw_type: entry.raw_type.clone(),
        })
}

fn is_numeric_vector(branch_type: BranchType) -> bool {
    matches!(
        branch_type,
        BranchType::VecI8
            | BranchType::VecU8
            | BranchType::VecI16
            | BranchType::VecU16
            | BranchType::VecI32
            | BranchType::VecU32
            | BranchType::VecI64
            | BranchType::VecU64
            | BranchType::VecF32
    )
}

fn is_numeric_branch(branch_type: BranchType) -> bool {
    matches!(
        branch_type,
        BranchType::I8
            | BranchType::U8
            | BranchType::I16
            | BranchType::U16
            | BranchType::I32
            | BranchType::U32
            | BranchType::I64
            | BranchType::U64
            | BranchType::F32
            | BranchType::VecI8
            | BranchType::VecU8
            | BranchType::VecI16
            | BranchType::VecU16
            | BranchType::VecI32
            | BranchType::VecU32
            | BranchType::VecI64
            | BranchType::VecU64
            | BranchType::VecF32
    )
}

fn parse_catalogue_branch_type(value: &str) -> Option<BranchType> {
    match value {
        "bool" => Some(BranchType::Bool),
        "int8" => Some(BranchType::I8),
        "uint8" => Some(BranchType::U8),
        "int16" => Some(BranchType::I16),
        "uint16" => Some(BranchType::U16),
        "int32" => Some(BranchType::I32),
        "uint32" => Some(BranchType::U32),
        "int64" => Some(BranchType::I64),
        "uint64" => Some(BranchType::U64),
        "float" => Some(BranchType::F32),
        "vec_bool" => Some(BranchType::VecBool),
        "vec_int8" => Some(BranchType::VecI8),
        "vec_uint8" => Some(BranchType::VecU8),
        "vec_int16" => Some(BranchType::VecI16),
        "vec_uint16" => Some(BranchType::VecU16),
        "vec_int32" => Some(BranchType::VecI32),
        "vec_uint32" => Some(BranchType::VecU32),
        "vec_int64" => Some(BranchType::VecI64),
        "vec_uint64" => Some(BranchType::VecU64),
        "vec_float" => Some(BranchType::VecF32),
        _ => None,
    }
}

fn validate_raw_analysis_spec(raw: &RawAnalysisSpec) -> Result<(), ParseError> {
    if raw.analysis.year.trim().is_empty() {
        return Err(ParseError::InvalidSpec(
            "analysis is missing non-empty `year`".to_string(),
        ));
    }

    for (name, object) in &raw.objects {
        validate_identifier(name, "objects")?;
        if object.source.trim().is_empty() {
            return Err(ParseError::InvalidSpec(format!(
                "object `{name}` is missing source"
            )));
        }
    }

    for name in raw.regions.keys() {
        validate_identifier(name, "regions")?;
    }

    Ok(())
}

fn parse_catalogue(input: &str, version: &str) -> Result<Catalogue, ParseError> {
    let mut branches = BTreeMap::new();
    let mut in_version = false;
    let mut found_version = false;
    let mut in_events = false;
    let mut current_branch: Option<String> = None;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = indentation(line);

        if indent == 2 && trimmed.ends_with(':') {
            let key = trimmed.trim_end_matches(':');
            if key == version {
                in_version = true;
                found_version = true;
            } else if in_version {
                break;
            }
            continue;
        }

        if !in_version {
            continue;
        }

        if indent == 6 && trimmed == "Events:" {
            in_events = true;
            continue;
        }
        if in_events && indent == 6 && trimmed.ends_with(':') && trimmed != "Events:" {
            break;
        }
        if !in_events || trimmed == "branches:" {
            continue;
        }

        if indent == 10 && trimmed.ends_with(':') {
            let name = trimmed.trim_end_matches(':').trim();
            current_branch = Some(unquote(name).to_string());
            continue;
        }

        if indent == 12 && trimmed.starts_with("type:") {
            let Some(name) = current_branch.take() else {
                return Err(ParseError::InvalidSpec(
                    "catalogue type line before branch name".to_string(),
                ));
            };
            let raw_type = unquote(trimmed.trim_start_matches("type:").trim()).to_string();
            branches.insert(
                name,
                CatalogueBranch {
                    branch_type: parse_catalogue_branch_type(&raw_type),
                    raw_type,
                },
            );
        }
    }

    if !found_version {
        return Err(ParseError::InvalidSpec(format!(
            "missing catalogue version {version}"
        )));
    }
    if !in_events {
        return Err(ParseError::InvalidSpec(
            "missing Events tree in catalogue".to_string(),
        ));
    }
    if branches.is_empty() {
        return Err(ParseError::InvalidSpec(
            "catalogue Events tree has no branches".to_string(),
        ));
    }

    Ok(Catalogue { branches })
}

fn indentation(line: &str) -> usize {
    line.chars().take_while(|ch| *ch == ' ').count()
}

fn unquote(input: &str) -> &str {
    input
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(input)
}

fn parse_cut(input: &str, object_name: &str) -> Result<Cut, ParseError> {
    let (lhs, op, rhs) = split_comparison(input)?;
    Ok(Cut {
        lhs: parse_expr(lhs, Some(object_name))?,
        op,
        rhs: parse_quantity(rhs)?,
    })
}

fn parse_requirement(input: &str) -> Result<Requirement, ParseError> {
    let (lhs, op, rhs) = split_comparison(input)?;
    let rhs = parse_unitless_number(rhs)?;
    Ok(Requirement {
        lhs: parse_expr(lhs, None)?,
        op,
        rhs,
    })
}

fn split_comparison(input: &str) -> Result<(&str, CmpOp, &str), ParseError> {
    for (token, op) in [
        (">=", CmpOp::Ge),
        ("<=", CmpOp::Le),
        ("==", CmpOp::Eq),
        ("!=", CmpOp::Ne),
        (">", CmpOp::Gt),
        ("<", CmpOp::Lt),
    ] {
        if let Some((lhs, rhs)) = input.split_once(token) {
            let lhs = lhs.trim();
            let rhs = rhs.trim();
            if lhs.is_empty() || rhs.is_empty() {
                break;
            }
            return Ok((lhs, op, rhs));
        }
    }

    Err(ParseError::InvalidSpec(format!(
        "could not parse comparison `{input}`"
    )))
}

fn parse_expr(input: &str, default_object: Option<&str>) -> Result<Expr, ParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(ParseError::InvalidSpec("empty expression".to_string()));
    }

    if let Some(inner) = input
        .strip_prefix("abs(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return Ok(Expr::Abs(Box::new(parse_expr(inner, default_object)?)));
    }

    if let Some(inner) = input
        .strip_prefix("count(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let object = inner.trim();
        validate_identifier(object, input)?;
        return Ok(Expr::Count(object.to_string()));
    }

    if let Some(rest) = input.strip_prefix("leading(") {
        let Some((object, attr)) = rest.split_once(").") else {
            return Err(ParseError::InvalidSpec(format!(
                "could not parse leading attribute expression `{input}`"
            )));
        };
        validate_identifier(object.trim(), input)?;
        validate_identifier(attr.trim(), input)?;
        return Ok(Expr::LeadingAttr {
            object: object.trim().to_string(),
            attr: attr.trim().to_string(),
        });
    }

    if let Some((object, attr)) = input.split_once('.') {
        validate_identifier(object.trim(), input)?;
        validate_identifier(attr.trim(), input)?;
        return Ok(Expr::Attr {
            object: object.trim().to_string(),
            attr: attr.trim().to_string(),
        });
    }

    if let Some(object) = default_object {
        validate_identifier(input, input)?;
        return Ok(Expr::Attr {
            object: object.to_string(),
            attr: input.to_string(),
        });
    }

    Err(ParseError::InvalidSpec(format!(
        "expression `{input}` needs an explicit object"
    )))
}

fn parse_quantity(input: &str) -> Result<Quantity, ParseError> {
    let mut parts = input.split_whitespace();
    let value = parts
        .next()
        .ok_or_else(|| ParseError::InvalidSpec("missing quantity value".to_string()))?
        .parse::<f64>()
        .map_err(|_| ParseError::InvalidSpec(format!("invalid quantity `{input}`")))?;
    let unit = match parts.next() {
        Some("GeV") => Unit::GeV,
        Some(unit) => {
            return Err(ParseError::InvalidSpec(format!(
                "unsupported unit `{unit}` in `{input}`"
            )));
        }
        None => Unit::Dimensionless,
    };
    if let Some(extra) = parts.next() {
        return Err(ParseError::InvalidSpec(format!(
            "unexpected token `{extra}` in quantity `{input}`"
        )));
    }
    Ok(Quantity { value, unit })
}

fn parse_unitless_number(input: &str) -> Result<f64, ParseError> {
    let value = input
        .trim()
        .parse::<f64>()
        .map_err(|_| ParseError::InvalidSpec(format!("expected unitless number, got `{input}`")))?;
    Ok(value)
}

fn validate_identifier(value: &str, expression: &str) -> Result<(), ParseError> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(ParseError::InvalidSpec(format!(
            "empty identifier in `{expression}`"
        )));
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        return Err(ParseError::InvalidSpec(format!(
            "invalid identifier `{value}` in `{expression}`"
        )));
    }
    Ok(())
}

fn validate_branch_name(value: &str, expression: &str) -> Result<(), ParseError> {
    validate_identifier(value, expression)
}

#[derive(Debug, serde::Deserialize)]
struct RawAnalysisSpec {
    analysis: RawAnalysis,
    #[serde(default)]
    objects: BTreeMap<String, RawObject>,
    #[serde(default, rename = "model")]
    models: Vec<RawModel>,
    #[serde(default)]
    regions: BTreeMap<String, RawRegion>,
    #[serde(default)]
    outputs: Vec<RawOutput>,
}

#[derive(Debug, serde::Deserialize)]
struct RawAnalysis {
    name: String,
    year: String,
}

#[derive(Debug, serde::Deserialize)]
struct RawObject {
    source: String,
    #[serde(default)]
    cuts: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct RawModel {
    name: String,
    #[serde(default)]
    inputs: Vec<String>,
    output: String,
    #[serde(default)]
    dtype: Option<String>,
    batch: String,
    #[serde(default)]
    provider: Option<RawModelProvider>,
}

#[derive(Debug, serde::Deserialize)]
struct RawModelProvider {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    launch: Option<String>,
    #[serde(default)]
    onnx_path: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct RawRegion {
    #[serde(default)]
    require: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct RawOutput {
    name: String,
    expr: String,
}

fn model_def_from_raw(raw: RawModel) -> Result<ModelDef, ParseError> {
    validate_identifier(&raw.name, "model name")?;
    for input in &raw.inputs {
        validate_branch_name(input, &format!("model `{}` input", raw.name))?;
    }
    validate_branch_name(&raw.output, &format!("model `{}` output", raw.name))?;
    validate_identifier(&raw.batch, &format!("model `{}` batch", raw.name))?;

    let output_dtype = match raw.dtype.as_deref() {
        None | Some("F32" | "f32" | "float" | "Float") => ModelOutputDType::F32,
        Some(dtype) => {
            return Err(ParseError::InvalidSpec(format!(
                "model `{}` has unsupported output dtype `{dtype}`; expected F32",
                raw.name
            )));
        }
    };

    Ok(ModelDef {
        name: raw.name,
        inputs: raw.inputs,
        output: raw.output,
        output_dtype,
        batch: raw.batch,
        provider: raw
            .provider
            .map(model_provider_from_raw)
            .unwrap_or_else(ModelProviderSpec::mock),
    })
}

fn model_provider_from_raw(raw: RawModelProvider) -> ModelProviderSpec {
    let kind = match raw.kind.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("mock") => ModelProviderKind::Mock,
        Some("inproc") => ModelProviderKind::InProcess,
        Some("remote") => ModelProviderKind::Remote,
        Some("managed") => ModelProviderKind::Managed,
        Some(other) => ModelProviderKind::Other(other.to_string()),
        None => ModelProviderKind::Other(String::new()),
    };

    ModelProviderSpec {
        kind,
        endpoint: raw.endpoint,
        launch: raw.launch,
        onnx_path: raw.onnx_path,
    }
}

/// Spec/cut parsing errors.
#[derive(Debug)]
pub enum ParseError {
    InvalidSpec(String),
    UnsupportedFormat { path: PathBuf },
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpec(message) => f.write_str(message),
            Self::UnsupportedFormat { path } => write!(
                f,
                "unsupported spec format for `{}`; expected .toml, .yaml, .yml, or .json",
                path.display()
            ),
            Self::Io { path, source } => {
                write!(f, "failed to read spec `{}`: {source}", path.display())
            }
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::InvalidSpec(_) | Self::UnsupportedFormat { .. } => None,
        }
    }
}

/// Static validation errors produced before any event loop starts.
#[derive(Debug, Clone, PartialEq)]
pub enum SpecError {
    MissingBranch {
        context: String,
        branch: String,
    },
    UnsupportedBranchType {
        context: String,
        branch: String,
        raw_type: String,
    },
    WrongBranchType {
        context: String,
        branch: String,
        expected: String,
        actual: BranchType,
    },
    MissingUnit {
        context: String,
        expr: String,
        expected: Unit,
    },
    UnitMismatch {
        context: String,
        expr: String,
        expected: Dimension,
        actual: Unit,
    },
    UndefinedObject {
        context: String,
        object: String,
    },
    UndefinedBatch {
        context: String,
        batch: String,
    },
    ModelOutputCollision {
        context: String,
        output: String,
    },
    InvalidModel {
        context: String,
        detail: String,
    },
    InvalidProvider {
        context: String,
        detail: String,
    },
    InvalidExpression {
        context: String,
        detail: String,
    },
    InvalidReadSchema {
        detail: String,
    },
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBranch { context, branch } => {
                write!(f, "{context}: missing branch `{branch}`")
            }
            Self::UnsupportedBranchType {
                context,
                branch,
                raw_type,
            } => write!(
                f,
                "{context}: branch `{branch}` has unsupported catalogue type `{raw_type}`"
            ),
            Self::WrongBranchType {
                context,
                branch,
                expected,
                actual,
            } => write!(
                f,
                "{context}: branch `{branch}` has type {actual:?}, expected {expected}"
            ),
            Self::MissingUnit {
                context,
                expr,
                expected,
            } => write!(
                f,
                "{context}: `{expr}` comparison is missing required unit {expected}"
            ),
            Self::UnitMismatch {
                context,
                expr,
                expected,
                actual,
            } => write!(
                f,
                "{context}: `{expr}` has unit {actual}, expected dimension {expected:?}"
            ),
            Self::UndefinedObject { context, object } => {
                write!(f, "{context}: undefined object `{object}`")
            }
            Self::UndefinedBatch { context, batch } => {
                write!(f, "{context}: undefined batch `{batch}`")
            }
            Self::ModelOutputCollision { context, output } => {
                write!(
                    f,
                    "{context}: model output `{output}` collides with an existing column"
                )
            }
            Self::InvalidModel { context, detail } => write!(f, "{context}: {detail}"),
            Self::InvalidProvider { context, detail } => write!(f, "{context}: {detail}"),
            Self::InvalidExpression { context, detail } => write!(f, "{context}: {detail}"),
            Self::InvalidReadSchema { detail } => write!(f, "invalid read schema: {detail}"),
        }
    }
}

impl Error for SpecError {}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GeV => f.write_str("GeV"),
            Self::Dimensionless => f.write_str("dimensionless"),
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Attr { object, attr } => write!(f, "{object}.{attr}"),
            Self::Abs(inner) => write!(f, "abs({inner})"),
            Self::Count(object) => write!(f, "count({object})"),
            Self::LeadingAttr { object, attr } => write!(f, "leading({object}).{attr}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MUON_SPEC_TOML: &str = include_str!("../examples/muon.toml");
    const MUON_TAGGER_SPEC_TOML: &str = include_str!("../examples/muon_tagger.toml");
    const MUON_SPEC_YAML: &str = include_str!("../examples/muon.yaml");
    const MUON_SPEC_JSON: &str = r#"
{
  "analysis": { "name": "muon_demo", "year": "Run2018" },
  "objects": {
    "good_muon": {
      "source": "Muon",
      "cuts": ["pt > 30 GeV", "abs(eta) < 2.4"]
    }
  },
  "regions": {
    "signal": {
      "require": ["count(good_muon) >= 1"]
    }
  },
  "outputs": [
    { "name": "n_good_muon", "expr": "count(good_muon)" },
    { "name": "lead_muon_pt", "expr": "leading(good_muon).pt" }
  ]
}
"#;
    const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");

    fn catalogue() -> Catalogue {
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse nanov9 catalogue")
    }

    fn parse_muon_spec() -> AnalysisSpec {
        AnalysisSpec::from_toml_str(MUON_SPEC_TOML).expect("parse muon spec")
    }

    #[test]
    fn parse_muon_spec_into_typed_ir() {
        let spec = parse_muon_spec();

        assert_eq!(spec.name, "muon_demo");
        assert_eq!(spec.year, Year::Run2018);
        assert_eq!(spec.objects[0].name, "good_muon");
        assert_eq!(spec.objects[0].source, "Muon");
        assert_eq!(
            spec.objects[0].cuts[0],
            Cut {
                lhs: Expr::Attr {
                    object: "good_muon".to_string(),
                    attr: "pt".to_string(),
                },
                op: CmpOp::Gt,
                rhs: Quantity {
                    value: 30.0,
                    unit: Unit::GeV,
                },
            }
        );
        assert_eq!(
            spec.outputs[1].expr,
            Expr::LeadingAttr {
                object: "good_muon".to_string(),
                attr: "pt".to_string(),
            }
        );
    }

    #[test]
    fn validation_derives_muon_read_branches() {
        let spec = parse_muon_spec();
        let plan = validate(&spec, &catalogue()).expect("validate muon spec");
        let read_branches = plan
            .read_branches
            .specs()
            .iter()
            .map(|spec| (spec.name.as_str(), spec.branch_type))
            .collect::<Vec<_>>();

        assert_eq!(
            read_branches,
            vec![
                ("nMuon", BranchType::U32),
                ("Muon_eta", BranchType::VecF32),
                ("Muon_pt", BranchType::VecF32),
            ]
        );
    }

    #[test]
    fn parses_model_binding_with_default_mock_provider() {
        let spec = AnalysisSpec::from_toml_str(MUON_TAGGER_SPEC_TOML).expect("parse model spec");

        assert_eq!(spec.models.len(), 1);
        assert_eq!(spec.models[0].name, "muon_tagger");
        assert_eq!(
            spec.models[0].inputs,
            vec!["Muon_pt", "Muon_eta", "Muon_phi"]
        );
        assert_eq!(spec.models[0].output, "Muon_topscore");
        assert_eq!(spec.models[0].output_dtype, ModelOutputDType::F32);
        assert_eq!(spec.models[0].provider.kind, ModelProviderKind::Mock);
    }

    #[test]
    fn validation_accepts_model_output_and_derives_model_inputs() {
        let spec = AnalysisSpec::from_toml_str(MUON_TAGGER_SPEC_TOML).expect("parse model spec");
        let plan = validate(&spec, &catalogue()).expect("validate model spec");
        let read_branches = plan
            .read_branches
            .specs()
            .iter()
            .map(|spec| (spec.name.as_str(), spec.branch_type))
            .collect::<Vec<_>>();

        assert_eq!(
            read_branches,
            vec![
                ("nMuon", BranchType::U32),
                ("Muon_eta", BranchType::VecF32),
                ("Muon_pt", BranchType::VecF32),
                ("Muon_phi", BranchType::VecF32),
            ]
        );
    }

    #[test]
    fn validation_rejects_unproduced_score_reference() {
        let spec_text = MUON_TAGGER_SPEC_TOML.replacen(
            "output = \"Muon_topscore\"",
            "output = \"Muon_other_score\"",
            1,
        );
        let spec = AnalysisSpec::from_toml_str(&spec_text).expect("parse modified spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::MissingBranch { branch, .. } if branch == "Muon_topscore"
        )));
    }

    #[test]
    fn validation_rejects_malformed_model_provider() {
        let spec_text = MUON_TAGGER_SPEC_TOML.replace(
            "kind = \"mock\"",
            "kind = \"remote\"\nendpoint = \"not a url\"",
        );
        let spec = AnalysisSpec::from_toml_str(&spec_text).expect("parse modified spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::InvalidProvider { detail, .. }
                if detail.contains("not a valid URL")
        )));
    }

    #[test]
    fn muon_toml_and_yaml_parse_to_same_typed_ir_and_plan() {
        let toml_spec = AnalysisSpec::from_toml_str(MUON_SPEC_TOML).expect("parse TOML spec");
        let yaml_spec = AnalysisSpec::from_yaml_str(MUON_SPEC_YAML).expect("parse YAML spec");

        assert_eq!(toml_spec, yaml_spec);

        let catalogue = catalogue();
        let toml_plan = validate(&toml_spec, &catalogue).expect("validate TOML spec");
        let yaml_plan = validate(&yaml_spec, &catalogue).expect("validate YAML spec");

        assert_eq!(toml_plan.spec, yaml_plan.spec);
        assert_eq!(
            toml_plan.read_branches.specs(),
            yaml_plan.read_branches.specs()
        );
    }

    #[test]
    fn json_spec_uses_same_serde_surface() {
        let toml_spec = AnalysisSpec::from_toml_str(MUON_SPEC_TOML).expect("parse TOML spec");
        let json_spec = AnalysisSpec::from_json_str(MUON_SPEC_JSON).expect("parse JSON spec");

        assert_eq!(json_spec, toml_spec);
    }

    #[test]
    fn spec_format_dispatches_by_file_extension() {
        assert_eq!(
            SpecFormat::from_path("analysis.toml").unwrap(),
            SpecFormat::Toml
        );
        assert_eq!(
            SpecFormat::from_path("analysis.yaml").unwrap(),
            SpecFormat::Yaml
        );
        assert_eq!(
            SpecFormat::from_path("analysis.yml").unwrap(),
            SpecFormat::Yaml
        );
        assert_eq!(
            SpecFormat::from_path("analysis.json").unwrap(),
            SpecFormat::Json
        );
        assert!(matches!(
            SpecFormat::from_path("analysis.txt"),
            Err(ParseError::UnsupportedFormat { .. })
        ));
    }

    #[test]
    fn validation_rejects_nonexistent_branch() {
        let yaml = MUON_SPEC_YAML.replace("abs(eta) < 2.4", "abs(nope) < 2.4");
        let spec = AnalysisSpec::from_yaml_str(&yaml).expect("parse modified spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::MissingBranch { branch, .. } if branch == "Muon_nope"
        )));
        assert!(errors
            .iter()
            .any(|error| error.to_string().contains("missing branch `Muon_nope`")));
    }

    #[test]
    fn validation_rejects_missing_unit() {
        let yaml = MUON_SPEC_YAML.replace("pt > 30 GeV", "pt > 30");
        let spec = AnalysisSpec::from_yaml_str(&yaml).expect("parse modified spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::MissingUnit {
                expr,
                expected: Unit::GeV,
                ..
            } if expr == "good_muon.pt"
        )));
    }

    #[test]
    fn validation_rejects_region_with_undefined_object() {
        let yaml = MUON_SPEC_YAML.replace("count(good_muon) >= 1", "count(ghost_muon) >= 1");
        let spec = AnalysisSpec::from_yaml_str(&yaml).expect("parse modified spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::UndefinedObject { object, .. } if object == "ghost_muon"
        )));
    }

    #[test]
    fn validation_rejects_wrong_branch_type() {
        let yaml = MUON_SPEC_YAML.replace("abs(eta) < 2.4", "looseId > 0");
        let spec = AnalysisSpec::from_yaml_str(&yaml).expect("parse modified spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::WrongBranchType {
                branch,
                actual: BranchType::VecBool,
                ..
            } if branch == "Muon_looseId"
        )));
    }
}
