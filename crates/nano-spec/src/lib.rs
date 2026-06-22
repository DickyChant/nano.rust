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

mod adl;
pub mod certificate;
pub mod codegen;
pub mod core;
pub mod interpret;
pub mod kir;

/// Typed semantic analysis specification.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AnalysisSpec {
    pub name: String,
    pub year: Year,
    pub objects: Vec<ObjectDef>,
    pub derived_objects: Vec<DerivedObjectDef>,
    pub models: Vec<ModelDef>,
    pub regions: Vec<RegionDef>,
    pub outputs: Vec<OutputDef>,
    pub histograms: Vec<HistogramDef>,
    pub weight: WeightDef,
    pub systematics: Vec<SystematicDef>,
    pub shape_corrections: Vec<ShapeCorrectionDef>,
    pub channels: Vec<ChannelDef>,
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

    /// Parse an analysis specification from the physics-facing ADL form.
    pub fn from_adl_str(input: &str) -> Result<Self, ParseError> {
        parse_analysis_spec_with_format(input, SpecFormat::Adl)
    }

    /// Load an analysis specification from a file, dispatching by extension.
    ///
    /// Supported extensions are `.toml`, `.yaml`, `.yml`, `.json`, and `.adl`.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ParseError> {
        load_analysis_spec(path)
    }

    /// The declared weight systematic, if this spec requests variation fan-out.
    pub fn weight_systematic(&self) -> Option<&WeightSystematicDef> {
        self.systematics
            .iter()
            .find_map(|systematic| match systematic {
                SystematicDef::Weight(def) => Some(def),
                SystematicDef::Nominal
                | SystematicDef::JesUp
                | SystematicDef::JesDown
                | SystematicDef::JerUp
                | SystematicDef::JerDown => None,
            })
    }

    /// Whether histogram fills should fan out over nominal/up/down weights.
    pub fn has_weight_systematic(&self) -> bool {
        self.weight_systematic().is_some()
    }

    /// The declared collection-attribute shape corrections.
    pub fn shape_corrections(&self) -> &[ShapeCorrectionDef] {
        &self.shape_corrections
    }

    /// Whether any systematic variation changes selected object kinematics.
    pub fn has_shape_correction(&self) -> bool {
        !self.shape_corrections.is_empty()
    }

    /// Whether histogram storage must keep a systematic axis.
    pub fn has_histogram_systematic(&self) -> bool {
        self.has_weight_systematic() || self.has_shape_correction()
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
    Adl,
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
            Some("adl") => Ok(Self::Adl),
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
            Self::Adl => f.write_str("ADL"),
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

/// Derived object definition, such as a selected dimuon pair built from muons.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DerivedObjectDef {
    pub name: String,
    pub source: DerivedSource,
}

/// Source operation for a derived object.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DerivedSource {
    Pair(ObjectPairDef),
    Candidate(ObjectCandidateDef),
}

/// Pair combinatorics over one selected object collection.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ObjectPairDef {
    pub object: String,
    pub constraints: Vec<PairConstraint>,
    pub filters: Vec<Cut>,
    pub selection: PairSelection,
    pub exclude: Vec<String>,
}

/// Candidate assembly from selected objects and/or previously derived objects.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ObjectCandidateDef {
    pub items: Vec<String>,
    pub filters: Vec<Cut>,
}

/// Pair-level constraints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PairConstraint {
    OppositeCharge,
    SameFlavor,
}

/// Rule used to choose one pair candidate from all valid combinations.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PairSelection {
    /// Sort selected objects by pT and take the first valid pair in that order.
    LeadingPt,
    /// Choose the valid pair whose invariant mass is closest to a target.
    NearestMass { target: Quantity },
    /// Match ROOT df103's helper: scan source order and compare against the
    /// previously stored candidate mass after truncating it to an integer.
    NearestMassTruncated { target: Quantity },
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

/// Arithmetic operators inside numeric expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

/// Expression nodes for the semantic selection IR.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Expr {
    Attr {
        object: String,
        attr: String,
    },
    Literal(f64),
    Binary {
        op: ArithOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Abs(Box<Expr>),
    Sqrt(Box<Expr>),
    Count(String),
    CountWhere {
        object: String,
        predicate: Box<Cut>,
    },
    SumAttr {
        object: String,
        attr: String,
    },
    All {
        object: String,
        predicate: Box<Cut>,
    },
    Any {
        object: String,
        predicate: Box<Cut>,
    },
    EitherPairPt {
        left: String,
        right: String,
        leading: Quantity,
        subleading: Quantity,
    },
    ClosestMass {
        left: String,
        right: String,
        target: Quantity,
    },
    OtherMass {
        left: String,
        right: String,
        target: Quantity,
    },
    LeadingAttr {
        object: String,
        attr: String,
    },
    PairDeltaR,
    PairLeadingPt,
    PairSubleadingPt,
    CandidateMinDeltaR,
    CandidateLeadingPt,
    CandidateSubleadingPt,
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
    pub rhs: Quantity,
}

/// Named region definition.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RegionDef {
    pub name: String,
    pub require: Vec<Requirement>,
}

/// Named output expression.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OutputDef {
    pub name: String,
    pub expr: Expr,
}

/// Histogram terminal requested by the spec.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HistogramDef {
    pub name: String,
    pub expr: Expr,
    pub bins: usize,
    pub range: [f64; 2],
}

/// Multiplicative event-weight factors.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WeightDef {
    pub nominal: Vec<f64>,
}

/// Systematic variations requested by a spec.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum SystematicDef {
    Nominal,
    JesUp,
    JesDown,
    JerUp,
    JerDown,
    Weight(WeightSystematicDef),
}

/// A two-sided normalization/weight systematic.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WeightSystematicDef {
    pub name: String,
    pub up: f64,
    pub down: f64,
}

/// A two-sided shape correction that scales one selected collection attribute.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShapeCorrectionDef {
    pub name: String,
    pub collection: String,
    pub attr: String,
    pub up: f64,
    pub down: f64,
}

/// One channel inside a multi-channel union spec.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChannelDef {
    pub name: String,
    pub objects: Vec<ObjectDef>,
    pub derived_objects: Vec<DerivedObjectDef>,
    pub regions: Vec<RegionDef>,
    pub outputs: Vec<OutputDef>,
}

impl ChannelDef {
    fn as_spec(&self, parent: &AnalysisSpec) -> AnalysisSpec {
        AnalysisSpec {
            name: format!("{}_{}", parent.name, self.name),
            year: parent.year.clone(),
            objects: self.objects.clone(),
            derived_objects: self.derived_objects.clone(),
            models: Vec::new(),
            regions: self.regions.clone(),
            outputs: self.outputs.clone(),
            histograms: parent.histograms.clone(),
            weight: parent.weight.clone(),
            systematics: parent.systematics.clone(),
            shape_corrections: parent.shape_corrections.clone(),
            channels: Vec::new(),
        }
    }
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
        SpecFormat::Adl => adl::parse_adl(input),
    }
}

fn analysis_spec_from_raw(raw: RawAnalysisSpec) -> Result<AnalysisSpec, ParseError> {
    validate_raw_analysis_spec(&raw)?;
    let objects = object_defs_from_raw(raw.objects)?;
    let derived_objects = derived_defs_from_raw(raw.derived)?;
    let models = raw
        .models
        .into_iter()
        .map(model_def_from_raw)
        .collect::<Result<Vec<_>, _>>()?;
    let regions = region_defs_from_raw(raw.regions)?;
    let outputs = output_defs_from_raw(&raw.outputs)?;
    let histograms = histogram_defs_from_raw(&raw.histograms)?;
    let weight = raw.weight.map(weight_def_from_raw).unwrap_or_default();
    let shape_corrections = raw
        .corrections
        .into_iter()
        .map(shape_correction_def_from_raw)
        .collect::<Result<Vec<_>, _>>()?;
    let mut systematics = raw
        .systematics
        .iter()
        .map(|systematic| systematic_def_from_raw(systematic))
        .collect::<Result<Vec<_>, _>>()?;
    for systematic in raw.systematic {
        systematics.push(weight_systematic_def_from_raw(systematic)?);
    }
    let systematics = if systematics.is_empty() {
        vec![SystematicDef::Nominal]
    } else {
        let needs_nominal = systematics
            .iter()
            .any(|systematic| matches!(systematic, SystematicDef::Weight(_)))
            || !shape_corrections.is_empty();
        if needs_nominal
            && !systematics
                .iter()
                .any(|systematic| matches!(systematic, SystematicDef::Nominal))
        {
            systematics.insert(0, SystematicDef::Nominal);
        }
        systematics
    };
    let channels = raw
        .channels
        .into_iter()
        .map(channel_def_from_raw)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(AnalysisSpec {
        name: raw.analysis.name,
        year: Year::parse(&raw.analysis.year),
        objects,
        derived_objects,
        models,
        regions,
        outputs,
        histograms,
        weight,
        systematics,
        shape_corrections,
        channels,
    })
}

/// Lower a typed surface spec into typed Core IR.
pub fn lower(spec: &AnalysisSpec, catalogue: &Catalogue) -> Result<core::CoreIr, Vec<SpecError>> {
    if !spec.channels.is_empty() {
        return lower_union(spec, catalogue);
    }

    let (required, model_outputs) = validate_flat(spec, catalogue)?;
    build_core_ir(spec, &required, &model_outputs)
}

