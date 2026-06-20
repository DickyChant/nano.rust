//! Semantic analysis specifications for nano.rust.
//!
//! This crate implements the first semantic-IR slice: parse a physics-facing
//! YAML specification, validate it against a NanoAOD branch catalogue, and
//! derive the exact [`nano_core::BranchSchema`] needed by the streaming reader.

use nano_core::{BranchSchema, BranchSpec, BranchType};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt;

pub mod codegen;

/// Typed semantic analysis specification.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AnalysisSpec {
    pub name: String,
    pub year: Year,
    pub objects: Vec<ObjectDef>,
    pub regions: Vec<RegionDef>,
    pub outputs: Vec<OutputDef>,
}

impl AnalysisSpec {
    /// Parse an analysis specification from the physics-facing YAML form.
    pub fn from_yaml_str(input: &str) -> Result<Self, ParseError> {
        parse_analysis_spec(input)
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

/// Parse the physics-facing YAML spec into typed IR.
pub fn parse_analysis_spec(input: &str) -> Result<AnalysisSpec, ParseError> {
    let raw = parse_raw_analysis_spec(input)?;

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

    for object in &spec.objects {
        required.require_counter(&object.source);
        for (index, cut) in object.cuts.iter().enumerate() {
            validate_cut(
                object,
                index,
                cut,
                catalogue,
                &object_sources,
                &mut required,
                &mut errors,
            );
        }
    }

    for region in &spec.regions {
        for (index, requirement) in region.require.iter().enumerate() {
            validate_requirement(
                region,
                index,
                requirement,
                catalogue,
                &object_sources,
                &mut required,
                &mut errors,
            );
        }
    }

    for output in &spec.outputs {
        validate_expr(
            &output.expr,
            &format!("output `{}`", output.name),
            catalogue,
            &object_sources,
            &mut required,
            &mut errors,
        );
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

fn validate_cut(
    object: &ObjectDef,
    index: usize,
    cut: &Cut,
    catalogue: &Catalogue,
    object_sources: &HashMap<&str, &str>,
    required: &mut RequiredBranches,
    errors: &mut Vec<SpecError>,
) {
    let context = format!("object `{}` cut {}", object.name, index + 1);
    let lhs_type = validate_expr(
        &cut.lhs,
        &context,
        catalogue,
        object_sources,
        required,
        errors,
    );

    match lhs_type {
        Some(ExprType::Numeric(dimension)) => {
            validate_quantity_unit(&context, &cut.lhs, dimension, cut, errors)
        }
        Some(ExprType::Count) => errors.push(SpecError::InvalidExpression {
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
    catalogue: &Catalogue,
    object_sources: &HashMap<&str, &str>,
    required: &mut RequiredBranches,
    errors: &mut Vec<SpecError>,
) {
    let context = format!("region `{}` requirement {}", region.name, index + 1);
    validate_expr(
        &requirement.lhs,
        &context,
        catalogue,
        object_sources,
        required,
        errors,
    );
}

fn validate_expr(
    expr: &Expr,
    context: &str,
    catalogue: &Catalogue,
    object_sources: &HashMap<&str, &str>,
    required: &mut RequiredBranches,
    errors: &mut Vec<SpecError>,
) -> Option<ExprType> {
    match expr {
        Expr::Attr { object, attr } => validate_attr(
            object,
            attr,
            context,
            catalogue,
            object_sources,
            required,
            errors,
        ),
        Expr::Abs(inner) => {
            match validate_expr(inner, context, catalogue, object_sources, required, errors) {
                Some(ExprType::Numeric(dimension)) => Some(ExprType::Numeric(dimension)),
                Some(ExprType::Count) => {
                    errors.push(SpecError::InvalidExpression {
                        context: context.to_string(),
                        detail: "abs(...) requires a numeric attribute".to_string(),
                    });
                    None
                }
                None => None,
            }
        }
        Expr::Count(object) => {
            let Some(source) = object_sources.get(object.as_str()) else {
                errors.push(SpecError::UndefinedObject {
                    context: context.to_string(),
                    object: object.clone(),
                });
                return None;
            };
            required.require_counter(source);
            Some(ExprType::Count)
        }
        Expr::LeadingAttr { object, attr } => validate_attr(
            object,
            attr,
            context,
            catalogue,
            object_sources,
            required,
            errors,
        ),
    }
}

fn validate_attr(
    object: &str,
    attr: &str,
    context: &str,
    catalogue: &Catalogue,
    object_sources: &HashMap<&str, &str>,
    required: &mut RequiredBranches,
    errors: &mut Vec<SpecError>,
) -> Option<ExprType> {
    let Some(source) = object_sources.get(object) else {
        errors.push(SpecError::UndefinedObject {
            context: context.to_string(),
            object: object.to_string(),
        });
        return None;
    };

    let branch = format!("{source}_{attr}");
    let Some(entry) = catalogue.branch(&branch) else {
        errors.push(SpecError::MissingBranch {
            context: context.to_string(),
            branch,
        });
        return None;
    };

    let Some(branch_type) = entry.branch_type else {
        errors.push(SpecError::UnsupportedBranchType {
            context: context.to_string(),
            branch,
            raw_type: entry.raw_type.clone(),
        });
        return None;
    };

    if !is_numeric_vector(branch_type) {
        errors.push(SpecError::WrongBranchType {
            context: context.to_string(),
            branch,
            expected: "numeric vector branch".to_string(),
            actual: branch_type,
        });
        return None;
    }

    required.require_counter(source);
    required.require_attr(source, attr);
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
}

impl RequiredBranches {
    fn require_counter(&mut self, source: &str) {
        self.counters.insert(source.to_string());
    }

    fn require_attr(&mut self, source: &str, attr: &str) {
        self.attrs.insert((source.to_string(), attr.to_string()));
    }

    fn to_branch_specs(&self, catalogue: &Catalogue) -> Result<Vec<BranchSpec>, SpecError> {
        let mut specs = Vec::with_capacity(self.counters.len() + self.attrs.len());

        for source in &self.counters {
            let branch = format!("n{source}");
            let branch_type = catalogue_branch_type(catalogue, &branch, "derived read_branches")?;
            specs.push(BranchSpec::new(branch, branch_type));
        }

        for (source, attr) in &self.attrs {
            let branch = format!("{source}_{attr}");
            let branch_type = catalogue_branch_type(catalogue, &branch, "derived read_branches")?;
            specs.push(BranchSpec::new(branch, branch_type));
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

fn parse_raw_analysis_spec(input: &str) -> Result<RawAnalysisSpec, ParseError> {
    let mut analysis = None;
    let mut objects = BTreeMap::new();
    let mut regions = BTreeMap::new();
    let mut outputs = Vec::new();
    let mut section = Section::None;
    let mut current_object: Option<String> = None;
    let mut current_region: Option<String> = None;

    for line in input.lines() {
        let without_comment = strip_comment(line);
        let trimmed = without_comment.trim();
        if trimmed.is_empty() {
            continue;
        }

        match trimmed {
            "objects:" => {
                section = Section::Objects;
                current_object = None;
                current_region = None;
                continue;
            }
            "regions:" => {
                section = Section::Regions;
                current_object = None;
                current_region = None;
                continue;
            }
            "outputs:" => {
                section = Section::Outputs;
                current_object = None;
                current_region = None;
                continue;
            }
            _ => {}
        }

        if let Some(rest) = trimmed.strip_prefix("analysis:") {
            let fields = parse_inline_map(rest.trim())?;
            let name = required_field(&fields, "name", "analysis")?.to_string();
            let year = required_field(&fields, "year", "analysis")?.to_string();
            analysis = Some(RawAnalysis { name, year });
            continue;
        }

        match section {
            Section::Objects => {
                let indent = indentation(line);
                if indent == 2 && trimmed.ends_with(':') {
                    let name = trimmed.trim_end_matches(':').trim().to_string();
                    validate_identifier(&name, trimmed)?;
                    objects.insert(
                        name.clone(),
                        RawObject {
                            source: String::new(),
                            cuts: Vec::new(),
                        },
                    );
                    current_object = Some(name);
                    continue;
                }

                let object_name = current_object.as_deref().ok_or_else(|| {
                    ParseError::InvalidSpec(format!("object field outside object: `{trimmed}`"))
                })?;
                let object = objects.get_mut(object_name).ok_or_else(|| {
                    ParseError::InvalidSpec(format!(
                        "internal parser error for object `{object_name}`"
                    ))
                })?;
                if let Some(rest) = trimmed.strip_prefix("source:") {
                    object.source = unquote(rest.trim()).to_string();
                } else if let Some(rest) = trimmed.strip_prefix("cuts:") {
                    object.cuts = parse_inline_list(rest.trim())?;
                }
            }
            Section::Regions => {
                let indent = indentation(line);
                if indent == 2 && trimmed.ends_with(':') {
                    let name = trimmed.trim_end_matches(':').trim().to_string();
                    validate_identifier(&name, trimmed)?;
                    regions.insert(
                        name.clone(),
                        RawRegion {
                            require: Vec::new(),
                        },
                    );
                    current_region = Some(name);
                    continue;
                }

                let region_name = current_region.as_deref().ok_or_else(|| {
                    ParseError::InvalidSpec(format!("region field outside region: `{trimmed}`"))
                })?;
                let region = regions.get_mut(region_name).ok_or_else(|| {
                    ParseError::InvalidSpec(format!(
                        "internal parser error for region `{region_name}`"
                    ))
                })?;
                if let Some(rest) = trimmed.strip_prefix("require:") {
                    region.require = parse_inline_list(rest.trim())?;
                }
            }
            Section::Outputs => {
                if let Some(rest) = trimmed.strip_prefix("- ") {
                    let fields = parse_inline_map(rest.trim())?;
                    outputs.push(RawOutput {
                        name: required_field(&fields, "name", "output")?.to_string(),
                        expr: required_field(&fields, "expr", "output")?.to_string(),
                    });
                }
            }
            Section::None => {}
        }
    }

    let analysis =
        analysis.ok_or_else(|| ParseError::InvalidSpec("missing analysis block".to_string()))?;
    for (name, object) in &objects {
        if object.source.is_empty() {
            return Err(ParseError::InvalidSpec(format!(
                "object `{name}` is missing source"
            )));
        }
    }

    Ok(RawAnalysisSpec {
        analysis,
        objects,
        regions,
        outputs,
    })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Objects,
    Regions,
    Outputs,
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(before, _)| before)
}

fn indentation(line: &str) -> usize {
    line.chars().take_while(|ch| *ch == ' ').count()
}

fn parse_inline_map(input: &str) -> Result<BTreeMap<String, String>, ParseError> {
    let inner = input
        .trim()
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .ok_or_else(|| ParseError::InvalidSpec(format!("expected inline map, got `{input}`")))?;
    let mut fields = BTreeMap::new();
    for item in split_csv(inner) {
        let (key, value) = item
            .split_once(':')
            .ok_or_else(|| ParseError::InvalidSpec(format!("expected key: value in `{item}`")))?;
        fields.insert(key.trim().to_string(), unquote(value.trim()).to_string());
    }
    Ok(fields)
}

fn parse_inline_list(input: &str) -> Result<Vec<String>, ParseError> {
    let inner = input
        .trim()
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or_else(|| ParseError::InvalidSpec(format!("expected inline list, got `{input}`")))?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    Ok(split_csv(inner)
        .into_iter()
        .map(|item| unquote(item.trim()).to_string())
        .collect())
}

fn split_csv(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quote = false;

    for (index, ch) in input.char_indices() {
        if ch == '"' {
            in_quote = !in_quote;
        } else if ch == ',' && !in_quote {
            parts.push(input[start..index].trim());
            start = index + 1;
        }
    }

    parts.push(input[start..].trim());
    parts
}

fn unquote(input: &str) -> &str {
    input
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(input)
}

fn required_field<'a>(
    fields: &'a BTreeMap<String, String>,
    key: &str,
    context: &str,
) -> Result<&'a str, ParseError> {
    fields
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| ParseError::InvalidSpec(format!("{context} is missing `{key}`")))
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

#[derive(Debug)]
struct RawAnalysisSpec {
    analysis: RawAnalysis,
    objects: BTreeMap<String, RawObject>,
    regions: BTreeMap<String, RawRegion>,
    outputs: Vec<RawOutput>,
}

#[derive(Debug)]
struct RawAnalysis {
    name: String,
    year: String,
}

#[derive(Debug)]
struct RawObject {
    source: String,
    cuts: Vec<String>,
}

#[derive(Debug)]
struct RawRegion {
    require: Vec<String>,
}

#[derive(Debug)]
struct RawOutput {
    name: String,
    expr: String,
}

/// Spec/cut parsing errors.
#[derive(Debug)]
pub enum ParseError {
    InvalidSpec(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSpec(message) => f.write_str(message),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        None
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

    const MUON_SPEC: &str = include_str!("../examples/muon.yaml");
    const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");

    fn catalogue() -> Catalogue {
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse nanov9 catalogue")
    }

    fn parse_muon_spec() -> AnalysisSpec {
        AnalysisSpec::from_yaml_str(MUON_SPEC).expect("parse muon spec")
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
    fn validation_rejects_nonexistent_branch() {
        let yaml = MUON_SPEC.replace("abs(eta) < 2.4", "abs(nope) < 2.4");
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
        let yaml = MUON_SPEC.replace("pt > 30 GeV", "pt > 30");
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
        let yaml = MUON_SPEC.replace("count(good_muon) >= 1", "count(ghost_muon) >= 1");
        let spec = AnalysisSpec::from_yaml_str(&yaml).expect("parse modified spec");
        let errors = validate(&spec, &catalogue()).expect_err("validation should fail");

        assert!(errors.iter().any(|error| matches!(
            error,
            SpecError::UndefinedObject { object, .. } if object == "ghost_muon"
        )));
    }

    #[test]
    fn validation_rejects_wrong_branch_type() {
        let yaml = MUON_SPEC.replace("abs(eta) < 2.4", "looseId > 0");
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
