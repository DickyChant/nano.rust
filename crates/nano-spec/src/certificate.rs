//! Preservation certificates for validated analysis plans.
//!
//! A certificate is a stable, serializable summary of the facts that validation
//! and lowering agree on: dependencies, produced values, systematic axes, model
//! outputs, Core effects, and stage fingerprints.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use nano_core::{split_branch_name, BranchSpec, BranchType};

use crate::core::{self, CoreIr};
use crate::kir::{self, KirProgram};
use crate::{
    AnalysisSpec, Catalogue, CatalogueBranch, Dimension, HistogramDef, ModelProviderKind,
    ResolvedPlan, ShapeCorrectionDef, SystematicDef, Unit, WeightSystematicDef,
};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanCertificate {
    pub analysis: String,
    pub required_branches: Vec<BranchCertificate>,
    pub outputs: Vec<ValueCertificate>,
    pub histograms: Vec<HistogramCertificate>,
    pub systematic_axis: SystematicAxisCertificate,
    pub shape_corrections: Vec<ShapeCorrectionCertificate>,
    pub weight_systematics: Vec<WeightSystematicCertificate>,
    pub model_outputs: Vec<ModelOutputCertificate>,
    pub effects: Vec<EffectCertificate>,
    pub stage_hashes: StageHashes,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BranchCertificate {
    pub name: String,
    pub branch_type: String,
    pub optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<Dimension>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<Unit>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ValueCertificate {
    pub name: String,
    pub value_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<Dimension>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HistogramCertificate {
    pub name: String,
    pub bins: usize,
    pub range: [f64; 2],
    pub value_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimension: Option<Dimension>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SystematicAxisCertificate {
    pub variations: Vec<VariationCertificate>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VariationCertificate {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShapeCorrectionCertificate {
    pub name: String,
    pub collection: String,
    pub attr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub down: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correction: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WeightSystematicCertificate {
    pub name: String,
    pub up: f64,
    pub down: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelOutputCertificate {
    pub model: String,
    pub output: String,
    pub output_dtype: String,
    pub batch: String,
    pub inputs: Vec<String>,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EffectCertificate {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StageHashes {
    pub core: String,
    pub kir: String,
}

#[derive(Debug)]
pub enum CertificateError {
    CoreLowering(Vec<crate::SpecError>),
    KirLowering(kir::KirError),
}

impl fmt::Display for CertificateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoreLowering(errors) => {
                let message = errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "failed to lower validated plan to Core IR: {message}")
            }
            Self::KirLowering(error) => {
                write!(f, "failed to lower validated plan to KIR: {error}")
            }
        }
    }
}

impl Error for CertificateError {}

/// Compute a preservation certificate for a validated plan.
///
/// This panics only if the validated plan can no longer be lowered through the
/// existing Core/KIR pipeline, which would indicate internal compiler drift.
pub fn certify(plan: &ResolvedPlan) -> PlanCertificate {
    try_certify(plan).expect("validated plan should certify")
}

/// Fallible certificate construction for callers that want structured errors.
pub fn try_certify(plan: &ResolvedPlan) -> Result<PlanCertificate, CertificateError> {
    let catalogue = catalogue_from_plan(plan);
    let core = crate::lower(&plan.spec, &catalogue).map_err(CertificateError::CoreLowering)?;
    let kir = kir::lower_plan_to_kir(plan)
        .or_else(|_| kir::lower_to_kir(&core))
        .map_err(CertificateError::KirLowering)?;

    let required_branches = branch_certificates(plan.read_branches.specs());
    let outputs = output_certificates(&core);
    let histograms = histogram_certificates(&plan.spec, &core);
    let systematic_axis = systematic_axis_certificate(&plan.spec);
    let shape_corrections = shape_correction_certificates(&plan.spec.shape_corrections);
    let weight_systematics = weight_systematic_certificates(&plan.spec.systematics);
    let model_outputs = model_output_certificates(&plan.spec);
    let effects = effect_certificates(&core);
    let stage_hashes = StageHashes {
        core: stable_hash(&core_summary(&core)),
        kir: stable_hash(&kir_summary(&kir)),
    };

    let body = CertificateBody {
        analysis: core.name.clone(),
        required_branches,
        outputs,
        histograms,
        systematic_axis,
        shape_corrections,
        weight_systematics,
        model_outputs,
        effects,
        stage_hashes,
    };
    let hash = stable_hash(&body);

    Ok(PlanCertificate {
        analysis: body.analysis,
        required_branches: body.required_branches,
        outputs: body.outputs,
        histograms: body.histograms,
        systematic_axis: body.systematic_axis,
        shape_corrections: body.shape_corrections,
        weight_systematics: body.weight_systematics,
        model_outputs: body.model_outputs,
        effects: body.effects,
        stage_hashes: body.stage_hashes,
        hash,
    })
}

#[derive(Debug, serde::Serialize)]
struct CertificateBody {
    analysis: String,
    required_branches: Vec<BranchCertificate>,
    outputs: Vec<ValueCertificate>,
    histograms: Vec<HistogramCertificate>,
    systematic_axis: SystematicAxisCertificate,
    shape_corrections: Vec<ShapeCorrectionCertificate>,
    weight_systematics: Vec<WeightSystematicCertificate>,
    model_outputs: Vec<ModelOutputCertificate>,
    effects: Vec<EffectCertificate>,
    stage_hashes: StageHashes,
}

fn catalogue_from_plan(plan: &ResolvedPlan) -> Catalogue {
    Catalogue {
        branches: plan
            .read_branches
            .specs()
            .iter()
            .map(|spec| {
                (
                    spec.name.clone(),
                    CatalogueBranch {
                        branch_type: Some(spec.branch_type),
                        raw_type: branch_type_name(spec.branch_type),
                    },
                )
            })
            .collect(),
    }
}

fn branch_certificates(branches: &[BranchSpec]) -> Vec<BranchCertificate> {
    let mut branches = branches
        .iter()
        .map(|branch| {
            let dimension = branch_dimension(&branch.name, branch.branch_type);
            BranchCertificate {
                name: branch.name.clone(),
                branch_type: branch_type_name(branch.branch_type),
                optional: branch.optional,
                dimension,
                unit: dimension.map(unit_for_dimension),
            }
        })
        .collect::<Vec<_>>();
    branches.sort_by(|left, right| left.name.cmp(&right.name));
    branches
}

fn output_certificates(core: &CoreIr) -> Vec<ValueCertificate> {
    let mut outputs = core
        .outputs
        .iter()
        .map(|(name, expr)| value_certificate(name, &core.expr(*expr).ty))
        .collect::<Vec<_>>();
    outputs.sort_by(|left, right| left.name.cmp(&right.name));
    outputs
}

fn histogram_certificates(spec: &AnalysisSpec, core: &CoreIr) -> Vec<HistogramCertificate> {
    let hist_defs = spec
        .histograms
        .iter()
        .map(|histogram| (histogram.name.as_str(), histogram))
        .collect::<BTreeMap<_, _>>();
    let mut histograms = core
        .histograms
        .iter()
        .filter_map(|(name, expr)| {
            let def = hist_defs.get(name.as_str())?;
            let value = value_type_certificate(&core.expr(*expr).ty);
            Some(HistogramCertificate {
                name: name.clone(),
                bins: def.bins,
                range: def.range,
                value_kind: value.value_kind,
                dimension: value.dimension,
            })
        })
        .collect::<Vec<_>>();
    histograms.sort_by(|left, right| left.name.cmp(&right.name));
    histograms
}

fn systematic_axis_certificate(spec: &AnalysisSpec) -> SystematicAxisCertificate {
    let mut variations = BTreeSet::new();
    for systematic in &spec.systematics {
        match systematic {
            SystematicDef::Nominal => {
                variations.insert(variation("Nominal", "nominal"));
            }
            SystematicDef::JesUp => {
                variations.insert(variation("JesUp", "shape"));
            }
            SystematicDef::JesDown => {
                variations.insert(variation("JesDown", "shape"));
            }
            SystematicDef::JerUp => {
                variations.insert(variation("JerUp", "shape"));
            }
            SystematicDef::JerDown => {
                variations.insert(variation("JerDown", "shape"));
            }
            SystematicDef::Weight(systematic) => {
                variations.insert(variation(format!("{}:up", systematic.name), "weight"));
                variations.insert(variation(format!("{}:down", systematic.name), "weight"));
            }
        }
    }
    for correction in &spec.shape_corrections {
        variations.insert(variation(format!("{}:up", correction.name), "shape"));
        variations.insert(variation(format!("{}:down", correction.name), "shape"));
    }
    if variations.is_empty() {
        variations.insert(variation("Nominal", "nominal"));
    }
    SystematicAxisCertificate {
        variations: variations.into_iter().collect(),
    }
}

fn variation(name: impl Into<String>, kind: impl Into<String>) -> VariationCertificate {
    VariationCertificate {
        name: name.into(),
        kind: kind.into(),
    }
}

fn shape_correction_certificates(
    corrections: &[ShapeCorrectionDef],
) -> Vec<ShapeCorrectionCertificate> {
    let mut corrections = corrections
        .iter()
        .map(|correction| ShapeCorrectionCertificate {
            name: correction.name.clone(),
            collection: correction.collection.clone(),
            attr: correction.attr.clone(),
            kind: match &correction.payload {
                crate::ShapeCorrectionPayload::Scale { .. } => None,
                crate::ShapeCorrectionPayload::Jes { .. } => Some("jes".to_string()),
            },
            up: correction.fixed_scale_factors().map(|(up, _)| up),
            down: correction.fixed_scale_factors().map(|(_, down)| down),
            file: correction
                .jes_payload()
                .map(|(file, _, _)| file.to_string()),
            correction: correction
                .jes_payload()
                .map(|(_, correction, _)| correction.to_string()),
            inputs: correction
                .jes_payload()
                .map(|(_, _, inputs)| inputs.iter().map(|input| input.name.clone()).collect())
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    corrections.sort_by(|left, right| left.name.cmp(&right.name));
    corrections
}

fn weight_systematic_certificates(
    systematics: &[SystematicDef],
) -> Vec<WeightSystematicCertificate> {
    let mut systematics = systematics
        .iter()
        .filter_map(|systematic| match systematic {
            SystematicDef::Weight(systematic) => Some(weight_systematic_certificate(systematic)),
            SystematicDef::Nominal
            | SystematicDef::JesUp
            | SystematicDef::JesDown
            | SystematicDef::JerUp
            | SystematicDef::JerDown => None,
        })
        .collect::<Vec<_>>();
    systematics.sort_by(|left, right| left.name.cmp(&right.name));
    systematics
}

fn weight_systematic_certificate(systematic: &WeightSystematicDef) -> WeightSystematicCertificate {
    WeightSystematicCertificate {
        name: systematic.name.clone(),
        up: systematic.up,
        down: systematic.down,
    }
}

fn model_output_certificates(spec: &AnalysisSpec) -> Vec<ModelOutputCertificate> {
    let mut outputs = spec
        .models
        .iter()
        .map(|model| ModelOutputCertificate {
            model: model.name.clone(),
            output: model.output.clone(),
            output_dtype: format!("{:?}", model.output_dtype),
            batch: model.batch.clone(),
            inputs: sorted_strings(model.inputs.iter().cloned()),
            provider: provider_name(&model.provider.kind),
        })
        .collect::<Vec<_>>();
    outputs.sort_by(|left, right| {
        left.output
            .cmp(&right.output)
            .then(left.model.cmp(&right.model))
    });
    outputs
}

fn effect_certificates(core: &CoreIr) -> Vec<EffectCertificate> {
    let mut effects = core
        .effects
        .iter()
        .map(|effect| match effect {
            core::Effect::ReadsBranch(branch) => EffectCertificate {
                kind: "ReadsBranch".to_string(),
                value: branch.clone(),
            },
            core::Effect::RequiresModel(model_id) => {
                let model = &core.models[model_id.0];
                EffectCertificate {
                    kind: "RequiresModel".to_string(),
                    value: format!("{}:{}", model.name, model.output),
                }
            }
            core::Effect::ProducesScore(output) => EffectCertificate {
                kind: "ProducesScore".to_string(),
                value: output.clone(),
            },
            core::Effect::ShapeDependsOn(axis_id) => EffectCertificate {
                kind: "ShapeDependsOn".to_string(),
                value: format!("axis_{}", axis_id.0),
            },
            core::Effect::RequiresCompat(mode) => EffectCertificate {
                kind: "RequiresCompat".to_string(),
                value: format!("{mode:?}"),
            },
        })
        .collect::<Vec<_>>();
    effects.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then(left.value.cmp(&right.value))
    });
    effects
}

fn value_certificate(name: &str, ty: &core::Type) -> ValueCertificate {
    let value = value_type_certificate(ty);
    ValueCertificate {
        name: name.to_string(),
        value_kind: value.value_kind,
        dimension: value.dimension,
    }
}

fn value_type_certificate(ty: &core::Type) -> ValueCertificate {
    ValueCertificate {
        name: String::new(),
        value_kind: match ty {
            core::Type::Bool => "bool",
            core::Type::Int => "int",
            core::Type::Float => "float",
            core::Type::Quantity(_) => "quantity",
            core::Type::ObjectSet => "object_set",
            core::Type::Candidate => "candidate",
            core::Type::Event => "event",
            core::Type::Weight => "weight",
            core::Type::Systematic => "systematic",
            core::Type::Tensor => "tensor",
            core::Type::Histogram => "histogram",
        }
        .to_string(),
        dimension: ty.numeric_dimension(),
    }
}

fn branch_dimension(name: &str, branch_type: BranchType) -> Option<Dimension> {
    if name.starts_with('n') && !branch_type.is_vector() {
        return Some(Dimension::Dimensionless);
    }
    split_branch_name(name).map(|(_, attr)| crate::attribute_dimension(attr))
}

fn unit_for_dimension(dimension: Dimension) -> Unit {
    match dimension {
        Dimension::Momentum => Unit::GeV,
        Dimension::Dimensionless => Unit::Dimensionless,
    }
}

fn branch_type_name(branch_type: BranchType) -> String {
    format!("{branch_type:?}")
}

fn provider_name(kind: &ModelProviderKind) -> String {
    match kind {
        ModelProviderKind::Mock => "Mock".to_string(),
        ModelProviderKind::InProcess => "InProcess".to_string(),
        ModelProviderKind::Remote => "Remote".to_string(),
        ModelProviderKind::Managed => "Managed".to_string(),
        ModelProviderKind::Other(kind) => format!("Other({kind})"),
    }
}

#[derive(Debug, serde::Serialize)]
struct CoreSummary {
    name: String,
    objects: Vec<ObjectSummary>,
    regions: Vec<String>,
    models: Vec<ModelOutputCertificate>,
    exprs: Vec<CoreExprSummary>,
    outputs: Vec<(String, usize)>,
    histograms: Vec<(String, usize)>,
    effects: Vec<EffectCertificate>,
}

#[derive(Debug, serde::Serialize)]
struct ObjectSummary {
    name: String,
    source: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct CoreExprSummary {
    id: usize,
    kind: String,
    value_kind: String,
    dimension: Option<Dimension>,
    effects: Vec<EffectCertificate>,
}

fn core_summary(core: &CoreIr) -> CoreSummary {
    CoreSummary {
        name: core.name.clone(),
        objects: core
            .objects
            .iter()
            .map(|object| ObjectSummary {
                name: object.name.clone(),
                source: object.source.clone(),
            })
            .collect(),
        regions: core
            .regions
            .iter()
            .map(|region| region.name.clone())
            .collect(),
        models: core
            .models
            .iter()
            .map(|model| ModelOutputCertificate {
                model: model.name.clone(),
                output: model.output.clone(),
                output_dtype: "F32".to_string(),
                batch: String::new(),
                inputs: Vec::new(),
                provider: String::new(),
            })
            .collect(),
        exprs: core
            .exprs
            .iter()
            .map(|expr| {
                let value = value_type_certificate(&expr.ty);
                let mut effects = expr
                    .effects
                    .iter()
                    .map(|effect| effect_certificate_for_expr(core, effect))
                    .collect::<Vec<_>>();
                effects.sort_by(|left, right| {
                    left.kind
                        .cmp(&right.kind)
                        .then(left.value.cmp(&right.value))
                });
                CoreExprSummary {
                    id: expr.id.0,
                    kind: expr_kind_name(&expr.kind),
                    value_kind: value.value_kind,
                    dimension: value.dimension,
                    effects,
                }
            })
            .collect(),
        outputs: core
            .outputs
            .iter()
            .map(|(name, expr)| (name.clone(), expr.0))
            .collect(),
        histograms: core
            .histograms
            .iter()
            .map(|(name, expr)| (name.clone(), expr.0))
            .collect(),
        effects: effect_certificates(core),
    }
}

fn effect_certificate_for_expr(core: &CoreIr, effect: &core::Effect) -> EffectCertificate {
    match effect {
        core::Effect::ReadsBranch(branch) => EffectCertificate {
            kind: "ReadsBranch".to_string(),
            value: branch.clone(),
        },
        core::Effect::RequiresModel(model_id) => {
            let model = &core.models[model_id.0];
            EffectCertificate {
                kind: "RequiresModel".to_string(),
                value: format!("{}:{}", model.name, model.output),
            }
        }
        core::Effect::ProducesScore(output) => EffectCertificate {
            kind: "ProducesScore".to_string(),
            value: output.clone(),
        },
        core::Effect::ShapeDependsOn(axis_id) => EffectCertificate {
            kind: "ShapeDependsOn".to_string(),
            value: format!("axis_{}", axis_id.0),
        },
        core::Effect::RequiresCompat(mode) => EffectCertificate {
            kind: "RequiresCompat".to_string(),
            value: format!("{mode:?}"),
        },
    }
}

fn expr_kind_name(kind: &core::ExprKind) -> String {
    match kind {
        core::ExprKind::Literal(value) => format!("literal:{value:?}"),
        core::ExprKind::Quantity(quantity) => {
            format!("quantity:{:?}:{:?}", quantity.value, quantity.unit)
        }
        core::ExprKind::EventScalar { branch } => format!("event_scalar:{branch}"),
        core::ExprKind::Attr {
            object,
            attr,
            branch,
        } => format!(
            "attr:{}:{attr}:{}",
            object.0,
            branch.as_deref().unwrap_or("")
        ),
        core::ExprKind::DerivedAttr { object, attr } => format!("derived_attr:{}:{attr}", object.0),
        core::ExprKind::Call { primitive, args } => {
            let args = args
                .iter()
                .map(|arg| arg.0.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!("call:{primitive}({args})")
        }
        core::ExprKind::Compare { op, lhs, rhs } => {
            format!("compare:{op:?}:{}:{}", lhs.0, rhs.0)
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct KirSummary {
    name: String,
    read_branches: Vec<BranchCertificate>,
    objects: Vec<String>,
    derived_objects: Vec<String>,
    regions: Vec<String>,
    outputs: Vec<String>,
    histograms: Vec<HistogramCertificate>,
    systematics: Vec<VariationCertificate>,
    shape_corrections: Vec<ShapeCorrectionCertificate>,
    statements: Vec<String>,
}

fn kir_summary(kir: &KirProgram) -> KirSummary {
    KirSummary {
        name: kir.name.clone(),
        read_branches: branch_certificates(&kir.read_branches),
        objects: kir
            .objects
            .iter()
            .map(|object| object.name.clone())
            .collect(),
        derived_objects: kir
            .derived_objects
            .iter()
            .map(|object| object.name.clone())
            .collect(),
        regions: kir
            .regions
            .iter()
            .map(|region| region.name.clone())
            .collect(),
        outputs: kir
            .outputs
            .iter()
            .map(|output| output.name.clone())
            .collect(),
        histograms: kir
            .histograms
            .iter()
            .map(|histogram| histogram_certificate_from_def(&histogram.def))
            .collect(),
        systematics: systematic_variations_from_defs(&kir.systematics),
        shape_corrections: kir
            .shape_corrections
            .iter()
            .map(|correction| ShapeCorrectionCertificate {
                name: correction.name.clone(),
                collection: correction.collection.clone(),
                attr: correction.attr.clone(),
                kind: match &correction.payload {
                    kir::KirShapeCorrectionPayload::Scale { .. } => None,
                    kir::KirShapeCorrectionPayload::Jes { .. } => Some("jes".to_string()),
                },
                up: match &correction.payload {
                    kir::KirShapeCorrectionPayload::Scale { up, .. } => Some(*up),
                    kir::KirShapeCorrectionPayload::Jes { .. } => None,
                },
                down: match &correction.payload {
                    kir::KirShapeCorrectionPayload::Scale { down, .. } => Some(*down),
                    kir::KirShapeCorrectionPayload::Jes { .. } => None,
                },
                file: match &correction.payload {
                    kir::KirShapeCorrectionPayload::Scale { .. } => None,
                    kir::KirShapeCorrectionPayload::Jes { file, .. } => Some(file.clone()),
                },
                correction: match &correction.payload {
                    kir::KirShapeCorrectionPayload::Scale { .. } => None,
                    kir::KirShapeCorrectionPayload::Jes { correction, .. } => {
                        Some(correction.clone())
                    }
                },
                inputs: match &correction.payload {
                    kir::KirShapeCorrectionPayload::Scale { .. } => Vec::new(),
                    kir::KirShapeCorrectionPayload::Jes { inputs, .. } => {
                        inputs.iter().map(|input| input.name.clone()).collect()
                    }
                },
            })
            .collect(),
        statements: block_statements(&kir.block),
    }
}

fn histogram_certificate_from_def(def: &HistogramDef) -> HistogramCertificate {
    HistogramCertificate {
        name: def.name.clone(),
        bins: def.bins,
        range: def.range,
        value_kind: "unknown".to_string(),
        dimension: None,
    }
}

fn systematic_variations_from_defs(systematics: &[SystematicDef]) -> Vec<VariationCertificate> {
    let mut variations = BTreeSet::new();
    for systematic in systematics {
        match systematic {
            SystematicDef::Nominal => {
                variations.insert(variation("Nominal", "nominal"));
            }
            SystematicDef::JesUp => {
                variations.insert(variation("JesUp", "shape"));
            }
            SystematicDef::JesDown => {
                variations.insert(variation("JesDown", "shape"));
            }
            SystematicDef::JerUp => {
                variations.insert(variation("JerUp", "shape"));
            }
            SystematicDef::JerDown => {
                variations.insert(variation("JerDown", "shape"));
            }
            SystematicDef::Weight(systematic) => {
                variations.insert(variation(format!("{}:up", systematic.name), "weight"));
                variations.insert(variation(format!("{}:down", systematic.name), "weight"));
            }
        }
    }
    variations.into_iter().collect()
}

fn block_statements(block: &kir::Block) -> Vec<String> {
    block
        .stmts
        .iter()
        .flat_map(|stmt| match stmt {
            kir::Stmt::Let { value, expr } => {
                vec![format!(
                    "let:{}:{:?}:{}",
                    value.id.0,
                    value.ty,
                    rvalue_name(expr)
                )]
            }
            kir::Stmt::ForEach { axis, item, body } => {
                let mut statements = vec![format!("foreach:{axis:?}:{}", item.id.0)];
                statements.extend(block_statements(body));
                statements
            }
            kir::Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                let mut statements = vec![format!("if:{}", condition.0)];
                statements.extend(block_statements(then_block));
                statements.push("else".to_string());
                statements.extend(block_statements(else_block));
                statements
            }
            kir::Stmt::Fill {
                histogram,
                value,
                weight,
            } => vec![format!(
                "fill:{}:{}:{}",
                histogram.0,
                value.0,
                weight
                    .map(|weight| weight.0.to_string())
                    .unwrap_or_else(|| "none".to_string())
            )],
            kir::Stmt::Require { condition } => vec![format!("require:{}", condition.0)],
            kir::Stmt::Return { values } => vec![format!(
                "return:{}",
                values
                    .iter()
                    .map(|value| format!("{}:{}", value.name, value.value.0))
                    .collect::<Vec<_>>()
                    .join(",")
            )],
        })
        .collect()
}

fn rvalue_name(expr: &kir::Rvalue) -> String {
    match expr {
        kir::Rvalue::Literal(value) => format!("literal:{value:?}"),
        kir::Rvalue::Quantity(quantity) => {
            format!("quantity:{:?}:{:?}", quantity.value, quantity.unit)
        }
        kir::Rvalue::ObjectRef(object) => format!("object_ref:{}", object.0),
        kir::Rvalue::CandidateRef(object) => format!("candidate_ref:{}", object.0),
        kir::Rvalue::Attr {
            object,
            attr,
            branch,
        } => format!(
            "attr:{}:{attr}:{}",
            object.0,
            branch.as_deref().unwrap_or("")
        ),
        kir::Rvalue::DerivedAttr { object, attr } => {
            format!("derived_attr:{}:{attr}", object.0)
        }
        kir::Rvalue::Call { primitive, args } => {
            let args = args
                .iter()
                .map(|arg| arg.0.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!("call:{}({args})", primitive.0)
        }
        kir::Rvalue::Compare { op, lhs, rhs } => format!("compare:{op:?}:{}:{}", lhs.0, rhs.0),
        kir::Rvalue::SelectObjects { object } => format!("select:{}", object.name),
        kir::Rvalue::DeriveObject { object } => format!("derive:{}", object.name),
        kir::Rvalue::Requirement { requirement } => {
            format!(
                "requirement:{}:{:?}:{:?}",
                requirement.lhs, requirement.op, requirement.rhs
            )
        }
        kir::Rvalue::LumiMask { mask } => {
            format!("lumi_mask:{} runs", mask.ranges_by_run().len())
        }
        kir::Rvalue::Output { expr, ty } => format!("output:{expr}:{ty:?}"),
        kir::Rvalue::Histogram { histogram } => format!("histogram:{}", histogram.name),
        kir::Rvalue::HistogramValue { expr, ty } => format!("histogram_value:{expr}:{ty:?}"),
        kir::Rvalue::ScaleFactor { systematic } => format!("scale_factor:{}", systematic.0),
        kir::Rvalue::Weight {
            systematic,
            scale_factor,
        } => format!(
            "weight:{}:{}",
            systematic.0,
            scale_factor
                .map(|value| value.0.to_string())
                .unwrap_or_default()
        ),
    }
}

fn sorted_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}

fn stable_hash(value: &impl serde::Serialize) -> String {
    let bytes = serde_json::to_string(value)
        .expect("certificate canonical JSON should serialize")
        .into_bytes();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

impl Ord for VariationCertificate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name).then(self.kind.cmp(&other.kind))
    }
}

impl PartialOrd for VariationCertificate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