/// Validate a typed spec against a branch catalogue and derive the read schema.
pub fn validate(
    spec: &AnalysisSpec,
    catalogue: &Catalogue,
) -> Result<ResolvedPlan, Vec<SpecError>> {
    if !spec.channels.is_empty() {
        return validate_union(spec, catalogue);
    }

    let core = lower(spec, catalogue)?;
    let branch_specs =
        branch_specs_from_core_effects(&core, catalogue).map_err(|error| vec![error])?;
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

fn validate_flat(
    spec: &AnalysisSpec,
    catalogue: &Catalogue,
) -> Result<(RequiredBranches, ModelOutputs), Vec<SpecError>> {
    let object_sources = spec
        .objects
        .iter()
        .map(|object| (object.name.as_str(), object.source.as_str()))
        .collect::<HashMap<_, _>>();
    let derived_objects = spec
        .derived_objects
        .iter()
        .map(|object| (object.name.as_str(), object))
        .collect::<HashMap<_, _>>();
    let mut errors = Vec::new();
    let mut required = RequiredBranches::default();
    let model_outputs =
        validate_models(spec, catalogue, &object_sources, &mut required, &mut errors);
    validate_unique_output_names(&spec.outputs, &mut errors);

    {
        let mut ctx = ValidationContext {
            catalogue,
            object_sources: &object_sources,
            derived_objects: &derived_objects,
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

        validate_shape_corrections(spec, &mut ctx);

        for derived in &spec.derived_objects {
            validate_derived_object(derived, &mut ctx);
        }

        for region in &spec.regions {
            for (index, requirement) in region.require.iter().enumerate() {
                validate_requirement(region, index, requirement, &mut ctx);
            }
        }

        for output in &spec.outputs {
            validate_expr(&output.expr, &format!("output `{}`", output.name), &mut ctx);
        }

        for histogram in &spec.histograms {
            validate_histogram(histogram, &mut ctx);
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok((required, model_outputs))
}

fn validate_unique_output_names(outputs: &[OutputDef], errors: &mut Vec<SpecError>) {
    let mut seen = BTreeSet::new();
    for output in outputs {
        if !seen.insert(output.name.as_str()) {
            errors.push(SpecError::InvalidExpression {
                context: format!("output `{}`", output.name),
                detail: format!("duplicate output name `{}`", output.name),
            });
        }
    }
}

fn lower_union(spec: &AnalysisSpec, catalogue: &Catalogue) -> Result<core::CoreIr, Vec<SpecError>> {
    let mut errors = Vec::new();
    if !spec.models.is_empty() {
        errors.push(SpecError::InvalidExpression {
            context: "multi-channel union".to_string(),
            detail: "model-aware union codegen is not yet supported".to_string(),
        });
    }
    if !spec.objects.is_empty() || !spec.derived_objects.is_empty() || !spec.regions.is_empty() {
        errors.push(SpecError::InvalidExpression {
            context: "multi-channel union".to_string(),
            detail: "declare objects, derived objects, and regions inside [[channel]] blocks"
                .to_string(),
        });
    }

    let Some(first) = spec.channels.first() else {
        unreachable!("caller checks channels is non-empty");
    };
    let first_schema = output_schema(&first.outputs);
    let mut builder = core::CoreBuilder::new(&spec.name);
    for channel in &spec.channels {
        if channel.outputs.is_empty() {
            errors.push(SpecError::InvalidExpression {
                context: format!("channel `{}`", channel.name),
                detail: "channels in a union must declare at least one output".to_string(),
            });
        }
        if output_schema(&channel.outputs) != first_schema {
            errors.push(SpecError::InvalidExpression {
                context: format!("channel `{}`", channel.name),
                detail: "channel output schema must match the first channel by name and order"
                    .to_string(),
            });
        }

        let channel_spec = channel.as_spec(spec);
        match lower(&channel_spec, catalogue) {
            Ok(channel_core) => {
                for effect in channel_core.effects {
                    builder.add_effect(effect);
                }
            }
            Err(channel_errors) => errors.extend(channel_errors),
        }
    }

    if errors.is_empty() {
        Ok(builder.finish())
    } else {
        Err(errors)
    }
}

fn branch_specs_from_core_effects(
    core: &core::CoreIr,
    catalogue: &Catalogue,
) -> Result<Vec<BranchSpec>, SpecError> {
    let branches = core.read_branches_ordered();
    let mut specs = Vec::with_capacity(branches.len());
    let mut seen = BTreeSet::new();

    for branch in branches {
        if !seen.insert(branch.to_string()) {
            continue;
        }
        let branch_type = catalogue_branch_type(catalogue, branch, "derived read_branches")?;
        specs.push(BranchSpec::new(branch.to_string(), branch_type));
    }

    Ok(specs)
}

fn build_core_ir(
    spec: &AnalysisSpec,
    required: &RequiredBranches,
    model_outputs: &ModelOutputs,
) -> Result<core::CoreIr, Vec<SpecError>> {
    let lowerer = CoreLowerer::new(spec, model_outputs);
    lowerer.lower_spec(required)
}

struct CoreLowerer<'a> {
    spec: &'a AnalysisSpec,
    model_outputs: &'a ModelOutputs,
    registry: core::PrimitiveRegistry,
    builder: core::CoreBuilder,
    object_ids: HashMap<&'a str, core::ObjectId>,
    object_sources: HashMap<&'a str, &'a str>,
    model_ids: HashMap<&'a str, core::ModelId>,
    errors: Vec<SpecError>,
}

impl<'a> CoreLowerer<'a> {
    fn new(spec: &'a AnalysisSpec, model_outputs: &'a ModelOutputs) -> Self {
        Self {
            spec,
            model_outputs,
            registry: core::PrimitiveRegistry::standard(),
            builder: core::CoreBuilder::new(&spec.name),
            object_ids: HashMap::new(),
            object_sources: HashMap::new(),
            model_ids: HashMap::new(),
            errors: Vec::new(),
        }
    }

    fn lower_spec(mut self, required: &RequiredBranches) -> Result<core::CoreIr, Vec<SpecError>> {
        for object in &self.spec.objects {
            let id = self
                .builder
                .add_object(&object.name, Some(object.source.clone()));
            self.object_ids.insert(&object.name, id);
            self.object_sources.insert(&object.name, &object.source);
        }
        for derived in &self.spec.derived_objects {
            let id = self.builder.add_object(&derived.name, None);
            self.object_ids.insert(&derived.name, id);
        }
        for model in &self.spec.models {
            let id = self.builder.add_model(&model.name, &model.output);
            self.model_ids.insert(&model.name, id);
            self.builder.add_effect(core::Effect::RequiresModel(id));
            self.builder
                .add_effect(core::Effect::ProducesScore(model.output.clone()));
        }
        for region in &self.spec.regions {
            self.builder.add_region(&region.name);
        }
        self.add_required_branch_effects(required);

        for object in &self.spec.objects {
            for cut in &object.cuts {
                self.lower_cut(cut, &format!("object `{}` cut", object.name));
            }
        }
        for derived in &self.spec.derived_objects {
            self.lower_derived_object(derived);
        }
        for region in &self.spec.regions {
            for requirement in &region.require {
                self.lower_requirement(requirement, &format!("region `{}`", region.name));
            }
        }
        for output in &self.spec.outputs {
            if let Some(expr) = self.lower_expr(&output.expr, &format!("output `{}`", output.name))
            {
                self.builder.add_output(&output.name, expr);
            }
        }
        for histogram in &self.spec.histograms {
            if let Some(expr) =
                self.lower_expr(&histogram.expr, &format!("histogram `{}`", histogram.name))
            {
                self.builder.add_histogram(&histogram.name, expr);
            }
        }

        if self.errors.is_empty() {
            Ok(self.builder.finish())
        } else {
            Err(self.errors)
        }
    }

    fn add_required_branch_effects(&mut self, required: &RequiredBranches) {
        for source in &required.counters {
            self.builder
                .add_effect(core::Effect::ReadsBranch(format!("n{source}")));
        }
        for (source, attr) in &required.attrs {
            self.builder
                .add_effect(core::Effect::ReadsBranch(format!("{source}_{attr}")));
        }
        for branch in &required.branches {
            self.builder
                .add_effect(core::Effect::ReadsBranch(branch.clone()));
        }
    }

    fn lower_derived_object(&mut self, derived: &DerivedObjectDef) {
        let context = format!("derived object `{}`", derived.name);
        match &derived.source {
            DerivedSource::Pair(pair) => {
                let Some(object) = self.object_ref(&pair.object, &context) else {
                    return;
                };
                let primitive = match &pair.selection {
                    PairSelection::LeadingPt => "pair",
                    PairSelection::NearestMass { target } => {
                        let target = self.quantity_node(target);
                        self.call("nearest_mass", vec![object, target], &context);
                        return;
                    }
                    PairSelection::NearestMassTruncated { target } => {
                        let target = self.quantity_node(target);
                        self.call("nearest_mass_truncated", vec![object, target], &context);
                        return;
                    }
                };
                self.call(primitive, vec![object], &context);
                for filter in &pair.filters {
                    self.lower_cut(filter, &context);
                }
            }
            DerivedSource::Candidate(candidate) => {
                let mut args = Vec::new();
                for item in &candidate.items {
                    if self.object_sources.contains_key(item.as_str()) {
                        if let Some(object) = self.object_ref(item, &context) {
                            args.push(object);
                        }
                    } else if let Some(candidate) = self.candidate_ref(item, &context) {
                        args.push(candidate);
                    }
                }
                self.call("combine", args, &context);
                for filter in &candidate.filters {
                    self.lower_cut(filter, &context);
                }
            }
        }
    }

    fn lower_requirement(&mut self, requirement: &Requirement, context: &str) {
        let Some(lhs) = self.lower_expr(&requirement.lhs, context) else {
            return;
        };
        let rhs = self.quantity_node(&requirement.rhs);
        self.compare(requirement.op, lhs, rhs, context);
    }

    fn lower_cut(&mut self, cut: &Cut, context: &str) {
        let Some(lhs) = self.lower_expr(&cut.lhs, context) else {
            return;
        };
        let rhs = self.quantity_node(&cut.rhs);
        self.compare(cut.op, lhs, rhs, context);
    }

    fn lower_expr(&mut self, expr: &Expr, context: &str) -> Option<core::ExprId> {
        match expr {
            Expr::Attr { object, attr } => self.attr_node(object, attr, context),
            Expr::Literal(value) => {
                self.registry_call("literal", &[], context)?;
                Some(self.builder.add_expr(
                    core::ExprKind::Literal(*value),
                    core::Type::Quantity(Dimension::Dimensionless),
                    BTreeSet::new(),
                ))
            }
            Expr::Binary { op, lhs, rhs } => {
                let lhs = self.lower_expr(lhs, context)?;
                let rhs = self.lower_expr(rhs, context)?;
                self.call(
                    core::primitive_name_for_arithmetic(*op),
                    vec![lhs, rhs],
                    context,
                )
            }
            Expr::Abs(inner) => {
                let inner = self.lower_expr(inner, context)?;
                self.call("abs", vec![inner], context)
            }
            Expr::Sqrt(inner) => {
                let inner = self.lower_expr(inner, context)?;
                self.call("sqrt", vec![inner], context)
            }
            Expr::Count(object) => {
                let object = self.object_ref(object, context)?;
                self.call("count", vec![object], context)
            }
            Expr::CountWhere { object, predicate } => {
                let object = self.object_ref(object, context)?;
                let predicate = self.lower_cut_expr(predicate, context)?;
                self.call("count_where", vec![object, predicate], context)
            }
            Expr::SumAttr { object, attr } => {
                let object_ref = self.object_ref(object, context)?;
                let attr = self.attr_node(object, attr, context)?;
                self.call("sum", vec![object_ref, attr], context)
            }
            Expr::All { object, predicate } => {
                let object = self.object_ref(object, context)?;
                let predicate = self.lower_cut_expr(predicate, context)?;
                self.call("all", vec![object, predicate], context)
            }
            Expr::Any { object, predicate } => {
                let object = self.object_ref(object, context)?;
                let predicate = self.lower_cut_expr(predicate, context)?;
                self.call("any", vec![object, predicate], context)
            }
            Expr::EitherPairPt {
                left,
                right,
                leading,
                subleading,
            } => {
                let left = self.object_ref(left, context)?;
                let right = self.object_ref(right, context)?;
                let leading = self.quantity_node(leading);
                let subleading = self.quantity_node(subleading);
                self.call(
                    "either_pair_pt",
                    vec![left, right, leading, subleading],
                    context,
                )
            }
            Expr::ClosestMass {
                left,
                right,
                target,
            } => {
                let left = self.candidate_ref(left, context)?;
                let right = self.candidate_ref(right, context)?;
                let target = self.quantity_node(target);
                self.call("closest_mass", vec![left, right, target], context)
            }
            Expr::OtherMass {
                left,
                right,
                target,
            } => {
                let left = self.candidate_ref(left, context)?;
                let right = self.candidate_ref(right, context)?;
                let target = self.quantity_node(target);
                self.call("other_mass", vec![left, right, target], context)
            }
            Expr::LeadingAttr { object, attr } => {
                let object_ref = self.object_ref(object, context)?;
                let attr = self.attr_node(object, attr, context)?;
                self.call("leading_attr", vec![object_ref, attr], context)
            }
            Expr::PairDeltaR => self.filter_call("pair_delta_r", context),
            Expr::PairLeadingPt => self.filter_call("pair_leading_pt", context),
            Expr::PairSubleadingPt => self.filter_call("pair_subleading_pt", context),
            Expr::CandidateMinDeltaR => self.filter_call("min_delta_r", context),
            Expr::CandidateLeadingPt => self.filter_call("candidate_leading_pt", context),
            Expr::CandidateSubleadingPt => self.filter_call("candidate_subleading_pt", context),
        }
    }

    fn lower_cut_expr(&mut self, cut: &Cut, context: &str) -> Option<core::ExprId> {
        let lhs = self.lower_expr(&cut.lhs, context)?;
        let rhs = self.quantity_node(&cut.rhs);
        self.compare(cut.op, lhs, rhs, context)
    }

    fn object_ref(&mut self, object: &str, context: &str) -> Option<core::ExprId> {
        let id = self.lookup_object(object, context)?;
        self.call_ref(
            "object",
            id,
            core::Type::ObjectSet,
            BTreeSet::new(),
            context,
        )
    }

    fn candidate_ref(&mut self, object: &str, context: &str) -> Option<core::ExprId> {
        let id = self.lookup_object(object, context)?;
        self.call_ref(
            "candidate",
            id,
            core::Type::Candidate,
            BTreeSet::new(),
            context,
        )
    }

    fn lookup_object(&mut self, object: &str, context: &str) -> Option<core::ObjectId> {
        self.object_ids.get(object).copied().or_else(|| {
            self.errors.push(SpecError::UndefinedObject {
                context: context.to_string(),
                object: object.to_string(),
            });
            None
        })
    }

    fn call_ref(
        &mut self,
        primitive: &'static str,
        _object: core::ObjectId,
        ty: core::Type,
        effects: BTreeSet<core::Effect>,
        context: &str,
    ) -> Option<core::ExprId> {
        self.registry_call(primitive, &[], context)?;
        Some(self.builder.add_expr(
            core::ExprKind::Call {
                primitive,
                args: Vec::new(),
            },
            ty,
            effects,
        ))
    }

    fn attr_node(&mut self, object: &str, attr: &str, context: &str) -> Option<core::ExprId> {
        let object_id = self.lookup_object(object, context)?;
        let branch = self
            .object_sources
            .get(object)
            .map(|source| format!("{source}_{attr}"));
        let ty = if let Some(branch) = branch.as_deref() {
            self.model_outputs
                .by_branch
                .get(branch)
                .map(|output| core::Type::Quantity(output.dimension))
                .unwrap_or_else(|| core::Type::Quantity(attribute_dimension(attr)))
        } else {
            core::Type::Quantity(derived_attribute_dimension(attr))
        };
        let effects = branch
            .as_ref()
            .filter(|branch| !self.model_outputs.by_branch.contains_key(branch.as_str()))
            .map(|branch| BTreeSet::from([core::Effect::ReadsBranch(branch.clone())]))
            .unwrap_or_default();
        let kind = if branch.is_some() {
            core::ExprKind::Attr {
                object: object_id,
                attr: attr.to_string(),
                branch,
            }
        } else {
            core::ExprKind::DerivedAttr {
                object: object_id,
                attr: attr.to_string(),
            }
        };
        Some(self.builder.add_expr(kind, ty, effects))
    }

    fn quantity_node(&mut self, quantity: &Quantity) -> core::ExprId {
        let ty = match quantity.unit {
            Unit::GeV => core::Type::Quantity(Dimension::Momentum),
            Unit::Dimensionless => core::Type::Quantity(Dimension::Dimensionless),
        };
        self.builder.add_expr(
            core::ExprKind::Quantity(quantity.clone()),
            ty,
            BTreeSet::new(),
        )
    }

    fn filter_call(&mut self, primitive: &'static str, context: &str) -> Option<core::ExprId> {
        let candidate = self.builder.add_expr(
            core::ExprKind::Call {
                primitive: "candidate",
                args: Vec::new(),
            },
            core::Type::Candidate,
            BTreeSet::new(),
        );
        self.call(primitive, vec![candidate], context)
    }

    fn compare(
        &mut self,
        op: CmpOp,
        lhs: core::ExprId,
        rhs: core::ExprId,
        context: &str,
    ) -> Option<core::ExprId> {
        let primitive = core::primitive_name_for_cmp(op);
        let call = self.registry_call(primitive, &[lhs, rhs], context)?;
        Some(self.builder.add_expr(
            core::ExprKind::Compare { op, lhs, rhs },
            call.ty,
            call.effects,
        ))
    }

    fn call(
        &mut self,
        primitive: &'static str,
        args: Vec<core::ExprId>,
        context: &str,
    ) -> Option<core::ExprId> {
        let call = self.registry_call(primitive, &args, context)?;
        Some(self.builder.add_expr(
            core::ExprKind::Call { primitive, args },
            call.ty,
            call.effects,
        ))
    }

    fn registry_call(
        &mut self,
        primitive: &'static str,
        args: &[core::ExprId],
        context: &str,
    ) -> Option<core::PrimitiveCall> {
        let args = args
            .iter()
            .map(|id| {
                let node = self.builder_expr(*id);
                core::PrimitiveArg::with_effects(node.ty.clone(), node.effects.clone())
            })
            .collect::<Vec<_>>();
        match self.registry.validate_call(primitive, &args) {
            Ok(call) => Some(call),
            Err(error) => {
                self.errors.push(SpecError::InvalidExpression {
                    context: context.to_string(),
                    detail: error.to_string(),
                });
                None
            }
        }
    }

    fn builder_expr(&self, id: core::ExprId) -> &core::ExprNode {
        self.builder.expr(id)
    }
}

fn derived_attribute_dimension(attr: &str) -> Dimension {
    match attr {
        "mass" | "pt" => Dimension::Momentum,
        "min_delta_r" | "dR" | "dr" => Dimension::Dimensionless,
        _ => attribute_dimension(attr),
    }
}

fn validate_union(
    spec: &AnalysisSpec,
    catalogue: &Catalogue,
) -> Result<ResolvedPlan, Vec<SpecError>> {
    let mut errors = Vec::new();
    if !spec.models.is_empty() {
        errors.push(SpecError::InvalidExpression {
            context: "multi-channel union".to_string(),
            detail: "model-aware union codegen is not yet supported".to_string(),
        });
    }
    if !spec.objects.is_empty() || !spec.derived_objects.is_empty() || !spec.regions.is_empty() {
        errors.push(SpecError::InvalidExpression {
            context: "multi-channel union".to_string(),
            detail: "declare objects, derived objects, and regions inside [[channel]] blocks"
                .to_string(),
        });
    }

    let Some(first) = spec.channels.first() else {
        unreachable!("caller checks channels is non-empty");
    };
    let first_schema = output_schema(&first.outputs);
    let mut branch_specs = BTreeMap::new();
    for channel in &spec.channels {
        if channel.outputs.is_empty() {
            errors.push(SpecError::InvalidExpression {
                context: format!("channel `{}`", channel.name),
                detail: "channels in a union must declare at least one output".to_string(),
            });
        }
        if output_schema(&channel.outputs) != first_schema {
            errors.push(SpecError::InvalidExpression {
                context: format!("channel `{}`", channel.name),
                detail: "channel output schema must match the first channel by name and order"
                    .to_string(),
            });
        }

        let channel_spec = channel.as_spec(spec);
        match validate(&channel_spec, catalogue) {
            Ok(plan) => {
                for branch in plan.read_branches.specs() {
                    branch_specs.insert(branch.name.clone(), branch.branch_type);
                }
            }
            Err(channel_errors) => errors.extend(channel_errors),
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let read_branches = BranchSchema::new(
        branch_specs
            .into_iter()
            .map(|(name, branch_type)| BranchSpec::new(name, branch_type))
            .collect::<Vec<_>>(),
    )
    .map_err(|error| {
        vec![SpecError::InvalidReadSchema {
            detail: error.to_string(),
        }]
    })?;

    Ok(ResolvedPlan {
        spec: spec.clone(),
        read_branches,
    })
}

fn output_schema(outputs: &[OutputDef]) -> Vec<&str> {
    outputs.iter().map(|output| output.name.as_str()).collect()
}

fn validate_histogram(histogram: &HistogramDef, ctx: &mut ValidationContext<'_>) {
    let context = format!("histogram `{}`", histogram.name);
    if histogram.bins == 0 {
        ctx.errors.push(SpecError::InvalidExpression {
            context: context.clone(),
            detail: "histogram bins must be greater than zero".to_string(),
        });
    }
    if !(histogram.range[0].is_finite()
        && histogram.range[1].is_finite()
        && histogram.range[1] > histogram.range[0])
    {
        ctx.errors.push(SpecError::InvalidExpression {
            context: context.clone(),
            detail: "histogram range must be finite and ordered".to_string(),
        });
    }
    match validate_expr(&histogram.expr, &context, ctx) {
        Some(ExprType::Numeric(_)) => {}
        Some(ExprType::Count) => {}
        Some(_) => ctx.errors.push(SpecError::InvalidExpression {
            context,
            detail: "histogram expression must be numeric".to_string(),
        }),
        None => {}
    }
}

fn validate_shape_corrections(spec: &AnalysisSpec, ctx: &mut ValidationContext<'_>) {
    for correction in &spec.shape_corrections {
        let context = format!("correction `{}`", correction.name);
        let Some(source) = ctx.object_sources.get(correction.collection.as_str()) else {
            ctx.errors.push(SpecError::UndefinedObject {
                context,
                object: correction.collection.clone(),
            });
            continue;
        };
        require_attr_branch_type(
            source,
            &correction.attr,
            BranchType::VecF32,
            "f32 vector branch for shape scaling",
            &context,
            ctx,
        );
    }
}

struct ValidationContext<'a> {
    catalogue: &'a Catalogue,
    object_sources: &'a HashMap<&'a str, &'a str>,
    derived_objects: &'a HashMap<&'a str, &'a DerivedObjectDef>,
    model_outputs: &'a ModelOutputs,
    required: &'a mut RequiredBranches,
    errors: &'a mut Vec<SpecError>,
}

fn validate_cut(object: &ObjectDef, index: usize, cut: &Cut, ctx: &mut ValidationContext<'_>) {
    let context = format!("object `{}` cut {}", object.name, index + 1);
    let lhs_type = validate_expr(&cut.lhs, &context, ctx);

    match lhs_type {
        Some(ExprType::Numeric(dimension)) => {
            validate_quantity_unit(&context, &cut.lhs, dimension, &cut.rhs, ctx.errors)
        }
        Some(ExprType::Count) => ctx.errors.push(SpecError::InvalidExpression {
            context,
            detail: "object cuts must compare branch attributes, not counts".to_string(),
        }),
        Some(ExprType::Bool) => ctx.errors.push(SpecError::InvalidExpression {
            context,
            detail: "object cuts must compare numeric expressions, not predicates".to_string(),
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
    if let Some(expr_type) = validate_expr(&requirement.lhs, &context, ctx) {
        match expr_type {
            ExprType::Numeric(dimension) => validate_quantity_unit(
                &context,
                &requirement.lhs,
                dimension,
                &requirement.rhs,
                ctx.errors,
            ),
            ExprType::Count => validate_quantity_unit(
                &context,
                &requirement.lhs,
                Dimension::Dimensionless,
                &requirement.rhs,
                ctx.errors,
            ),
            ExprType::Bool => {
                validate_quantity_unit(
                    &context,
                    &requirement.lhs,
                    Dimension::Dimensionless,
                    &requirement.rhs,
                    ctx.errors,
                );
                validate_bool_requirement(&context, requirement, ctx.errors);
            }
        }
    }
}

fn validate_bool_requirement(
    context: &str,
    requirement: &Requirement,
    errors: &mut Vec<SpecError>,
) {
    let valid_rhs = requirement.rhs.value == 0.0 || requirement.rhs.value == 1.0;
    let valid_op = matches!(requirement.op, CmpOp::Eq | CmpOp::Ne);
    if !valid_rhs || !valid_op {
        errors.push(SpecError::InvalidExpression {
            context: context.to_string(),
            detail: format!(
                "boolean predicate `{}` supports only == 1, != 0, == 0, or != 1",
                requirement.lhs
            ),
        });
    }
}

fn validate_derived_object(derived: &DerivedObjectDef, ctx: &mut ValidationContext<'_>) {
    let context = format!("derived object `{}`", derived.name);
    match &derived.source {
        DerivedSource::Pair(pair) => {
            let Some(source) = ctx.object_sources.get(pair.object.as_str()) else {
                ctx.errors.push(SpecError::UndefinedObject {
                    context,
                    object: pair.object.clone(),
                });
                return;
            };

            ctx.required.require_counter(source);
            require_four_vector(source, &context, ctx);
            for constraint in &pair.constraints {
                match constraint {
                    PairConstraint::OppositeCharge => {
                        require_attr_branch_type(
                            source,
                            "charge",
                            BranchType::VecI32,
                            "i32 vector branch for opposite-charge pairing",
                            &context,
                            ctx,
                        );
                    }
                    PairConstraint::SameFlavor => {}
                }
            }

            for (index, filter) in pair.filters.iter().enumerate() {
                validate_pair_filter(pair, index, filter, &context, ctx);
            }

            match &pair.selection {
                PairSelection::LeadingPt => {}
                PairSelection::NearestMass { target } => {
                    validate_quantity_unit(
                        &context,
                        &Expr::Attr {
                            object: derived.name.clone(),
                            attr: "mass".to_string(),
                        },
                        Dimension::Momentum,
                        target,
                        ctx.errors,
                    );
                }
                PairSelection::NearestMassTruncated { target } => {
                    validate_quantity_unit(
                        &context,
                        &Expr::Attr {
                            object: derived.name.clone(),
                            attr: "mass".to_string(),
                        },
                        Dimension::Momentum,
                        target,
                        ctx.errors,
                    );
                }
            }

            for excluded in &pair.exclude {
                let Some(excluded_derived) = ctx.derived_objects.get(excluded.as_str()) else {
                    ctx.errors.push(SpecError::UndefinedObject {
                        context: context.clone(),
                        object: excluded.clone(),
                    });
                    continue;
                };
                match &excluded_derived.source {
                    DerivedSource::Pair(excluded_pair) if excluded_pair.object == pair.object => {}
                    DerivedSource::Pair(excluded_pair) => {
                        ctx.errors.push(SpecError::InvalidExpression {
                            context: context.clone(),
                            detail: format!(
                                "pair `{}` excludes `{excluded}`, but `{excluded}` is built from `{}` instead of `{}`",
                                derived.name, excluded_pair.object, pair.object
                            ),
                        });
                    }
                    DerivedSource::Candidate(_) => {
                        ctx.errors.push(SpecError::InvalidExpression {
                            context: context.clone(),
                            detail: format!(
                                "pair `{}` can only exclude pair-derived selections, not candidate `{excluded}`",
                                derived.name
                            ),
                        });
                    }
                }
            }
        }
        DerivedSource::Candidate(candidate) => {
            if candidate.items.is_empty() {
                ctx.errors.push(SpecError::InvalidExpression {
                    context,
                    detail: "candidate items must not be empty".to_string(),
                });
                return;
            }
            for item in &candidate.items {
                validate_candidate_item(item, &context, ctx);
            }
            for (index, filter) in candidate.filters.iter().enumerate() {
                validate_candidate_filter(index, filter, &context, ctx);
            }
        }
    }
}

fn validate_pair_filter(
    pair: &ObjectPairDef,
    index: usize,
    filter: &Cut,
    parent_context: &str,
    ctx: &mut ValidationContext<'_>,
) {
    let context = format!("{parent_context} filter {}", index + 1);
    let Some(dimension) = validate_filter_expr(&filter.lhs, FilterContext::Pair, &context, ctx)
    else {
        return;
    };
    validate_quantity_unit(&context, &filter.lhs, dimension, &filter.rhs, ctx.errors);
    let Some(source) = ctx.object_sources.get(pair.object.as_str()) else {
        return;
    };
    match filter.lhs {
        Expr::PairDeltaR => {
            require_attr_branch_type(
                source,
                "eta",
                BranchType::VecF32,
                "f32 vector branch for pair delta-R",
                &context,
                ctx,
            );
            require_attr_branch_type(
                source,
                "phi",
                BranchType::VecF32,
                "f32 vector branch for pair delta-R",
                &context,
                ctx,
            );
        }
        Expr::PairLeadingPt | Expr::PairSubleadingPt => {
            require_attr_branch_type(
                source,
                "pt",
                BranchType::VecF32,
                "f32 vector branch for pair pT filter",
                &context,
                ctx,
            );
        }
        _ => {}
    }
}

fn validate_candidate_filter(
    index: usize,
    filter: &Cut,
    parent_context: &str,
    ctx: &mut ValidationContext<'_>,
) {
    let context = format!("{parent_context} filter {}", index + 1);
    let Some(dimension) =
        validate_filter_expr(&filter.lhs, FilterContext::Candidate, &context, ctx)
    else {
        return;
    };
    validate_quantity_unit(&context, &filter.lhs, dimension, &filter.rhs, ctx.errors);
}

fn validate_candidate_item(item: &str, context: &str, ctx: &mut ValidationContext<'_>) {
    if let Some(source) = ctx.object_sources.get(item) {
        ctx.required.require_counter(source);
        require_four_vector(source, context, ctx);
    } else if let Some(derived) = ctx.derived_objects.get(item) {
        validate_derived_attr(derived, "mass", context, ctx);
    } else {
        ctx.errors.push(SpecError::UndefinedObject {
            context: context.to_string(),
            object: item.to_string(),
        });
    }
}

fn validate_expr(expr: &Expr, context: &str, ctx: &mut ValidationContext<'_>) -> Option<ExprType> {
    match expr {
        Expr::Attr { object, attr } => validate_attr(object, attr, context, ctx),
        Expr::Literal(_) => Some(ExprType::Numeric(Dimension::Dimensionless)),
        Expr::Binary { op, lhs, rhs } => {
            let lhs_type = validate_expr(lhs, context, ctx);
            let rhs_type = validate_expr(rhs, context, ctx);
            match (lhs_type, rhs_type) {
                (Some(ExprType::Numeric(lhs)), Some(ExprType::Numeric(rhs))) => {
                    validate_binary_dimension(*op, lhs, rhs, context, expr, ctx)
                }
                (Some(_), Some(_)) => {
                    ctx.errors.push(SpecError::InvalidExpression {
                        context: context.to_string(),
                        detail: format!("arithmetic expression `{expr}` requires numeric operands"),
                    });
                    None
                }
                _ => None,
            }
        }
        Expr::Abs(inner) => match validate_expr(inner, context, ctx) {
            Some(ExprType::Numeric(dimension)) => Some(ExprType::Numeric(dimension)),
            Some(ExprType::Count) => {
                ctx.errors.push(SpecError::InvalidExpression {
                    context: context.to_string(),
                    detail: "abs(...) requires a numeric attribute".to_string(),
                });
                None
            }
            Some(ExprType::Bool) => {
                ctx.errors.push(SpecError::InvalidExpression {
                    context: context.to_string(),
                    detail: "abs(...) requires a numeric expression".to_string(),
                });
                None
            }
            None => None,
        },
        Expr::Sqrt(inner) => match validate_expr(inner, context, ctx) {
            Some(ExprType::Numeric(dimension)) => Some(ExprType::Numeric(dimension)),
            Some(_) => {
                ctx.errors.push(SpecError::InvalidExpression {
                    context: context.to_string(),
                    detail: "sqrt(...) requires a numeric expression".to_string(),
                });
                None
            }
            None => None,
        },
        Expr::Count(object) => {
            let Some(source) = ctx.object_sources.get(object.as_str()) else {
                if ctx.derived_objects.contains_key(object.as_str()) {
                    ctx.errors.push(SpecError::InvalidExpression {
                        context: context.to_string(),
                        detail: format!(
                            "count({object}) is only defined for selected object collections, not derived objects"
                        ),
                    });
                } else {
                    ctx.errors.push(SpecError::UndefinedObject {
                        context: context.to_string(),
                        object: object.clone(),
                    });
                }
                return None;
            };
            ctx.required.require_counter(source);
            Some(ExprType::Count)
        }
        Expr::CountWhere { object, predicate } => {
            let Some(source) = ctx.object_sources.get(object.as_str()) else {
                ctx.errors.push(SpecError::UndefinedObject {
                    context: context.to_string(),
                    object: object.clone(),
                });
                return None;
            };
            ctx.required.require_counter(source);
            validate_collection_predicate(object, predicate, context, ctx);
            Some(ExprType::Count)
        }
        Expr::SumAttr { object, attr } => validate_attr(object, attr, context, ctx),
        Expr::All { object, predicate } | Expr::Any { object, predicate } => {
            let Some(source) = ctx.object_sources.get(object.as_str()) else {
                ctx.errors.push(SpecError::UndefinedObject {
                    context: context.to_string(),
                    object: object.clone(),
                });
                return None;
            };
            ctx.required.require_counter(source);
            validate_collection_predicate(object, predicate, context, ctx);
            Some(ExprType::Bool)
        }
        Expr::EitherPairPt {
            left,
            right,
            leading,
            subleading,
        } => {
            validate_pair_pt_object(left, leading, subleading, context, ctx);
            validate_pair_pt_object(right, leading, subleading, context, ctx);
            Some(ExprType::Bool)
        }
        Expr::ClosestMass {
            left,
            right,
            target,
        }
        | Expr::OtherMass {
            left,
            right,
            target,
        } => {
            validate_mass_order_object(left, context, ctx);
            validate_mass_order_object(right, context, ctx);
            validate_quantity_unit(context, expr, Dimension::Momentum, target, ctx.errors);
            Some(ExprType::Numeric(Dimension::Momentum))
        }
        Expr::LeadingAttr { object, attr } => validate_attr(object, attr, context, ctx),
        Expr::PairDeltaR
        | Expr::PairLeadingPt
        | Expr::PairSubleadingPt
        | Expr::CandidateMinDeltaR
        | Expr::CandidateLeadingPt
        | Expr::CandidateSubleadingPt => {
            ctx.errors.push(SpecError::InvalidExpression {
                context: context.to_string(),
                detail: format!("filter-only expression `{expr}` is not valid here"),
            });
            None
        }
    }
}

fn validate_pair_pt_object(
    object: &str,
    leading: &Quantity,
    subleading: &Quantity,
    context: &str,
    ctx: &mut ValidationContext<'_>,
) {
    validate_quantity_unit(
        context,
        &Expr::Attr {
            object: object.to_string(),
            attr: "pt".to_string(),
        },
        Dimension::Momentum,
        leading,
        ctx.errors,
    );
    validate_quantity_unit(
        context,
        &Expr::Attr {
            object: object.to_string(),
            attr: "pt".to_string(),
        },
        Dimension::Momentum,
        subleading,
        ctx.errors,
    );
    let Some(source) = ctx.object_sources.get(object) else {
        ctx.errors.push(SpecError::UndefinedObject {
            context: context.to_string(),
            object: object.to_string(),
        });
        return;
    };
    require_attr_branch_type(
        source,
        "pt",
        BranchType::VecF32,
        "f32 vector branch for leading/subleading pT",
        context,
        ctx,
    );
}

fn validate_mass_order_object(object: &str, context: &str, ctx: &mut ValidationContext<'_>) {
    let Some(derived) = ctx.derived_objects.get(object) else {
        ctx.errors.push(SpecError::UndefinedObject {
            context: context.to_string(),
            object: object.to_string(),
        });
        return;
    };
    validate_derived_attr(derived, "mass", context, ctx);
}

fn validate_binary_dimension(
    op: ArithOp,
    lhs: Dimension,
    rhs: Dimension,
    context: &str,
    expr: &Expr,
    ctx: &mut ValidationContext<'_>,
) -> Option<ExprType> {
    match op {
        ArithOp::Add | ArithOp::Sub if lhs == rhs => Some(ExprType::Numeric(lhs)),
        ArithOp::Add | ArithOp::Sub => {
            ctx.errors.push(SpecError::InvalidExpression {
                context: context.to_string(),
                detail: format!("`{expr}` cannot add or subtract incompatible dimensions"),
            });
            None
        }
        ArithOp::Mul if lhs == Dimension::Dimensionless => Some(ExprType::Numeric(rhs)),
        ArithOp::Mul if rhs == Dimension::Dimensionless => Some(ExprType::Numeric(lhs)),
        ArithOp::Mul => Some(ExprType::Numeric(Dimension::Dimensionless)),
        ArithOp::Div if rhs == Dimension::Dimensionless => Some(ExprType::Numeric(lhs)),
        ArithOp::Div => Some(ExprType::Numeric(Dimension::Dimensionless)),
        ArithOp::Pow => {
            if rhs != Dimension::Dimensionless {
                ctx.errors.push(SpecError::InvalidExpression {
                    context: context.to_string(),
                    detail: format!("`{expr}` exponent must be dimensionless"),
                });
                None
            } else {
                Some(ExprType::Numeric(lhs))
            }
        }
    }
}

fn validate_collection_predicate(
    object: &str,
    predicate: &Cut,
    context: &str,
    ctx: &mut ValidationContext<'_>,
) {
    let predicate_context = format!("{context} predicate");
    match validate_expr(&predicate.lhs, &predicate_context, ctx) {
        Some(ExprType::Numeric(dimension)) => validate_quantity_unit(
            &predicate_context,
            &predicate.lhs,
            dimension,
            &predicate.rhs,
            ctx.errors,
        ),
        Some(_) => ctx.errors.push(SpecError::InvalidExpression {
            context: predicate_context,
            detail: format!("predicate for `{object}` must compare a numeric expression"),
        }),
        None => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterContext {
    Pair,
    Candidate,
}

fn validate_filter_expr(
    expr: &Expr,
    filter_context: FilterContext,
    context: &str,
    ctx: &mut ValidationContext<'_>,
) -> Option<Dimension> {
    match (filter_context, expr) {
        (FilterContext::Pair, Expr::PairDeltaR)
        | (FilterContext::Candidate, Expr::CandidateMinDeltaR) => Some(Dimension::Dimensionless),
        (FilterContext::Pair, Expr::PairLeadingPt | Expr::PairSubleadingPt)
        | (FilterContext::Candidate, Expr::CandidateLeadingPt | Expr::CandidateSubleadingPt) => {
            Some(Dimension::Momentum)
        }
        _ => {
            ctx.errors.push(SpecError::InvalidExpression {
                context: context.to_string(),
                detail: format!("unsupported filter expression `{expr}`"),
            });
            None
        }
    }
}

fn validate_attr(
    object: &str,
    attr: &str,
    context: &str,
    ctx: &mut ValidationContext<'_>,
) -> Option<ExprType> {
    if let Some(derived) = ctx.derived_objects.get(object) {
        return validate_derived_attr(derived, attr, context, ctx);
    }

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

fn validate_derived_attr(
    derived: &DerivedObjectDef,
    attr: &str,
    context: &str,
    ctx: &mut ValidationContext<'_>,
) -> Option<ExprType> {
    match &derived.source {
        DerivedSource::Pair(pair) => {
            let Some(source) = ctx.object_sources.get(pair.object.as_str()) else {
                ctx.errors.push(SpecError::UndefinedObject {
                    context: context.to_string(),
                    object: pair.object.clone(),
                });
                return None;
            };

            match attr {
                "mass" => {
                    require_four_vector(source, context, ctx);
                    Some(ExprType::Numeric(Dimension::Momentum))
                }
                "pt" => {
                    require_attr_branch_type(
                        source,
                        "pt",
                        BranchType::VecF32,
                        "f32 vector branch for pair pT",
                        context,
                        ctx,
                    );
                    require_attr_branch_type(
                        source,
                        "phi",
                        BranchType::VecF32,
                        "f32 vector branch for pair pT",
                        context,
                        ctx,
                    );
                    Some(ExprType::Numeric(Dimension::Momentum))
                }
                "min_delta_r" | "dR" | "dr" => {
                    require_attr_branch_type(
                        source,
                        "eta",
                        BranchType::VecF32,
                        "f32 vector branch for pair delta-R",
                        context,
                        ctx,
                    );
                    require_attr_branch_type(
                        source,
                        "phi",
                        BranchType::VecF32,
                        "f32 vector branch for pair delta-R",
                        context,
                        ctx,
                    );
                    Some(ExprType::Numeric(Dimension::Dimensionless))
                }
                other => {
                    ctx.errors.push(SpecError::InvalidExpression {
                        context: context.to_string(),
                        detail: format!(
                            "derived pair `{}` has no attribute `{other}`; supported attributes are `mass` and `pt`",
                            derived.name
                        ),
                    });
                    None
                }
            }
        }
        DerivedSource::Candidate(_) => match attr {
            "mass" => Some(ExprType::Numeric(Dimension::Momentum)),
            "pt" => Some(ExprType::Numeric(Dimension::Momentum)),
            "min_delta_r" | "dR" | "dr" => Some(ExprType::Numeric(Dimension::Dimensionless)),
            other => {
                ctx.errors.push(SpecError::InvalidExpression {
                    context: context.to_string(),
                    detail: format!(
                        "derived candidate `{}` has no attribute `{other}`; supported attributes are `mass` and `pt`",
                        derived.name
                    ),
                });
                None
            }
        },
    }
}

fn require_four_vector(source: &str, context: &str, ctx: &mut ValidationContext<'_>) {
    for attr in ["pt", "eta", "phi", "mass"] {
        require_attr_branch_type(
            source,
            attr,
            BranchType::VecF32,
            "f32 vector branch for pt/eta/phi/mass four-vector",
            context,
            ctx,
        );
    }
}

fn require_attr_branch_type(
    source: &str,
    attr: &str,
    expected_type: BranchType,
    expected: &str,
    context: &str,
    ctx: &mut ValidationContext<'_>,
) {
    let branch = format!("{source}_{attr}");
    let Some(entry) = ctx.catalogue.branch(&branch) else {
        ctx.errors.push(SpecError::MissingBranch {
            context: context.to_string(),
            branch,
        });
        return;
    };
    let Some(branch_type) = entry.branch_type else {
        ctx.errors.push(SpecError::UnsupportedBranchType {
            context: context.to_string(),
            branch,
            raw_type: entry.raw_type.clone(),
        });
        return;
    };
    if branch_type != expected_type {
        ctx.errors.push(SpecError::WrongBranchType {
            context: context.to_string(),
            branch,
            expected: expected.to_string(),
            actual: branch_type,
        });
        return;
    }
    ctx.required.require_counter(source);
    ctx.required.require_attr(source, attr);
}

fn validate_quantity_unit(
    context: &str,
    lhs: &Expr,
    dimension: Dimension,
    rhs: &Quantity,
    errors: &mut Vec<SpecError>,
) {
    match (dimension, rhs.unit) {
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
    Bool,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
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

    for name in raw.derived.keys() {
        validate_identifier(name, "derived")?;
        if raw.objects.contains_key(name) {
            return Err(ParseError::InvalidSpec(format!(
                "derived object `{name}` duplicates an object name"
            )));
        }
    }

    for name in raw.regions.keys() {
        validate_identifier(name, "regions")?;
    }

    let mut weight_systematics = BTreeSet::new();
    for systematic in &raw.systematic {
        validate_identifier(&systematic.name, "systematic name")?;
        if !weight_systematics.insert(systematic.name.as_str()) {
            return Err(ParseError::InvalidSpec(format!(
                "duplicate systematic `{}`",
                systematic.name
            )));
        }
    }
    if raw.systematic.len() > 1 {
        return Err(ParseError::InvalidSpec(
            "this compiler slice supports at most one weight systematic".to_string(),
        ));
    }

    let mut shape_corrections = BTreeSet::new();
    for correction in &raw.corrections {
        validate_identifier(&correction.name, "correction name")?;
        if !shape_corrections.insert(correction.name.as_str()) {
            return Err(ParseError::InvalidSpec(format!(
                "duplicate correction `{}`",
                correction.name
            )));
        }
    }
    if raw.corrections.len() > 1 {
        return Err(ParseError::InvalidSpec(
            "this compiler slice supports at most one shape correction".to_string(),
        ));
    }

    let mut channel_names = BTreeSet::new();
    for channel in &raw.channels {
        validate_identifier(&channel.name, "channel name")?;
        if !channel_names.insert(channel.name.as_str()) {
            return Err(ParseError::InvalidSpec(format!(
                "duplicate channel `{}`",
                channel.name
            )));
        }
        for (name, object) in &channel.objects {
            validate_identifier(name, &format!("channel `{}` objects", channel.name))?;
            if object.source.trim().is_empty() {
                return Err(ParseError::InvalidSpec(format!(
                    "channel `{}` object `{name}` is missing source",
                    channel.name
                )));
            }
        }
        for name in channel.derived.keys() {
            validate_identifier(name, &format!("channel `{}` derived", channel.name))?;
            if channel.objects.contains_key(name) {
                return Err(ParseError::InvalidSpec(format!(
                    "channel `{}` derived object `{name}` duplicates an object name",
                    channel.name
                )));
            }
        }
        for name in channel.regions.keys() {
            validate_identifier(name, &format!("channel `{}` regions", channel.name))?;
        }
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

fn parse_pair_filter(input: &str) -> Result<Cut, ParseError> {
    let (lhs, op, rhs) = split_comparison(input)?;
    Ok(Cut {
        lhs: parse_expr(lhs, None)?,
        op,
        rhs: parse_quantity(rhs)?,
    })
}

fn parse_candidate_filter(input: &str) -> Result<Cut, ParseError> {
    let mut filter = parse_pair_filter(input)?;
    filter.lhs = match filter.lhs {
        Expr::PairDeltaR => Expr::CandidateMinDeltaR,
        Expr::PairLeadingPt => Expr::CandidateLeadingPt,
        Expr::PairSubleadingPt => Expr::CandidateSubleadingPt,
        other => other,
    };
    Ok(filter)
}

fn parse_requirement(input: &str) -> Result<Requirement, ParseError> {
    let trimmed = input.trim();
    if starts_with_call(trimmed, "all")
        || starts_with_call(trimmed, "any")
        || starts_with_call(trimmed, "either_pair_pt")
    {
        return Ok(Requirement {
            lhs: parse_expr(trimmed, None)?,
            op: CmpOp::Eq,
            rhs: Quantity {
                value: 1.0,
                unit: Unit::Dimensionless,
            },
        });
    }
    let (lhs, op, rhs) = split_comparison(input)?;
    let rhs = parse_quantity(rhs)?;
    Ok(Requirement {
        lhs: parse_expr(lhs, None)?,
        op,
        rhs,
    })
}

fn split_comparison(input: &str) -> Result<(&str, CmpOp, &str), ParseError> {
    let bytes = input.as_bytes();
    let mut depth = 0_i32;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ if depth == 0 => {
                for (token, op) in [
                    (">=", CmpOp::Ge),
                    ("<=", CmpOp::Le),
                    ("==", CmpOp::Eq),
                    ("!=", CmpOp::Ne),
                    (">", CmpOp::Gt),
                    ("<", CmpOp::Lt),
                ] {
                    if input[index..].starts_with(token) {
                        let lhs = input[..index].trim();
                        let rhs = input[index + token.len()..].trim();
                        if !lhs.is_empty() && !rhs.is_empty() {
                            return Ok((lhs, op, rhs));
                        }
                    }
                }
            }
            _ => {}
        }
        index += 1;
    }

    Err(ParseError::InvalidSpec(format!(
        "could not parse comparison `{input}`"
    )))
}

fn parse_expr(input: &str, default_object: Option<&str>) -> Result<Expr, ParseError> {
    parse_expr_prec(input, default_object, 0)
}

fn parse_expr_prec(
    input: &str,
    default_object: Option<&str>,
    min_prec: u8,
) -> Result<Expr, ParseError> {
    let input = strip_wrapping_parens(input.trim());
    if input.is_empty() {
        return Err(ParseError::InvalidSpec("empty expression".to_string()));
    }

    if let Some((index, op, precedence)) = find_binary_operator(input, min_prec) {
        let lhs = parse_expr_prec(&input[..index], default_object, precedence + 1)?;
        let rhs = parse_expr_prec(&input[index + 1..], default_object, precedence)?;
        return Ok(Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        });
    }

    if let Some(inner) = input
        .strip_prefix("abs(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return Ok(Expr::Abs(Box::new(parse_expr(inner, default_object)?)));
    }

    if let Some(inner) = input
        .strip_prefix("sqrt(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return Ok(Expr::Sqrt(Box::new(parse_expr(inner, default_object)?)));
    }

    if let Some(inner) = input
        .strip_prefix("count(")
        .and_then(|value| value.strip_suffix(')'))
    {
        if let Some((object, predicate)) = split_top_level_comma(inner) {
            let object = object.trim();
            validate_identifier(object, input)?;
            return Ok(Expr::CountWhere {
                object: object.to_string(),
                predicate: Box::new(parse_cut(predicate, object)?),
            });
        }
        let object = inner.trim();
        validate_identifier(object, input)?;
        return Ok(Expr::Count(object.to_string()));
    }

    if let Some(inner) = input
        .strip_prefix("sum(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let Some((object, attr)) = inner.trim().split_once('.') else {
            return Err(ParseError::InvalidSpec(format!(
                "could not parse sum expression `{input}`; expected sum(object.attr)"
            )));
        };
        validate_identifier(object.trim(), input)?;
        validate_identifier(attr.trim(), input)?;
        return Ok(Expr::SumAttr {
            object: object.trim().to_string(),
            attr: attr.trim().to_string(),
        });
    }

    if let Some(inner) = input
        .strip_prefix("all(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let Some((object, predicate)) = split_top_level_comma(inner) else {
            return Err(ParseError::InvalidSpec(format!(
                "could not parse all predicate `{input}`"
            )));
        };
        let object = object.trim();
        validate_identifier(object, input)?;
        return Ok(Expr::All {
            object: object.to_string(),
            predicate: Box::new(parse_cut(predicate, object)?),
        });
    }

    if let Some(inner) = input
        .strip_prefix("any(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let Some((object, predicate)) = split_top_level_comma(inner) else {
            return Err(ParseError::InvalidSpec(format!(
                "could not parse any predicate `{input}`"
            )));
        };
        let object = object.trim();
        validate_identifier(object, input)?;
        return Ok(Expr::Any {
            object: object.to_string(),
            predicate: Box::new(parse_cut(predicate, object)?),
        });
    }

    if let Some(inner) = input
        .strip_prefix("either_pair_pt(")
        .and_then(|value| value.strip_suffix(')'))
    {
        let args = split_top_level_args(inner);
        if args.len() != 4 {
            return Err(ParseError::InvalidSpec(format!(
                "could not parse either_pair_pt predicate `{input}`; expected either_pair_pt(left, right, leading, subleading)"
            )));
        }
        let left = args[0].trim();
        let right = args[1].trim();
        validate_identifier(left, input)?;
        validate_identifier(right, input)?;
        return Ok(Expr::EitherPairPt {
            left: left.to_string(),
            right: right.to_string(),
            leading: parse_quantity(args[2].trim())?,
            subleading: parse_quantity(args[3].trim())?,
        });
    }

    if let Some(inner) = input
        .strip_prefix("closest_mass(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return parse_mass_order_expr(input, inner, true);
    }

    if let Some(inner) = input
        .strip_prefix("other_mass(")
        .and_then(|value| value.strip_suffix(')'))
    {
        return parse_mass_order_expr(input, inner, false);
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

    match input {
        "dR" | "dr" | "delta_r" | "deltaR" => return Ok(Expr::PairDeltaR),
        "min_dR" | "min_dr" | "min_delta_r" | "min_deltaR" => return Ok(Expr::CandidateMinDeltaR),
        "leading_pt" => return Ok(Expr::PairLeadingPt),
        "subleading_pt" => return Ok(Expr::PairSubleadingPt),
        "candidate_leading_pt" => return Ok(Expr::CandidateLeadingPt),
        "candidate_subleading_pt" => return Ok(Expr::CandidateSubleadingPt),
        _ => {}
    }

    if let Ok(value) = input.parse::<f64>() {
        return Ok(Expr::Literal(value));
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

fn starts_with_call(input: &str, function: &str) -> bool {
    input
        .strip_prefix(function)
        .is_some_and(|rest| rest.starts_with('(') && rest.ends_with(')'))
}

fn strip_wrapping_parens(mut input: &str) -> &str {
    loop {
        let trimmed = input.trim();
        if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
            return trimmed;
        }
        let mut depth = 0_i32;
        let mut wraps = true;
        for (index, ch) in trimmed.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 && index != trimmed.len() - 1 {
                        wraps = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if wraps {
            input = &trimmed[1..trimmed.len() - 1];
        } else {
            return trimmed;
        }
    }
}

fn split_top_level_comma(input: &str) -> Option<(&str, &str)> {
    let mut depth = 0_i32;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => return Some((&input[..index], &input[index + 1..])),
            _ => {}
        }
    }
    None
}

fn split_top_level_args(input: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                args.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    args.push(&input[start..]);
    args
}

fn parse_mass_order_expr(input: &str, inner: &str, closest: bool) -> Result<Expr, ParseError> {
    let args = split_top_level_args(inner);
    if args.len() != 3 {
        return Err(ParseError::InvalidSpec(format!(
            "could not parse mass-order expression `{input}`; expected function(left, right, target)"
        )));
    }
    let left = args[0].trim();
    let right = args[1].trim();
    validate_identifier(left, input)?;
    validate_identifier(right, input)?;
    let target = parse_quantity(args[2].trim())?;
    if closest {
        Ok(Expr::ClosestMass {
            left: left.to_string(),
            right: right.to_string(),
            target,
        })
    } else {
        Ok(Expr::OtherMass {
            left: left.to_string(),
            right: right.to_string(),
            target,
        })
    }
}

fn find_binary_operator(input: &str, min_prec: u8) -> Option<(usize, ArithOp, u8)> {
    for precedence in min_prec..=3 {
        let mut depth = 0_i32;
        for (index, ch) in input.char_indices().rev() {
            match ch {
                ')' => depth += 1,
                '(' => depth -= 1,
                '+' | '-' | '*' | '/' | '^' if depth == 0 => {
                    let Some((op, op_precedence)) = arith_operator(ch) else {
                        continue;
                    };
                    if op_precedence != precedence || is_unary_minus(input, index, ch) {
                        continue;
                    }
                    return Some((index, op, op_precedence));
                }
                _ => {}
            }
        }
    }
    None
}

fn arith_operator(ch: char) -> Option<(ArithOp, u8)> {
    match ch {
        '+' => Some((ArithOp::Add, 1)),
        '-' => Some((ArithOp::Sub, 1)),
        '*' => Some((ArithOp::Mul, 2)),
        '/' => Some((ArithOp::Div, 2)),
        '^' => Some((ArithOp::Pow, 3)),
        _ => None,
    }
}

fn is_unary_minus(input: &str, index: usize, ch: char) -> bool {
    if ch != '-' {
        return false;
    }
    input[..index]
        .trim_end()
        .chars()
        .next_back()
        .is_none_or(|previous| matches!(previous, '(' | '+' | '-' | '*' | '/' | '^'))
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
    #[serde(default, alias = "derived_objects")]
    derived: BTreeMap<String, RawDerivedObject>,
    #[serde(default, rename = "model")]
    models: Vec<RawModel>,
    #[serde(default)]
    regions: BTreeMap<String, RawRegion>,
    #[serde(default)]
    outputs: Vec<RawOutput>,
    #[serde(default, rename = "histogram")]
    histograms: Vec<RawHistogram>,
    #[serde(default)]
    weight: Option<RawWeight>,
    #[serde(default)]
    systematics: Vec<String>,
    #[serde(default, rename = "systematic")]
    systematic: Vec<RawSystematic>,
    #[serde(default, rename = "correction")]
    corrections: Vec<RawCorrection>,
    #[serde(default, rename = "channel")]
    channels: Vec<RawChannel>,
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
struct RawDerivedObject {
    kind: String,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    items: Vec<String>,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    filters: Vec<String>,
    #[serde(default)]
    selection: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    exclude: Vec<String>,
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

#[derive(Debug, serde::Deserialize)]
struct RawHistogram {
    name: String,
    expr: String,
    bins: usize,
    range: [f64; 2],
}

#[derive(Debug, serde::Deserialize)]
struct RawWeight {
    #[serde(default)]
    nominal: Vec<f64>,
}

#[derive(Debug, serde::Deserialize)]
struct RawSystematic {
    name: String,
    kind: String,
    up: f64,
    down: f64,
}

#[derive(Debug, serde::Deserialize)]
struct RawCorrection {
    name: String,
    kind: String,
    collection: String,
    attr: String,
    up: f64,
    down: f64,
}

#[derive(Debug, serde::Deserialize)]
struct RawChannel {
    name: String,
    #[serde(default)]
    objects: BTreeMap<String, RawObject>,
    #[serde(default, alias = "derived_objects")]
    derived: BTreeMap<String, RawDerivedObject>,
    #[serde(default)]
    regions: BTreeMap<String, RawRegion>,
    #[serde(default)]
    outputs: Vec<RawOutput>,
}

fn object_defs_from_raw(
    raw_objects: BTreeMap<String, RawObject>,
) -> Result<Vec<ObjectDef>, ParseError> {
    let mut objects = Vec::with_capacity(raw_objects.len());
    for (name, object) in raw_objects {
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
    Ok(objects)
}

fn derived_defs_from_raw(
    raw_derived: BTreeMap<String, RawDerivedObject>,
) -> Result<Vec<DerivedObjectDef>, ParseError> {
    raw_derived
        .into_iter()
        .map(derived_object_def_from_raw)
        .collect::<Result<Vec<_>, _>>()
}

fn region_defs_from_raw(
    raw_regions: BTreeMap<String, RawRegion>,
) -> Result<Vec<RegionDef>, ParseError> {
    let mut regions = Vec::with_capacity(raw_regions.len());
    for (name, region) in raw_regions {
        let require = region
            .require
            .iter()
            .map(|requirement| parse_requirement(requirement))
            .collect::<Result<Vec<_>, _>>()?;
        regions.push(RegionDef { name, require });
    }
    Ok(regions)
}

fn output_defs_from_raw(raw_outputs: &[RawOutput]) -> Result<Vec<OutputDef>, ParseError> {
    raw_outputs
        .iter()
        .map(|output| {
            Ok(OutputDef {
                name: output.name.clone(),
                expr: parse_expr(&output.expr, None)?,
            })
        })
        .collect::<Result<Vec<_>, ParseError>>()
}

fn histogram_defs_from_raw(
    raw_histograms: &[RawHistogram],
) -> Result<Vec<HistogramDef>, ParseError> {
    raw_histograms
        .iter()
        .map(|histogram| {
            validate_identifier(&histogram.name, "histogram name")?;
            Ok(HistogramDef {
                name: histogram.name.clone(),
                expr: parse_expr(&histogram.expr, None)?,
                bins: histogram.bins,
                range: histogram.range,
            })
        })
        .collect::<Result<Vec<_>, ParseError>>()
}

fn weight_def_from_raw(raw: RawWeight) -> WeightDef {
    WeightDef {
        nominal: raw.nominal,
    }
}

fn systematic_def_from_raw(value: &str) -> Result<SystematicDef, ParseError> {
    match value {
        "nominal" | "Nominal" => Ok(SystematicDef::Nominal),
        "jes_up" | "JesUp" => Ok(SystematicDef::JesUp),
        "jes_down" | "JesDown" => Ok(SystematicDef::JesDown),
        "jer_up" | "JerUp" => Ok(SystematicDef::JerUp),
        "jer_down" | "JerDown" => Ok(SystematicDef::JerDown),
        other => Err(ParseError::InvalidSpec(format!(
            "unsupported systematic `{other}`"
        ))),
    }
}

fn weight_systematic_def_from_raw(raw: RawSystematic) -> Result<SystematicDef, ParseError> {
    validate_identifier(&raw.name, "systematic name")?;
    if raw.kind != "weight" {
        return Err(ParseError::InvalidSpec(format!(
            "systematic `{}` has unsupported kind `{}`; expected `weight`",
            raw.name, raw.kind
        )));
    }
    if !(raw.up.is_finite() && raw.down.is_finite()) {
        return Err(ParseError::InvalidSpec(format!(
            "weight systematic `{}` has non-finite up/down multiplier",
            raw.name
        )));
    }
    Ok(SystematicDef::Weight(WeightSystematicDef {
        name: raw.name,
        up: raw.up,
        down: raw.down,
    }))
}

fn shape_correction_def_from_raw(raw: RawCorrection) -> Result<ShapeCorrectionDef, ParseError> {
    validate_identifier(&raw.name, "correction name")?;
    validate_identifier(&raw.collection, "correction collection")?;
    validate_identifier(&raw.attr, "correction attribute")?;
    if raw.kind != "scale" {
        return Err(ParseError::InvalidSpec(format!(
            "correction `{}` has unsupported kind `{}`; expected `scale`",
            raw.name, raw.kind
        )));
    }
    if raw.attr != "pt" {
        return Err(ParseError::InvalidSpec(format!(
            "correction `{}` scales `{}`; this compiler slice only supports `pt`",
            raw.name, raw.attr
        )));
    }
    if !(raw.up.is_finite() && raw.down.is_finite()) {
        return Err(ParseError::InvalidSpec(format!(
            "shape correction `{}` has non-finite up/down scale factor",
            raw.name
        )));
    }
    Ok(ShapeCorrectionDef {
        name: raw.name,
        collection: raw.collection,
        attr: raw.attr,
        up: raw.up,
        down: raw.down,
    })
}

fn channel_def_from_raw(raw: RawChannel) -> Result<ChannelDef, ParseError> {
    validate_identifier(&raw.name, "channel name")?;
    Ok(ChannelDef {
        name: raw.name,
        objects: object_defs_from_raw(raw.objects)?,
        derived_objects: derived_defs_from_raw(raw.derived)?,
        regions: region_defs_from_raw(raw.regions)?,
        outputs: output_defs_from_raw(&raw.outputs)?,
    })
}

fn derived_object_def_from_raw(
    (name, raw): (String, RawDerivedObject),
) -> Result<DerivedObjectDef, ParseError> {
    validate_identifier(&name, "derived object name")?;

    let source = match raw.kind.as_str() {
        "pair" => {
            let object = raw.object.ok_or_else(|| {
                ParseError::InvalidSpec(format!("derived pair `{name}` is missing `object`"))
            })?;
            validate_identifier(&object, &format!("derived object `{name}` source object"))?;
            let constraints = raw
                .constraints
                .iter()
                .map(|constraint| pair_constraint_from_raw(&name, constraint))
                .collect::<Result<Vec<_>, _>>()?;
            let filters = raw
                .filters
                .iter()
                .map(|filter| parse_pair_filter(filter))
                .collect::<Result<Vec<_>, _>>()?;
            let selection = pair_selection_from_raw(
                &name,
                raw.selection.as_deref().unwrap_or("leading_pt"),
                raw.target.as_deref(),
            )?;
            for excluded in &raw.exclude {
                validate_identifier(excluded, &format!("derived pair `{name}` exclude"))?;
            }
            DerivedSource::Pair(ObjectPairDef {
                object,
                constraints,
                filters,
                selection,
                exclude: raw.exclude,
            })
        }
        "candidate" | "combine" => {
            if raw.object.is_some() {
                return Err(ParseError::InvalidSpec(format!(
                    "derived candidate `{name}` uses `items`, not `object`"
                )));
            }
            if raw.selection.is_some() || raw.target.is_some() || !raw.constraints.is_empty() {
                return Err(ParseError::InvalidSpec(format!(
                    "derived candidate `{name}` does not accept `selection`, `target`, or `constraints`"
                )));
            }
            for item in &raw.items {
                validate_identifier(item, &format!("derived candidate `{name}` item"))?;
            }
            let filters = raw
                .filters
                .iter()
                .map(|filter| parse_candidate_filter(filter))
                .collect::<Result<Vec<_>, _>>()?;
            DerivedSource::Candidate(ObjectCandidateDef {
                items: raw.items,
                filters,
            })
        }
        kind => {
            return Err(ParseError::InvalidSpec(format!(
                "derived object `{name}` has unsupported kind `{kind}`; expected `pair` or `combine`"
            )));
        }
    };

    Ok(DerivedObjectDef { name, source })
}

fn pair_constraint_from_raw(name: &str, constraint: &str) -> Result<PairConstraint, ParseError> {
    match constraint {
        "opposite_charge" => Ok(PairConstraint::OppositeCharge),
        "same_flavor" => Ok(PairConstraint::SameFlavor),
        other => Err(ParseError::InvalidSpec(format!(
            "derived object `{name}` has unsupported pair constraint `{other}`"
        ))),
    }
}

fn pair_selection_from_raw(
    name: &str,
    selection: &str,
    target: Option<&str>,
) -> Result<PairSelection, ParseError> {
    match selection {
        "leading_pt" => {
            if target.is_some() {
                return Err(ParseError::InvalidSpec(format!(
                    "derived object `{name}` selection `leading_pt` does not accept `target`"
                )));
            }
            Ok(PairSelection::LeadingPt)
        }
        "nearest_mass" => {
            let Some(target) = target else {
                return Err(ParseError::InvalidSpec(format!(
                    "derived object `{name}` selection `nearest_mass` requires `target`"
                )));
            };
            Ok(PairSelection::NearestMass {
                target: parse_quantity(target)?,
            })
        }
        "nearest_mass_truncated" => {
            let Some(target) = target else {
                return Err(ParseError::InvalidSpec(format!(
                    "derived object `{name}` selection `nearest_mass_truncated` requires `target`"
                )));
            };
            Ok(PairSelection::NearestMassTruncated {
                target: parse_quantity(target)?,
            })
        }
        other => Err(ParseError::InvalidSpec(format!(
            "derived object `{name}` has unsupported pair selection `{other}`"
        ))),
    }
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
                "unsupported spec format for `{}`; expected .toml, .yaml, .yml, .json, or .adl",
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
            Self::Literal(value) => write!(f, "{value}"),
            Self::Binary { op, lhs, rhs } => write!(f, "({lhs} {} {rhs})", op.as_str()),
            Self::Abs(inner) => write!(f, "abs({inner})"),
            Self::Sqrt(inner) => write!(f, "sqrt({inner})"),
            Self::Count(object) => write!(f, "count({object})"),
            Self::CountWhere { object, predicate } => write!(
                f,
                "count({object}, {} {} {})",
                predicate.lhs,
                predicate.op.as_str(),
                predicate.rhs
            ),
            Self::SumAttr { object, attr } => write!(f, "sum({object}.{attr})"),
            Self::All { object, predicate } => write!(
                f,
                "all({object}, {} {} {})",
                predicate.lhs,
                predicate.op.as_str(),
                predicate.rhs
            ),
            Self::Any { object, predicate } => write!(
                f,
                "any({object}, {} {} {})",
                predicate.lhs,
                predicate.op.as_str(),
                predicate.rhs
            ),
            Self::EitherPairPt {
                left,
                right,
                leading,
                subleading,
            } => write!(
                f,
                "either_pair_pt({left}, {right}, {leading}, {subleading})"
            ),
            Self::ClosestMass {
                left,
                right,
                target,
            } => write!(f, "closest_mass({left}, {right}, {target})"),
            Self::OtherMass {
                left,
                right,
                target,
            } => write!(f, "other_mass({left}, {right}, {target})"),
            Self::LeadingAttr { object, attr } => write!(f, "leading({object}).{attr}"),
            Self::PairDeltaR => f.write_str("dR"),
            Self::PairLeadingPt => f.write_str("leading_pt"),
            Self::PairSubleadingPt => f.write_str("subleading_pt"),
            Self::CandidateMinDeltaR => f.write_str("min_dR"),
            Self::CandidateLeadingPt => f.write_str("leading_pt"),
            Self::CandidateSubleadingPt => f.write_str("subleading_pt"),
        }
    }
}

impl ArithOp {
    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Pow => "^",
        }
    }
}

impl CmpOp {
    fn as_str(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.unit {
            Unit::GeV => write!(f, "{} GeV", self.value),
            Unit::Dimensionless => write!(f, "{}", self.value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MUON_SPEC_TOML: &str = include_str!("../examples/muon.toml");
    const MUON_SPEC_ADL: &str = include_str!("../examples/muon.adl");
    const MUON_TAGGER_SPEC_TOML: &str = include_str!("../examples/muon_tagger.toml");
    const DIMUON_SPEC_TOML: &str = include_str!("../examples/dimuon.toml");
    const DIMUON_SPEC_ADL: &str = include_str!("../examples/dimuon.adl");
    const MUON_WEIGHT_SYSTEMATIC_SPEC_TOML: &str =
        include_str!("../examples/muon_hist_weight_systematic.toml");
    const MUON_WEIGHT_SYSTEMATIC_SPEC_ADL: &str =
        include_str!("../examples/muon_hist_weight_systematic.adl");
    const MUON_SHAPE_CORRECTION_SPEC_TOML: &str =
        include_str!("../examples/muon_hist_shape_correction.toml");
    const MUON_SHAPE_CORRECTION_SPEC_ADL: &str =
        include_str!("../examples/muon_hist_shape_correction.adl");
    const HIGGS4MU_MINIMAL_SPEC_TOML: &str = include_str!("../examples/higgs4mu_minimal.toml");
    const HIGGS2E2MU_MINIMAL_SPEC_TOML: &str = include_str!("../examples/higgs2e2mu_minimal.toml");
    const EXAMPLE_SPECS: &[(&str, &str)] = &[
        ("dimuon.toml", include_str!("../examples/dimuon.toml")),
        (
            "higgs2e2mu_minimal.toml",
            include_str!("../examples/higgs2e2mu_minimal.toml"),
        ),
        ("higgs4l.toml", include_str!("../examples/higgs4l.toml")),
        (
            "higgs4l_2e2mu.toml",
            include_str!("../examples/higgs4l_2e2mu.toml"),
        ),
        (
            "higgs4l_4e.toml",
            include_str!("../examples/higgs4l_4e.toml"),
        ),
        (
            "higgs4l_all.toml",
            include_str!("../examples/higgs4l_all.toml"),
        ),
        (
            "higgs4mu_minimal.toml",
            include_str!("../examples/higgs4mu_minimal.toml"),
        ),
        ("muon.toml", include_str!("../examples/muon.toml")),
        (
            "muon_hist_nominal.toml",
            include_str!("../examples/muon_hist_nominal.toml"),
        ),
        (
            "muon_hist_weight_systematic.toml",
            include_str!("../examples/muon_hist_weight_systematic.toml"),
        ),
        (
            "muon_hist_shape_nominal.toml",
            include_str!("../examples/muon_hist_shape_nominal.toml"),
        ),
        (
            "muon_hist_shape_correction.toml",
            include_str!("../examples/muon_hist_shape_correction.toml"),
        ),
        (
            "muon_tagger.toml",
            include_str!("../examples/muon_tagger.toml"),
        ),
        (
            "selection_all.toml",
            include_str!("../examples/selection_all.toml"),
        ),
        (
            "selection_charge_balance.toml",
            include_str!("../examples/selection_charge_balance.toml"),
        ),
        (
            "selection_pair_dr.toml",
            include_str!("../examples/selection_pair_dr.toml"),
        ),
        (
            "selection_sip3d.toml",
            include_str!("../examples/selection_sip3d.toml"),
        ),
    ];
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
    fn parses_weight_systematic_surface() {
        let spec = AnalysisSpec::from_toml_str(include_str!(
            "../examples/muon_hist_weight_systematic.toml"
        ))
        .expect("parse weight systematic spec");

        assert_eq!(spec.systematics.len(), 2);
        let systematic = spec.weight_systematic().expect("weight systematic");
        assert_eq!(systematic.name, "muon_weight");
        assert_eq!(systematic.up, 2.0);
        assert_eq!(systematic.down, 0.5);
    }

    #[test]
    fn parses_shape_correction_surface() {
        let spec = AnalysisSpec::from_toml_str(include_str!(
            "../examples/muon_hist_shape_correction.toml"
        ))
        .expect("parse shape correction spec");

        assert_eq!(spec.shape_corrections.len(), 1);
        let correction = &spec.shape_corrections[0];
        assert_eq!(correction.name, "jes");
        assert_eq!(correction.collection, "good_muon");
        assert_eq!(correction.attr, "pt");
        assert_eq!(correction.up, 1.05);
        assert_eq!(correction.down, 0.95);
    }

    #[test]
    fn lowering_all_example_specs_succeeds() {
        let catalogue = catalogue();
        for (name, input) in EXAMPLE_SPECS {
            let spec =
                AnalysisSpec::from_toml_str(input).unwrap_or_else(|_| panic!("parse {name}"));
            lower(&spec, &catalogue).unwrap_or_else(|errors| panic!("lower {name}: {errors:?}"));
        }
    }

    #[test]
    fn lowering_muon_core_ir_has_stable_structure() {
        let spec = parse_muon_spec();
        let core = lower(&spec, &catalogue()).expect("lower muon spec");
        let calls = core
            .exprs
            .iter()
            .filter_map(|expr| match &expr.kind {
                core::ExprKind::Call { primitive, .. } => Some(*primitive),
                core::ExprKind::Literal(_)
                | core::ExprKind::Quantity(_)
                | core::ExprKind::Attr { .. }
                | core::ExprKind::DerivedAttr { .. }
                | core::ExprKind::Compare { .. } => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(core.name, "muon_demo");
        assert_eq!(core.objects.len(), 1);
        assert_eq!(core.objects[0].name, "good_muon");
        assert_eq!(core.objects[0].source.as_deref(), Some("Muon"));
        assert_eq!(core.regions.len(), 1);
        assert_eq!(core.outputs.len(), 2);
        assert_eq!(
            core.read_branches_ordered(),
            vec!["nMuon", "Muon_eta", "Muon_pt"]
        );
        assert_eq!(
            calls,
            vec![
                "abs",
                "object",
                "count",
                "object",
                "count",
                "object",
                "leading_attr",
            ]
        );
    }

    #[test]
    fn parses_dimuon_pair_into_typed_ir() {
        let spec = AnalysisSpec::from_toml_str(DIMUON_SPEC_TOML).expect("parse dimuon spec");

        assert_eq!(spec.derived_objects.len(), 1);
        assert_eq!(spec.derived_objects[0].name, "dimuon");
        assert_eq!(
            spec.derived_objects[0].source,
            DerivedSource::Pair(ObjectPairDef {
                object: "good_muon".to_string(),
                constraints: vec![PairConstraint::OppositeCharge],
                filters: vec![],
                selection: PairSelection::LeadingPt,
                exclude: vec![],
            })
        );
        assert_eq!(
            spec.outputs[0].expr,
            Expr::Attr {
                object: "dimuon".to_string(),
                attr: "mass".to_string(),
            }
        );
    }

    #[test]
    fn validation_derives_dimuon_pair_read_branches() {
        let spec = AnalysisSpec::from_toml_str(DIMUON_SPEC_TOML).expect("parse dimuon spec");
        let plan = validate(&spec, &catalogue()).expect("validate dimuon spec");
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
                ("Muon_charge", BranchType::VecI32),
                ("Muon_eta", BranchType::VecF32),
                ("Muon_mass", BranchType::VecF32),
                ("Muon_phi", BranchType::VecF32),
                ("Muon_pt", BranchType::VecF32),
            ]
        );
    }

    #[test]
    fn adl_examples_desugar_to_same_ir_and_plan_as_toml() {
        for (name, toml, adl) in [
            ("muon", MUON_SPEC_TOML, MUON_SPEC_ADL),
            ("dimuon", DIMUON_SPEC_TOML, DIMUON_SPEC_ADL),
            (
                "muon weight systematic",
                MUON_WEIGHT_SYSTEMATIC_SPEC_TOML,
                MUON_WEIGHT_SYSTEMATIC_SPEC_ADL,
            ),
            (
                "muon shape correction",
                MUON_SHAPE_CORRECTION_SPEC_TOML,
                MUON_SHAPE_CORRECTION_SPEC_ADL,
            ),
        ] {
            let toml_spec =
                AnalysisSpec::from_toml_str(toml).unwrap_or_else(|_| panic!("parse {name} TOML"));
            let adl_spec =
                AnalysisSpec::from_adl_str(adl).unwrap_or_else(|_| panic!("parse {name} ADL"));
            assert_eq!(adl_spec, toml_spec, "{name} AnalysisSpec differs");

            let catalogue = catalogue();
            let toml_core =
                lower(&toml_spec, &catalogue).unwrap_or_else(|_| panic!("lower {name} TOML"));
            let adl_core =
                lower(&adl_spec, &catalogue).unwrap_or_else(|_| panic!("lower {name} ADL"));
            assert_eq!(adl_core, toml_core, "{name} Core IR differs");

            let toml_plan =
                validate(&toml_spec, &catalogue).unwrap_or_else(|_| panic!("validate {name} TOML"));
            let adl_plan =
                validate(&adl_spec, &catalogue).unwrap_or_else(|_| panic!("validate {name} ADL"));
            assert_eq!(adl_plan.spec, toml_plan.spec, "{name} plan spec differs");
            assert_eq!(
                adl_plan.read_branches.specs(),
                toml_plan.read_branches.specs(),
                "{name} read branches differ"
            );
        }
    }

    #[test]
    fn adl_and_toml_plans_interpret_same_synthetic_events() {
        for (name, toml, adl) in [
            ("muon", MUON_SPEC_TOML, MUON_SPEC_ADL),
            ("dimuon", DIMUON_SPEC_TOML, DIMUON_SPEC_ADL),
            (
                "muon weight systematic",
                MUON_WEIGHT_SYSTEMATIC_SPEC_TOML,
                MUON_WEIGHT_SYSTEMATIC_SPEC_ADL,
            ),
            (
                "muon shape correction",
                MUON_SHAPE_CORRECTION_SPEC_TOML,
                MUON_SHAPE_CORRECTION_SPEC_ADL,
            ),
        ] {
            let catalogue = catalogue();
            let toml_spec =
                AnalysisSpec::from_toml_str(toml).unwrap_or_else(|_| panic!("parse {name} TOML"));
            let adl_spec =
                AnalysisSpec::from_adl_str(adl).unwrap_or_else(|_| panic!("parse {name} ADL"));
            let toml_plan =
                validate(&toml_spec, &catalogue).unwrap_or_else(|_| panic!("validate {name} TOML"));
            let adl_plan =
                validate(&adl_spec, &catalogue).unwrap_or_else(|_| panic!("validate {name} ADL"));
            let mut toml_histograms = interpret::InterpretedHistograms::new(&toml_plan);
            let mut adl_histograms = interpret::InterpretedHistograms::new(&adl_plan);

            for event in synthetic_muon_events() {
                let toml_row =
                    interpret::interpret_and_fill(&toml_plan, &event, &mut toml_histograms)
                        .expect("interpret TOML plan");
                let adl_row = interpret::interpret_and_fill(&adl_plan, &event, &mut adl_histograms)
                    .expect("interpret ADL plan");
                assert_eq!(adl_row, toml_row, "{name} interpreted row differs");
            }
            assert_eq!(
                adl_histograms, toml_histograms,
                "{name} interpreted histograms differ"
            );
        }
    }

    #[test]
    fn adl_rejects_malformed_surface() {
        let error = AnalysisSpec::from_adl_str("analysis bad year Run2018\nobject x : Muon {}")
            .expect_err("missing semicolon should fail");

        assert!(error.to_string().contains("expected `;`"));
    }

    #[test]
    fn adl_rejects_bad_cut_unit() {
        let error = AnalysisSpec::from_adl_str(
            r#"
analysis bad_unit year Run2018;
object good_muon : Muon {
  pt > 30 TeV;
}
"#,
        )
        .expect_err("bad unit should fail");

        assert!(error.to_string().contains("unsupported unit `TeV`"));
    }

    #[test]
    fn adl_rejects_malformed_systematic() {
        let bad_kind = AnalysisSpec::from_adl_str(
            r#"
analysis bad_systematic year Run2018;
weight nominal;
systematic muon_weight kind shape up 2.0 down 0.5;
object good_muon : Muon {}
"#,
        )
        .expect_err("bad systematic kind should fail");
        assert!(bad_kind
            .to_string()
            .contains("unsupported kind `shape`; expected `weight`"));

        let missing_down = AnalysisSpec::from_adl_str(
            r#"
analysis missing_systematic_down year Run2018;
weight nominal;
systematic muon_weight kind weight up 2.0;
object good_muon : Muon {}
"#,
        )
        .expect_err("missing systematic down factor should fail");
        assert!(missing_down.to_string().contains("expected `down`"));
    }

    #[test]
    fn adl_rejects_malformed_correction() {
        let bad_kind = AnalysisSpec::from_adl_str(
            r#"
analysis bad_correction year Run2018;
weight nominal;
correction jes kind shift collection good_muon attr pt up 1.05 down 0.95;
object good_muon : Muon {}
"#,
        )
        .expect_err("bad correction kind should fail");
        assert!(bad_kind
            .to_string()
            .contains("unsupported kind `shift`; expected `scale`"));

        let missing_up = AnalysisSpec::from_adl_str(
            r#"
analysis missing_correction_up year Run2018;
weight nominal;
correction jes kind scale collection good_muon attr pt down 0.95;
object good_muon : Muon {}
"#,
        )
        .expect_err("missing correction up factor should fail");
        assert!(missing_up.to_string().contains("expected `up`"));
    }

    #[test]
    fn adl_correction_unknown_collection_and_attr_reach_validator() {
        let unknown_collection = AnalysisSpec::from_adl_str(
            r#"
analysis unknown_collection year Run2018;
weight nominal;
correction jes kind scale collection ghost_muon attr pt up 1.05 down 0.95;
object good_muon : Muon {}
"#,
        )
        .expect("parse ADL with unknown correction collection");
        let errors =
            validate(&unknown_collection, &catalogue()).expect_err("validation should fail");
        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::UndefinedObject { object, .. } if object == "ghost_muon"
        )));

        let unknown_attr = AnalysisSpec::from_adl_str(
            r#"
analysis unknown_attr year Run2018;
weight nominal;
correction jes kind scale collection good_muon attr pt up 1.05 down 0.95;
object good_muon : Muon {}
"#,
        )
        .expect("parse ADL with correction attr missing from catalogue");
        let catalogue_text = NANOV9_CATALOGUE.replace("\"Muon_pt\":", "\"Muon_missing_pt\":");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(&catalogue_text, "v9").expect("parse catalogue");
        let errors = validate(&unknown_attr, &catalogue).expect_err("validation should fail");
        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::MissingBranch { branch, .. } if branch == "Muon_pt"
        )));
    }

    #[test]
    fn adl_rejects_undefined_output_alias() {
        let error = AnalysisSpec::from_adl_str(
            r#"
analysis bad_alias year Run2018;
object good_muon : Muon {}
output missing_alias;
"#,
        )
        .expect_err("undefined alias should fail");

        assert!(error
            .to_string()
            .contains("undefined alias `missing_alias`"));
    }

    #[test]
    fn adl_parses_histogram_declarations() {
        let spec = AnalysisSpec::from_adl_str(
            r#"
analysis hist_demo year Run2018;
object good_muon : Muon {
  pt > 30 GeV;
}
define lead_muon_pt = leading(good_muon).pt;
output lead_muon_pt;
histogram lead_muon_pt_hist {
  expr = lead_muon_pt;
  bins = 20;
  range = [0.0, 100.0];
}
"#,
        )
        .expect("parse ADL histogram");

        assert_eq!(spec.histograms.len(), 1);
        assert_eq!(spec.histograms[0].name, "lead_muon_pt_hist");
        assert_eq!(spec.histograms[0].bins, 20);
        assert_eq!(spec.histograms[0].range, [0.0, 100.0]);
        assert_eq!(
            spec.histograms[0].expr,
            Expr::LeadingAttr {
                object: "good_muon".to_string(),
                attr: "pt".to_string(),
            }
        );
    }

    #[test]
    fn adl_undefined_object_reaches_existing_validator() {
        let spec = AnalysisSpec::from_adl_str(
            r#"
analysis bad_object year Run2018;
object good_muon : Muon {}
region signal {
  count(ghost_muon) >= 1;
}
define n_good_muon = count(good_muon);
output n_good_muon;
"#,
        )
        .expect("parse ADL with undefined region object");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::UndefinedObject { object, .. } if object == "ghost_muon"
        )));
    }

    #[test]
    fn parses_nested_four_lepton_candidates_into_typed_ir() {
        let spec = AnalysisSpec::from_toml_str(HIGGS4MU_MINIMAL_SPEC_TOML).expect("parse 4mu spec");

        assert_eq!(spec.derived_objects.len(), 3);
        let z2 = spec
            .derived_objects
            .iter()
            .find(|derived| derived.name == "z2")
            .expect("z2 derived object");
        let h = spec
            .derived_objects
            .iter()
            .find(|derived| derived.name == "h")
            .expect("h derived object");
        assert!(matches!(
            &z2.source,
            DerivedSource::Pair(ObjectPairDef { exclude, .. }) if exclude == &vec!["z1".to_string()]
        ));
        assert_eq!(
            h.source,
            DerivedSource::Candidate(ObjectCandidateDef {
                items: vec!["z1".to_string(), "z2".to_string()],
                filters: vec![],
            })
        );
    }

    #[test]
    fn validation_derives_nested_four_lepton_read_branches() {
        let spec = AnalysisSpec::from_toml_str(HIGGS4MU_MINIMAL_SPEC_TOML).expect("parse 4mu spec");
        let plan = validate(&spec, &catalogue()).expect("validate 4mu spec");
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
                ("Muon_charge", BranchType::VecI32),
                ("Muon_eta", BranchType::VecF32),
                ("Muon_mass", BranchType::VecF32),
                ("Muon_phi", BranchType::VecF32),
                ("Muon_pt", BranchType::VecF32),
            ]
        );
    }

    #[test]
    fn validation_derives_cross_collection_candidate_read_branches() {
        let spec =
            AnalysisSpec::from_toml_str(HIGGS2E2MU_MINIMAL_SPEC_TOML).expect("parse 2e2mu spec");
        let plan = validate(&spec, &catalogue()).expect("validate 2e2mu spec");
        let read_branches = plan
            .read_branches
            .specs()
            .iter()
            .map(|spec| (spec.name.as_str(), spec.branch_type))
            .collect::<Vec<_>>();

        assert_eq!(
            read_branches,
            vec![
                ("nElectron", BranchType::U32),
                ("nMuon", BranchType::U32),
                ("Electron_eta", BranchType::VecF32),
                ("Electron_mass", BranchType::VecF32),
                ("Electron_phi", BranchType::VecF32),
                ("Electron_pt", BranchType::VecF32),
                ("Muon_eta", BranchType::VecF32),
                ("Muon_mass", BranchType::VecF32),
                ("Muon_phi", BranchType::VecF32),
                ("Muon_pt", BranchType::VecF32),
            ]
        );
    }

    #[test]
    fn validation_rejects_candidate_item_over_undefined_source() {
        let spec_text = HIGGS2E2MU_MINIMAL_SPEC_TOML.replace(
            "items = [\"z_mu\", \"z_el\"]",
            "items = [\"z_mu\", \"ghost\"]",
        );
        let spec = AnalysisSpec::from_toml_str(&spec_text).expect("parse modified spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::UndefinedObject { object, .. } if object == "ghost"
        )));
    }

    #[test]
    fn validation_rejects_remaining_from_incompatible_collection() {
        let spec_text = r#"
[analysis]
name = "bad_remaining"
year = "Run2012"

[objects.good_muon]
source = "Muon"
cuts = []

[objects.good_electron]
source = "Electron"
cuts = []

[derived.z_el]
kind = "pair"
object = "good_electron"
constraints = ["opposite_charge"]
selection = "leading_pt"

[derived.z_mu_remaining]
kind = "pair"
object = "good_muon"
constraints = ["opposite_charge"]
selection = "leading_pt"
exclude = ["z_el"]

[[outputs]]
name = "mass"
expr = "z_mu_remaining.mass"
"#;
        let spec = AnalysisSpec::from_toml_str(spec_text).expect("parse modified spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::InvalidExpression { detail, .. }
                if detail.contains("instead of `good_muon`")
        )));
    }

    #[test]
    fn validation_rejects_pair_over_undefined_object() {
        let spec_text = DIMUON_SPEC_TOML.replace("object = \"good_muon\"", "object = \"ghost\"");
        let spec = AnalysisSpec::from_toml_str(&spec_text).expect("parse modified dimuon spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::UndefinedObject { object, .. } if object == "ghost"
        )));
    }

    #[test]
    fn validation_rejects_unknown_derived_pair_attribute() {
        let spec_text = DIMUON_SPEC_TOML.replace("dimuon.mass", "dimuon.energy");
        let spec = AnalysisSpec::from_toml_str(&spec_text).expect("parse modified dimuon spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::InvalidExpression { detail, .. }
                if detail.contains("supported attributes are `mass` and `pt`")
        )));
    }

    #[test]
    fn validation_rejects_invariant_mass_without_four_vector_branch() {
        let catalogue_text = NANOV9_CATALOGUE.replace("\"Muon_mass\":", "\"Muon_notmass\":");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(&catalogue_text, "v9").expect("parse catalogue");
        let spec = AnalysisSpec::from_toml_str(DIMUON_SPEC_TOML).expect("parse dimuon spec");
        let errors = validate(&spec, &catalogue).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::MissingBranch { branch, .. } if branch == "Muon_mass"
        )));
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
        assert_eq!(
            SpecFormat::from_path("analysis.adl").unwrap(),
            SpecFormat::Adl
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

    fn synthetic_muon_events() -> Vec<nano_core::Event> {
        use nano_core::{BranchColumn, BranchSchema, BranchSpec};

        let schema = BranchSchema::new([
            BranchSpec::new("nMuon", BranchType::U32),
            BranchSpec::new("Muon_charge", BranchType::VecI32),
            BranchSpec::new("Muon_eta", BranchType::VecF32),
            BranchSpec::new("Muon_mass", BranchType::VecF32),
            BranchSpec::new("Muon_phi", BranchType::VecF32),
            BranchSpec::new("Muon_pt", BranchType::VecF32),
        ])
        .expect("schema");
        (0..3)
            .map(|entry| {
                nano_core::Event::from_columns(
                    schema.clone(),
                    [
                        ("nMuon", BranchColumn::U32(vec![2, 1, 2])),
                        (
                            "Muon_charge",
                            BranchColumn::VecI32(vec![vec![1, -1], vec![1], vec![1, 1]]),
                        ),
                        (
                            "Muon_eta",
                            BranchColumn::VecF32(vec![vec![0.1, 0.2], vec![2.5], vec![1.0, -1.0]]),
                        ),
                        (
                            "Muon_mass",
                            BranchColumn::VecF32(vec![
                                vec![0.105, 0.105],
                                vec![0.105],
                                vec![0.105, 0.105],
                            ]),
                        ),
                        (
                            "Muon_phi",
                            BranchColumn::VecF32(vec![vec![0.3, -0.4], vec![0.1], vec![1.0, -1.0]]),
                        ),
                        (
                            "Muon_pt",
                            BranchColumn::VecF32(vec![
                                vec![40.0, 35.0],
                                vec![29.0],
                                vec![45.0, 20.0],
                            ]),
                        ),
                    ],
                    entry,
                )
                .expect("event")
            })
            .collect()
    }
}
