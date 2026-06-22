//! Kernel IR for typed, executable analysis kernels.
//!
//! KIR is deliberately plain data: a typed SSA-ish block plus enough analysis
//! scheduling metadata for the interpreter to stop deriving semantics directly
//! from the surface plan. The Core-to-KIR lowering below covers the typed
//! expression graph. The current interpreter uses the executable plan lowering
//! while Core still lacks object identity on zero-argument `object` calls.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use nano_core::{BranchSpec, BranchType};

use crate::core::{self, CoreIr, ExprKind, PrimitiveArg, PrimitiveError, PrimitiveRegistry, Type};
use crate::{
    Catalogue, CatalogueBranch, CmpOp, Cut, DerivedObjectDef, DerivedSource, Expr, HistogramDef,
    Quantity, ResolvedPlan, SpecError, SystematicDef, WeightDef,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValueId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrimitiveId(pub &'static str);

#[derive(Debug, Clone, PartialEq)]
pub struct TypedValue {
    pub id: ValueId,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KirProgram {
    pub name: String,
    pub block: Block,
    pub read_branches: Vec<BranchSpec>,
    pub model_outputs: Vec<String>,
    pub lumi_mask: Option<crate::LumiMaskDef>,
    pub objects: Vec<KirObject>,
    pub derived_objects: Vec<KirDerivedObject>,
    pub regions: Vec<KirRegion>,
    pub outputs: Vec<KirOutput>,
    pub histograms: Vec<KirHistogram>,
    pub systematics: Vec<SystematicDef>,
    pub shape_corrections: Vec<KirShapeCorrection>,
    pub scale_factor_corrections: Vec<KirScaleFactorCorrection>,
    pub weight: WeightDef,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        value: TypedValue,
        expr: Rvalue,
    },
    ForEach {
        axis: ForEachAxis,
        item: TypedValue,
        body: Block,
    },
    If {
        condition: ValueId,
        then_block: Block,
        else_block: Block,
    },
    Fill {
        histogram: ValueId,
        value: ValueId,
        weight: Option<ValueId>,
    },
    Require {
        condition: ValueId,
    },
    Return {
        values: Vec<NamedValue>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rvalue {
    Literal(f64),
    Quantity(Quantity),
    ObjectRef(core::ObjectId),
    CandidateRef(core::ObjectId),
    Attr {
        object: core::ObjectId,
        attr: String,
        branch: Option<String>,
    },
    DerivedAttr {
        object: core::ObjectId,
        attr: String,
    },
    Call {
        primitive: PrimitiveId,
        args: Vec<ValueId>,
    },
    Compare {
        op: CmpOp,
        lhs: ValueId,
        rhs: ValueId,
    },
    SelectObjects {
        object: KirObject,
    },
    DeriveObject {
        object: KirDerivedObject,
    },
    Requirement {
        requirement: KirRequirement,
    },
    LumiMask {
        mask: crate::LumiMask,
    },
    Output {
        expr: crate::Expr,
        ty: Type,
    },
    Histogram {
        histogram: KirHistogram,
    },
    HistogramValue {
        expr: crate::Expr,
        ty: Type,
    },
    ScaleFactor {
        systematic: ValueId,
    },
    Weight {
        systematic: ValueId,
        scale_factor: Option<ValueId>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForEachAxis {
    Systematic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NamedValue {
    pub name: String,
    pub value: ValueId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KirObject {
    pub name: String,
    pub source: String,
    pub cuts: Vec<Cut>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KirDerivedObject {
    pub name: String,
    pub def: DerivedObjectDef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KirRegion {
    pub name: String,
    pub requirements: Vec<KirRequirement>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KirRequirement {
    pub lhs: crate::Expr,
    pub op: CmpOp,
    pub rhs: Quantity,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KirOutput {
    pub name: String,
    pub expr: crate::Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KirHistogram {
    pub name: String,
    pub def: HistogramDef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KirShapeCorrection {
    pub name: String,
    pub collection: String,
    pub attr: String,
    pub payload: KirShapeCorrectionPayload,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KirShapeCorrectionPayload {
    Scale {
        up: f64,
        down: f64,
    },
    Jes {
        file: String,
        correction: String,
        inputs: Vec<crate::ScaleFactorInputDef>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct KirScaleFactorCorrection {
    pub name: String,
    pub file: String,
    pub correction: String,
    pub collection: String,
    pub inputs: Vec<crate::ScaleFactorInputDef>,
    pub systematic: Option<crate::ScaleFactorSystematicDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KirError {
    Lower(String),
    UnknownValue(ValueId),
    DuplicateValue(ValueId),
    TypeMismatch {
        value: ValueId,
        expected: Type,
        actual: Type,
    },
    Primitive(PrimitiveError),
    InvalidControl(String),
    Unsupported(String),
}

impl fmt::Display for KirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lower(detail) => f.write_str(detail),
            Self::UnknownValue(value) => write!(f, "KIR uses undefined value {value:?}"),
            Self::DuplicateValue(value) => write!(f, "KIR defines value {value:?} more than once"),
            Self::TypeMismatch {
                value,
                expected,
                actual,
            } => write!(
                f,
                "KIR value {value:?} has type {actual:?}, expected {expected:?}"
            ),
            Self::Primitive(error) => write!(f, "KIR primitive error: {error}"),
            Self::InvalidControl(detail) | Self::Unsupported(detail) => f.write_str(detail),
        }
    }
}

impl Error for KirError {}

impl From<PrimitiveError> for KirError {
    fn from(error: PrimitiveError) -> Self {
        Self::Primitive(error)
    }
}

pub fn lower_to_kir(core: &CoreIr) -> Result<KirProgram, KirError> {
    let mut block = Block::default();
    for expr in &core.exprs {
        let value = TypedValue {
            id: ValueId(expr.id.0),
            ty: expr.ty.clone(),
        };
        let rvalue = match &expr.kind {
            ExprKind::Literal(value) => Rvalue::Literal(*value),
            ExprKind::Quantity(quantity) => Rvalue::Quantity(quantity.clone()),
            ExprKind::EventScalar { branch } => Rvalue::Attr {
                object: core::ObjectId(usize::MAX),
                attr: branch.clone(),
                branch: Some(branch.clone()),
            },
            ExprKind::Attr {
                object,
                attr,
                branch,
            } => Rvalue::Attr {
                object: *object,
                attr: attr.clone(),
                branch: branch.clone(),
            },
            ExprKind::DerivedAttr { object, attr } => Rvalue::DerivedAttr {
                object: *object,
                attr: attr.clone(),
            },
            ExprKind::Call { primitive, args } => {
                let rvalue = match (*primitive, args.as_slice(), &expr.ty) {
                    ("object", [], Type::ObjectSet) => {
                        // Core currently erases the ObjectId for object refs.
                        Rvalue::Call {
                            primitive: PrimitiveId(primitive),
                            args: Vec::new(),
                        }
                    }
                    ("candidate", [], Type::Candidate) => Rvalue::Call {
                        primitive: PrimitiveId(primitive),
                        args: Vec::new(),
                    },
                    _ => Rvalue::Call {
                        primitive: PrimitiveId(primitive),
                        args: args.iter().map(|id| ValueId(id.0)).collect(),
                    },
                };
                rvalue
            }
            ExprKind::Compare { op, lhs, rhs } => Rvalue::Compare {
                op: *op,
                lhs: ValueId(lhs.0),
                rhs: ValueId(rhs.0),
            },
        };
        block.stmts.push(Stmt::Let {
            value,
            expr: rvalue,
        });
    }

    block.stmts.push(Stmt::Return {
        values: core
            .outputs
            .iter()
            .map(|(name, expr)| NamedValue {
                name: name.clone(),
                value: ValueId(expr.0),
            })
            .collect(),
    });

    let program = KirProgram {
        name: core.name.clone(),
        block,
        read_branches: Vec::new(),
        model_outputs: Vec::new(),
        lumi_mask: None,
        objects: core
            .objects
            .iter()
            .filter_map(|object| {
                object.source.as_ref().map(|source| KirObject {
                    name: object.name.clone(),
                    source: source.clone(),
                    cuts: Vec::new(),
                })
            })
            .collect(),
        derived_objects: Vec::new(),
        regions: core
            .regions
            .iter()
            .map(|region| KirRegion {
                name: region.name.clone(),
                requirements: Vec::new(),
            })
            .collect(),
        outputs: core
            .outputs
            .iter()
            .map(|(name, _)| KirOutput {
                name: name.clone(),
                expr: crate::Expr::Literal(0.0),
            })
            .collect(),
        histograms: core
            .histograms
            .iter()
            .map(|(name, _)| KirHistogram {
                name: name.clone(),
                def: HistogramDef {
                    name: name.clone(),
                    expr: crate::Expr::Literal(0.0),
                    bins: 1,
                    range: [0.0, 1.0],
                },
            })
            .collect(),
        systematics: vec![SystematicDef::Nominal],
        shape_corrections: Vec::new(),
        scale_factor_corrections: Vec::new(),
        weight: WeightDef::default(),
    };
    verify(&program)?;
    Ok(program)
}

/// Lower a validated semantic plan into executable KIR.
pub fn lower_plan_to_kir(plan: &ResolvedPlan) -> Result<KirProgram, KirError> {
    let catalogue = catalogue_from_schema(plan.read_branches.specs());
    let core = crate::lower(&plan.spec, &catalogue).map_err(format_spec_errors)?;
    let mut program = lower_to_kir(&core)?;

    program.objects = plan
        .spec
        .objects
        .iter()
        .map(|object| KirObject {
            name: object.name.clone(),
            source: object.source.clone(),
            cuts: object.cuts.clone(),
        })
        .collect();
    program.derived_objects = plan
        .spec
        .derived_objects
        .iter()
        .map(|derived| KirDerivedObject {
            name: derived.name.clone(),
            def: derived.clone(),
        })
        .collect();
    program.regions = plan
        .spec
        .regions
        .iter()
        .map(|region| KirRegion {
            name: region.name.clone(),
            requirements: region
                .require
                .iter()
                .map(|requirement| KirRequirement {
                    lhs: requirement.lhs.clone(),
                    op: requirement.op,
                    rhs: requirement.rhs.clone(),
                })
                .collect(),
        })
        .collect();
    program.outputs = plan
        .spec
        .outputs
        .iter()
        .map(|output| KirOutput {
            name: output.name.clone(),
            expr: output.expr.clone(),
        })
        .collect();
    program.histograms = plan
        .spec
        .histograms
        .iter()
        .map(|histogram| KirHistogram {
            name: histogram.name.clone(),
            def: histogram.clone(),
        })
        .collect();
    program.systematics = plan.spec.systematics.clone();
    program.lumi_mask = plan.spec.lumi_mask.clone();
    program.shape_corrections = plan
        .spec
        .shape_corrections
        .iter()
        .map(|correction| KirShapeCorrection {
            name: correction.name.clone(),
            collection: correction.collection.clone(),
            attr: correction.attr.clone(),
            payload: match &correction.payload {
                crate::ShapeCorrectionPayload::Scale { up, down } => {
                    KirShapeCorrectionPayload::Scale {
                        up: *up,
                        down: *down,
                    }
                }
                crate::ShapeCorrectionPayload::Jes {
                    file,
                    correction,
                    inputs,
                } => KirShapeCorrectionPayload::Jes {
                    file: file.clone(),
                    correction: correction.clone(),
                    inputs: inputs.clone(),
                },
            },
        })
        .collect();
    program.scale_factor_corrections = plan
        .spec
        .scale_factor_corrections
        .iter()
        .map(|correction| KirScaleFactorCorrection {
            name: correction.name.clone(),
            file: correction.file.clone(),
            correction: correction.correction.clone(),
            collection: correction.collection.clone(),
            inputs: correction.inputs.clone(),
            systematic: correction.systematic.clone(),
        })
        .collect();
    program.weight = plan.spec.weight.clone();
    program.read_branches = plan.read_branches.specs().to_vec();
    program.model_outputs = plan
        .spec
        .models
        .iter()
        .map(|model| model.output.clone())
        .collect();
    program.block = executable_block(&program)?;
    verify(&program)?;
    Ok(program)
}

fn executable_block(program: &KirProgram) -> Result<Block, KirError> {
    let mut block = Block::default();
    let mut next_value = 0_usize;

    if let Some(mask) = &program.lumi_mask {
        let ranges = mask.ranges.clone().ok_or_else(|| {
            KirError::Lower(format!(
                "lumi_mask `{}` was not resolved before KIR lowering",
                mask.file
            ))
        })?;
        let condition = ValueId(next_value);
        next_value += 1;
        block.stmts.push(Stmt::Let {
            value: TypedValue {
                id: condition,
                ty: Type::Bool,
            },
            expr: Rvalue::LumiMask { mask: ranges },
        });
        block.stmts.push(Stmt::Require { condition });
    }

    for object in &program.objects {
        let value = TypedValue {
            id: ValueId(next_value),
            ty: Type::ObjectSet,
        };
        next_value += 1;
        block.stmts.push(Stmt::Let {
            value,
            expr: Rvalue::SelectObjects {
                object: object.clone(),
            },
        });
    }

    for object in ordered_derived_objects(program)? {
        let value = TypedValue {
            id: ValueId(next_value),
            ty: Type::Candidate,
        };
        next_value += 1;
        block.stmts.push(Stmt::Let {
            value,
            expr: Rvalue::DeriveObject {
                object: object.clone(),
            },
        });
    }

    for region in &program.regions {
        for requirement in &region.requirements {
            let condition = ValueId(next_value);
            next_value += 1;
            block.stmts.push(Stmt::Let {
                value: TypedValue {
                    id: condition,
                    ty: Type::Bool,
                },
                expr: Rvalue::Requirement {
                    requirement: requirement.clone(),
                },
            });
            block.stmts.push(Stmt::Require { condition });
        }
    }

    let mut returned = Vec::with_capacity(program.outputs.len());
    for output in &program.outputs {
        let value = ValueId(next_value);
        next_value += 1;
        block.stmts.push(Stmt::Let {
            value: TypedValue {
                id: value,
                ty: output_expr_type(&output.expr),
            },
            expr: Rvalue::Output {
                expr: output.expr.clone(),
                ty: output_expr_type(&output.expr),
            },
        });
        returned.push(NamedValue {
            name: output.name.clone(),
            value,
        });
    }

    if has_weight_systematic(program)
        && program.shape_corrections.is_empty()
        && !program.histograms.is_empty()
    {
        let systematic = ValueId(next_value);
        next_value += 1;
        let mut body = Block::default();
        let scale_factor = if program.scale_factor_corrections.is_empty() {
            None
        } else {
            let value = ValueId(next_value);
            next_value += 1;
            body.stmts.push(Stmt::Let {
                value: TypedValue {
                    id: value,
                    ty: Type::Quantity(crate::Dimension::Dimensionless),
                },
                expr: Rvalue::ScaleFactor { systematic },
            });
            Some(value)
        };
        let weight = ValueId(next_value);
        next_value += 1;
        body.stmts.push(Stmt::Let {
            value: TypedValue {
                id: weight,
                ty: Type::Weight,
            },
            expr: Rvalue::Weight {
                systematic,
                scale_factor,
            },
        });
        for histogram in &program.histograms {
            let histogram_value = ValueId(next_value);
            next_value += 1;
            block.stmts.push(Stmt::Let {
                value: TypedValue {
                    id: histogram_value,
                    ty: Type::Histogram,
                },
                expr: Rvalue::Histogram {
                    histogram: histogram.clone(),
                },
            });
            let fill_value = ValueId(next_value);
            next_value += 1;
            body.stmts.push(Stmt::Let {
                value: TypedValue {
                    id: fill_value,
                    ty: output_expr_type(&histogram.def.expr),
                },
                expr: Rvalue::HistogramValue {
                    expr: histogram.def.expr.clone(),
                    ty: output_expr_type(&histogram.def.expr),
                },
            });
            body.stmts.push(Stmt::Fill {
                histogram: histogram_value,
                value: fill_value,
                weight: Some(weight),
            });
        }
        block.stmts.push(Stmt::ForEach {
            axis: ForEachAxis::Systematic,
            item: TypedValue {
                id: systematic,
                ty: Type::Systematic,
            },
            body,
        });
    }
    block.stmts.push(Stmt::Return { values: returned });

    Ok(block)
}

fn ordered_derived_objects(program: &KirProgram) -> Result<Vec<&KirDerivedObject>, KirError> {
    let mut ordered: Vec<&KirDerivedObject> = Vec::with_capacity(program.derived_objects.len());
    let mut pending = program.derived_objects.iter().collect::<Vec<_>>();

    while !pending.is_empty() {
        let Some(index) = pending.iter().position(|object| {
            derived_dependencies(&object.def)
                .into_iter()
                .all(|dependency| {
                    !program
                        .derived_objects
                        .iter()
                        .any(|derived| derived.name == dependency)
                        || ordered.iter().any(|derived| derived.name == dependency)
                })
        }) else {
            return Err(KirError::Lower(
                "derived object dependency cycle or unresolved dependency".to_string(),
            ));
        };
        ordered.push(pending.remove(index));
    }

    Ok(ordered)
}

fn derived_dependencies(object: &DerivedObjectDef) -> Vec<String> {
    match &object.source {
        DerivedSource::Pair(pair) => pair.exclude.clone(),
        DerivedSource::Candidate(candidate) => candidate.items.clone(),
    }
}

fn output_expr_type(expr: &Expr) -> Type {
    match expr {
        Expr::Count(_) | Expr::CountWhere { .. } => Type::Int,
        Expr::EventScalar(_) | Expr::All { .. } | Expr::Any { .. } | Expr::EitherPairPt { .. } => {
            Type::Bool
        }
        Expr::Attr { .. }
        | Expr::Literal(_)
        | Expr::Binary { .. }
        | Expr::Abs(_)
        | Expr::Sqrt(_)
        | Expr::SumAttr { .. }
        | Expr::ClosestMass { .. }
        | Expr::OtherMass { .. }
        | Expr::LeadingAttr { .. }
        | Expr::PairDeltaR
        | Expr::PairLeadingPt
        | Expr::PairSubleadingPt
        | Expr::CandidateMinDeltaR
        | Expr::CandidateLeadingPt
        | Expr::CandidateSubleadingPt => Type::Float,
    }
}

pub fn verify(program: &KirProgram) -> Result<(), KirError> {
    verify_requirements(program)?;
    verify_shape_corrections(program)?;
    verify_scale_factor_corrections(program)?;
    let registry = PrimitiveRegistry::standard();
    let mut values = BTreeMap::new();
    verify_block(&program.block, &registry, &mut values)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequirementExprType {
    Numeric(crate::Dimension),
    Count,
    Bool,
}

fn verify_requirements(program: &KirProgram) -> Result<(), KirError> {
    for region in &program.regions {
        for requirement in &region.requirements {
            let context = format!("region `{}` requirement `{}`", region.name, requirement.lhs);
            let expr_type = verify_requirement_expr(program, &requirement.lhs, &context)?;
            let dimension = match expr_type {
                RequirementExprType::Numeric(dimension) => dimension,
                RequirementExprType::Count | RequirementExprType::Bool => {
                    crate::Dimension::Dimensionless
                }
            };
            verify_requirement_unit(&context, &requirement.lhs, dimension, &requirement.rhs)?;
            if matches!(expr_type, RequirementExprType::Bool) {
                verify_bool_requirement(&context, requirement)?;
            }
        }
    }
    Ok(())
}

fn verify_requirement_expr(
    program: &KirProgram,
    expr: &Expr,
    context: &str,
) -> Result<RequirementExprType, KirError> {
    match expr {
        Expr::EventScalar(branch) => {
            require_event_scalar_branch(program, branch, context)?;
            Ok(RequirementExprType::Bool)
        }
        Expr::Count(object) => {
            require_object(program, object, context)?;
            Ok(RequirementExprType::Count)
        }
        Expr::CountWhere { object, predicate } => {
            require_object(program, object, context)?;
            verify_selected_numeric_expr(program, object, &predicate.lhs, context)?;
            Ok(RequirementExprType::Count)
        }
        Expr::SumAttr { object, attr } | Expr::LeadingAttr { object, attr } => {
            require_object_attr(program, object, attr, context).map(RequirementExprType::Numeric)
        }
        Expr::All { object, predicate } | Expr::Any { object, predicate } => {
            require_object(program, object, context)?;
            verify_selected_numeric_expr(program, object, &predicate.lhs, context)?;
            Ok(RequirementExprType::Bool)
        }
        Expr::EitherPairPt { left, right, .. } => {
            require_object_attr(program, left, "pt", context)?;
            require_object_attr(program, right, "pt", context)?;
            Ok(RequirementExprType::Bool)
        }
        Expr::ClosestMass { left, right, .. } | Expr::OtherMass { left, right, .. } => {
            require_derived_attr(program, left, "mass", context)?;
            require_derived_attr(program, right, "mass", context)?;
            Ok(RequirementExprType::Numeric(crate::Dimension::Momentum))
        }
        Expr::Attr { object, attr } => if program
            .derived_objects
            .iter()
            .any(|derived| derived.name == *object)
        {
            require_derived_attr(program, object, attr, context)
        } else {
            require_object_attr(program, object, attr, context)
        }
        .map(RequirementExprType::Numeric),
        Expr::Literal(_)
        | Expr::Binary { .. }
        | Expr::Abs(_)
        | Expr::Sqrt(_)
        | Expr::PairDeltaR
        | Expr::PairLeadingPt
        | Expr::PairSubleadingPt
        | Expr::CandidateMinDeltaR
        | Expr::CandidateLeadingPt
        | Expr::CandidateSubleadingPt => Err(KirError::Unsupported(format!(
            "{context}: expression `{expr}` is not supported as a region requirement"
        ))),
    }
}

fn verify_selected_numeric_expr(
    program: &KirProgram,
    object_name: &str,
    expr: &Expr,
    context: &str,
) -> Result<crate::Dimension, KirError> {
    match expr {
        Expr::Attr { object, attr } if object == object_name => {
            require_object_attr(program, object, attr, context)
        }
        Expr::Attr { object, .. } => Err(KirError::Unsupported(format!(
            "{context}: collection predicate for `{object_name}` references `{object}`"
        ))),
        Expr::Literal(_) => Ok(crate::Dimension::Dimensionless),
        Expr::Binary { op, lhs, rhs } => {
            let lhs = verify_selected_numeric_expr(program, object_name, lhs, context)?;
            let rhs = verify_selected_numeric_expr(program, object_name, rhs, context)?;
            verify_arithmetic_dimension(*op, lhs, rhs, context, expr)
        }
        Expr::Abs(inner) | Expr::Sqrt(inner) => {
            verify_selected_numeric_expr(program, object_name, inner, context)
        }
        other => Err(KirError::Unsupported(format!(
            "{context}: collection predicate expression `{other}` is not supported"
        ))),
    }
}

fn verify_arithmetic_dimension(
    op: crate::ArithOp,
    lhs: crate::Dimension,
    rhs: crate::Dimension,
    context: &str,
    expr: &Expr,
) -> Result<crate::Dimension, KirError> {
    match op {
        crate::ArithOp::Add | crate::ArithOp::Sub if lhs == rhs => Ok(lhs),
        crate::ArithOp::Add | crate::ArithOp::Sub => Err(KirError::Lower(format!(
            "{context}: `{expr}` cannot add or subtract incompatible dimensions"
        ))),
        crate::ArithOp::Mul if lhs == crate::Dimension::Dimensionless => Ok(rhs),
        crate::ArithOp::Mul if rhs == crate::Dimension::Dimensionless => Ok(lhs),
        crate::ArithOp::Mul => Ok(crate::Dimension::Dimensionless),
        crate::ArithOp::Div if rhs == crate::Dimension::Dimensionless => Ok(lhs),
        crate::ArithOp::Div => Ok(crate::Dimension::Dimensionless),
        crate::ArithOp::Pow if rhs == crate::Dimension::Dimensionless => Ok(lhs),
        crate::ArithOp::Pow => Err(KirError::Lower(format!(
            "{context}: `{expr}` exponent must be dimensionless"
        ))),
    }
}

fn verify_requirement_unit(
    context: &str,
    lhs: &Expr,
    dimension: crate::Dimension,
    rhs: &Quantity,
) -> Result<(), KirError> {
    match (dimension, rhs.unit) {
        (crate::Dimension::Momentum, crate::Unit::GeV)
        | (crate::Dimension::Dimensionless, crate::Unit::Dimensionless) => Ok(()),
        (crate::Dimension::Momentum, crate::Unit::Dimensionless) => Err(KirError::Lower(format!(
            "{context}: `{lhs}` requires unit GeV"
        ))),
        (expected, actual) => Err(KirError::Lower(format!(
            "{context}: `{lhs}` has dimension {expected:?}, but rhs has unit {actual:?}"
        ))),
    }
}

fn verify_bool_requirement(context: &str, requirement: &KirRequirement) -> Result<(), KirError> {
    let valid_rhs = requirement.rhs.value == 0.0 || requirement.rhs.value == 1.0;
    let valid_op = matches!(requirement.op, CmpOp::Eq | CmpOp::Ne);
    if valid_rhs && valid_op {
        Ok(())
    } else {
        Err(KirError::Lower(format!(
            "{context}: boolean predicate `{}` supports only == 1, != 0, == 0, or != 1",
            requirement.lhs
        )))
    }
}

fn require_object<'a>(
    program: &'a KirProgram,
    object: &str,
    context: &str,
) -> Result<&'a KirObject, KirError> {
    program
        .objects
        .iter()
        .find(|candidate| candidate.name == object)
        .ok_or_else(|| KirError::Lower(format!("{context}: unknown object `{object}`")))
}

fn require_event_scalar_branch(
    program: &KirProgram,
    branch: &str,
    context: &str,
) -> Result<(), KirError> {
    let Some(spec) = program
        .read_branches
        .iter()
        .find(|spec| spec.name == branch)
    else {
        return Err(KirError::Lower(format!(
            "{context}: event scalar branch `{branch}` is missing from read schema"
        )));
    };
    if spec.branch_type == BranchType::Bool {
        Ok(())
    } else {
        Err(KirError::Lower(format!(
            "{context}: event scalar branch `{branch}` has type {}, expected bool",
            raw_branch_type(spec.branch_type)
        )))
    }
}

fn require_object_attr(
    program: &KirProgram,
    object: &str,
    attr: &str,
    context: &str,
) -> Result<crate::Dimension, KirError> {
    let object = require_object(program, object, context)?;
    let branch = format!("{}_{}", object.source, attr);
    if program.model_outputs.iter().any(|output| output == &branch) {
        return Ok(crate::Dimension::Dimensionless);
    }
    let branch_type = program
        .read_branches
        .iter()
        .find(|spec| spec.name == branch)
        .map(|spec| spec.branch_type)
        .ok_or_else(|| KirError::Lower(format!("{context}: missing branch `{branch}`")))?;
    if !is_numeric_vector_branch(branch_type) {
        return Err(KirError::Lower(format!(
            "{context}: branch `{branch}` has type {branch_type:?}, expected numeric vector"
        )));
    }
    Ok(attribute_dimension(attr))
}

fn require_derived_attr(
    program: &KirProgram,
    object: &str,
    attr: &str,
    context: &str,
) -> Result<crate::Dimension, KirError> {
    let derived = program
        .derived_objects
        .iter()
        .find(|candidate| candidate.name == object)
        .ok_or_else(|| KirError::Lower(format!("{context}: unknown derived object `{object}`")))?;
    match (&derived.def.source, attr) {
        (DerivedSource::Pair(_), "mass" | "pt") | (DerivedSource::Candidate(_), "mass" | "pt") => {
            Ok(crate::Dimension::Momentum)
        }
        (DerivedSource::Pair(_), "min_delta_r" | "dR" | "dr")
        | (DerivedSource::Candidate(_), "min_delta_r" | "dR" | "dr") => {
            Ok(crate::Dimension::Dimensionless)
        }
        _ => Err(KirError::Unsupported(format!(
            "{context}: derived object `{object}` has no supported attribute `{attr}`"
        ))),
    }
}

fn attribute_dimension(attr: &str) -> crate::Dimension {
    match attr {
        "pt" | "mass" | "energy" | "msoftdrop" | "rawFactor" => crate::Dimension::Momentum,
        value if value.ends_with("Pt") || value.ends_with("Mass") => crate::Dimension::Momentum,
        _ => crate::Dimension::Dimensionless,
    }
}

fn is_numeric_vector_branch(branch_type: BranchType) -> bool {
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

fn verify_shape_corrections(program: &KirProgram) -> Result<(), KirError> {
    for correction in &program.shape_corrections {
        if !program
            .objects
            .iter()
            .any(|object| object.name == correction.collection)
        {
            return Err(KirError::Lower(format!(
                "shape correction `{}` references unknown collection `{}`",
                correction.name, correction.collection
            )));
        }
        if correction.attr != "pt" {
            return Err(KirError::Unsupported(format!(
                "shape correction `{}` scales `{}`; this KIR slice only supports `pt`",
                correction.name, correction.attr
            )));
        }
        match &correction.payload {
            KirShapeCorrectionPayload::Scale { up, down } => {
                if !(up.is_finite() && down.is_finite()) {
                    return Err(KirError::Lower(format!(
                        "shape correction `{}` has non-finite up/down scale factor",
                        correction.name
                    )));
                }
            }
            KirShapeCorrectionPayload::Jes {
                file,
                correction: payload_name,
                inputs,
            } => {
                let set = nano_corrections::CorrectionSet::from_path(file).map_err(|error| {
                    KirError::Lower(format!(
                        "JES correction `{}` failed to load `{}`: {error}",
                        correction.name, file
                    ))
                })?;
                let payload = set.correction(payload_name).map_err(|error| {
                    KirError::Lower(format!(
                        "JES correction `{}` payload lookup failed: {error}",
                        correction.name
                    ))
                })?;
                let declared = inputs
                    .iter()
                    .map(|input| input.name.as_str())
                    .collect::<Vec<_>>();
                let expected = payload
                    .inputs
                    .iter()
                    .map(|input| input.name.as_str())
                    .collect::<Vec<_>>();
                if declared != expected {
                    return Err(KirError::Lower(format!(
                        "JES correction `{}` declared inputs [{}] do not match correctionlib inputs [{}]",
                        correction.name,
                        declared.join(", "),
                        expected.join(", ")
                    )));
                }
            }
        }
    }
    Ok(())
}

fn verify_scale_factor_corrections(program: &KirProgram) -> Result<(), KirError> {
    for correction in &program.scale_factor_corrections {
        if !program
            .objects
            .iter()
            .any(|object| object.name == correction.collection)
        {
            return Err(KirError::Lower(format!(
                "scale-factor correction `{}` references unknown collection `{}`",
                correction.name, correction.collection
            )));
        }
        let set =
            nano_corrections::CorrectionSet::from_path(&correction.file).map_err(|error| {
                KirError::Lower(format!(
                    "scale-factor correction `{}` failed to load `{}`: {error}",
                    correction.name, correction.file
                ))
            })?;
        let payload = set.correction(&correction.correction).map_err(|error| {
            KirError::Lower(format!(
                "scale-factor correction `{}` payload lookup failed: {error}",
                correction.name
            ))
        })?;
        let declared = correction
            .inputs
            .iter()
            .map(|input| input.name.as_str())
            .chain(
                correction
                    .systematic
                    .iter()
                    .map(|systematic| systematic.name.as_str()),
            )
            .collect::<Vec<_>>();
        let expected = payload
            .inputs
            .iter()
            .map(|input| input.name.as_str())
            .collect::<Vec<_>>();
        if declared != expected {
            return Err(KirError::Lower(format!(
                "scale-factor correction `{}` declared inputs [{}] do not match payload inputs [{}]",
                correction.name,
                declared.join(", "),
                expected.join(", ")
            )));
        }
    }
    Ok(())
}

fn verify_block(
    block: &Block,
    registry: &PrimitiveRegistry,
    values: &mut BTreeMap<ValueId, Type>,
) -> Result<(), KirError> {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { value, expr } => {
                if values.contains_key(&value.id) {
                    return Err(KirError::DuplicateValue(value.id));
                }
                let actual = match expr {
                    Rvalue::Attr { .. } | Rvalue::DerivedAttr { .. } => value.ty.clone(),
                    _ => verify_rvalue(expr, registry, values)?,
                };
                if actual != value.ty {
                    return Err(KirError::TypeMismatch {
                        value: value.id,
                        expected: value.ty.clone(),
                        actual,
                    });
                }
                values.insert(value.id, value.ty.clone());
            }
            Stmt::ForEach { axis, item, body } => {
                let expected = match axis {
                    ForEachAxis::Systematic => Type::Systematic,
                };
                if item.ty != expected {
                    return Err(KirError::TypeMismatch {
                        value: item.id,
                        expected,
                        actual: item.ty.clone(),
                    });
                }
                let mut body_values = values.clone();
                if body_values.insert(item.id, item.ty.clone()).is_some() {
                    return Err(KirError::DuplicateValue(item.id));
                }
                verify_block(body, registry, &mut body_values)?;
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
            } => {
                require_type(*condition, Type::Bool, values)?;
                verify_block(then_block, registry, &mut values.clone())?;
                verify_block(else_block, registry, &mut values.clone())?;
            }
            Stmt::Fill {
                histogram,
                value,
                weight,
            } => {
                require_type(*histogram, Type::Histogram, values)?;
                let value_ty = values
                    .get(value)
                    .ok_or(KirError::UnknownValue(*value))?
                    .clone();
                if value_ty.numeric_dimension().is_none() {
                    return Err(KirError::InvalidControl(format!(
                        "KIR fill value {value:?} must be numeric, got {value_ty:?}"
                    )));
                }
                match weight {
                    Some(weight) => require_type(*weight, Type::Weight, values)?,
                    None => {
                        return Err(KirError::Unsupported(
                            "KIR Fill requires a weighted/selected context; typestate lowering will add it in the histogram move".to_string(),
                        ));
                    }
                }
            }
            Stmt::Require { condition } => {
                require_type(*condition, Type::Bool, values)?;
            }
            Stmt::Return { values: returned } => {
                for returned in returned {
                    if !values.contains_key(&returned.value) {
                        return Err(KirError::UnknownValue(returned.value));
                    }
                }
            }
        }
    }
    Ok(())
}

fn verify_rvalue(
    expr: &Rvalue,
    registry: &PrimitiveRegistry,
    values: &BTreeMap<ValueId, Type>,
) -> Result<Type, KirError> {
    match expr {
        Rvalue::Literal(_) => Ok(Type::Quantity(crate::Dimension::Dimensionless)),
        Rvalue::Quantity(quantity) => match quantity.unit {
            crate::Unit::GeV => Ok(Type::Quantity(crate::Dimension::Momentum)),
            crate::Unit::Dimensionless => Ok(Type::Quantity(crate::Dimension::Dimensionless)),
        },
        Rvalue::ObjectRef(_) => Ok(Type::ObjectSet),
        Rvalue::CandidateRef(_) => Ok(Type::Candidate),
        Rvalue::Attr { .. } | Rvalue::DerivedAttr { .. } => unreachable!("handled by Let"),
        Rvalue::Call { primitive, args } => {
            let args = args
                .iter()
                .map(|id| {
                    values
                        .get(id)
                        .cloned()
                        .map(PrimitiveArg::new)
                        .ok_or(KirError::UnknownValue(*id))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(registry.validate_call(primitive.0, &args)?.ty)
        }
        Rvalue::Compare { op, lhs, rhs } => {
            let lhs = values
                .get(lhs)
                .cloned()
                .map(PrimitiveArg::new)
                .ok_or(KirError::UnknownValue(*lhs))?;
            let rhs = values
                .get(rhs)
                .cloned()
                .map(PrimitiveArg::new)
                .ok_or(KirError::UnknownValue(*rhs))?;
            Ok(registry
                .validate_call(core::primitive_name_for_cmp(*op), &[lhs, rhs])?
                .ty)
        }
        Rvalue::SelectObjects { .. } => Ok(Type::ObjectSet),
        Rvalue::DeriveObject { .. } => Ok(Type::Candidate),
        Rvalue::Requirement { .. } => Ok(Type::Bool),
        Rvalue::LumiMask { .. } => Ok(Type::Bool),
        Rvalue::Output { ty, .. } => Ok(ty.clone()),
        Rvalue::Histogram { .. } => Ok(Type::Histogram),
        Rvalue::HistogramValue { ty, .. } => Ok(ty.clone()),
        Rvalue::ScaleFactor { systematic } => {
            require_type(*systematic, Type::Systematic, values)?;
            Ok(Type::Quantity(crate::Dimension::Dimensionless))
        }
        Rvalue::Weight {
            systematic,
            scale_factor,
        } => {
            require_type(*systematic, Type::Systematic, values)?;
            if let Some(scale_factor) = scale_factor {
                require_type(
                    *scale_factor,
                    Type::Quantity(crate::Dimension::Dimensionless),
                    values,
                )?;
            }
            Ok(Type::Weight)
        }
    }
}

fn has_weight_systematic(program: &KirProgram) -> bool {
    program
        .systematics
        .iter()
        .any(|systematic| matches!(systematic, SystematicDef::Weight(_)))
        || program
            .scale_factor_corrections
            .iter()
            .any(|correction| correction.systematic.is_some())
}

fn require_type(
    id: ValueId,
    expected: Type,
    values: &BTreeMap<ValueId, Type>,
) -> Result<(), KirError> {
    let actual = values.get(&id).ok_or(KirError::UnknownValue(id))?;
    if *actual == expected {
        Ok(())
    } else {
        Err(KirError::TypeMismatch {
            value: id,
            expected,
            actual: actual.clone(),
        })
    }
}

fn catalogue_from_schema(specs: &[BranchSpec]) -> Catalogue {
    Catalogue {
        branches: specs
            .iter()
            .map(|spec| {
                (
                    spec.name.clone(),
                    CatalogueBranch {
                        branch_type: Some(spec.branch_type),
                        raw_type: raw_branch_type(spec.branch_type).to_string(),
                    },
                )
            })
            .collect(),
    }
}

fn raw_branch_type(branch_type: BranchType) -> &'static str {
    match branch_type {
        BranchType::Bool => "bool",
        BranchType::I8 => "int8",
        BranchType::U8 => "uint8",
        BranchType::I16 => "int16",
        BranchType::U16 => "uint16",
        BranchType::I32 => "int32",
        BranchType::U32 => "uint32",
        BranchType::I64 => "int64",
        BranchType::U64 => "uint64",
        BranchType::F32 => "float",
        BranchType::VecBool => "vec_bool",
        BranchType::VecI8 => "vec_int8",
        BranchType::VecU8 => "vec_uint8",
        BranchType::VecI16 => "vec_int16",
        BranchType::VecU16 => "vec_uint16",
        BranchType::VecI32 => "vec_int32",
        BranchType::VecU32 => "vec_uint32",
        BranchType::VecI64 => "vec_int64",
        BranchType::VecU64 => "vec_uint64",
        BranchType::VecF32 => "vec_float",
    }
}

fn format_spec_errors(errors: Vec<SpecError>) -> KirError {
    KirError::Lower(
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnalysisSpec, Catalogue};

    const MUON_SPEC: &str = include_str!("../examples/muon.toml");
    const MUON_WEIGHT_SYSTEMATIC_SPEC: &str =
        include_str!("../examples/muon_hist_weight_systematic.toml");
    const MUON_SHAPE_CORRECTION_SPEC: &str =
        include_str!("../examples/muon_hist_shape_correction.toml");
    const MUON_SF_SPEC: &str = include_str!("../examples/muon_sf.toml");
    const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");

    #[test]
    fn lowers_and_verifies_muon_core_to_kir() {
        let spec = AnalysisSpec::from_toml_str(MUON_SPEC).expect("parse muon spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        let core = crate::lower(&spec, &catalogue).expect("lower core");
        let kir = lower_to_kir(&core).expect("lower kir");

        verify(&kir).expect("verify kir");
        assert_eq!(kir.name, "muon_demo");
        assert!(matches!(kir.block.stmts.last(), Some(Stmt::Return { .. })));
    }

    #[test]
    fn lowers_weight_systematic_histogram_to_for_each_fill() {
        let spec =
            AnalysisSpec::from_toml_str(MUON_WEIGHT_SYSTEMATIC_SPEC).expect("parse muon spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        let plan = crate::validate(&spec, &catalogue).expect("validate spec");
        let kir = lower_plan_to_kir(&plan).expect("lower executable kir");

        verify(&kir).expect("verify kir");
        assert!(kir.block.stmts.iter().any(|stmt| matches!(
            stmt,
            Stmt::ForEach {
                axis: ForEachAxis::Systematic,
                body,
                ..
            } if body
                .stmts
                .iter()
                .any(|stmt| matches!(stmt, Stmt::Fill { .. }))
        )));
    }

    #[test]
    fn lowers_shape_correction_metadata_to_kir() {
        let spec =
            AnalysisSpec::from_toml_str(MUON_SHAPE_CORRECTION_SPEC).expect("parse muon spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        let plan = crate::validate(&spec, &catalogue).expect("validate spec");
        let kir = lower_plan_to_kir(&plan).expect("lower executable kir");

        verify(&kir).expect("verify kir");
        assert_eq!(kir.shape_corrections.len(), 1);
        assert_eq!(kir.shape_corrections[0].collection, "good_muon");
        assert_eq!(kir.shape_corrections[0].attr, "pt");
    }

    #[test]
    fn lowers_scale_factor_weight_metadata_and_node_to_kir() {
        let spec = AnalysisSpec::from_toml_str(MUON_SF_SPEC).expect("parse muon SF spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        let plan = crate::validate(&spec, &catalogue).expect("validate spec");
        let kir = lower_plan_to_kir(&plan).expect("lower executable kir");

        verify(&kir).expect("verify kir");
        assert_eq!(kir.scale_factor_corrections.len(), 1);
        assert_eq!(kir.scale_factor_corrections[0].collection, "good_muon");
        assert!(kir.block.stmts.iter().any(|stmt| matches!(
            stmt,
            Stmt::ForEach {
                axis: ForEachAxis::Systematic,
                body,
                ..
            } if body.stmts.iter().any(|stmt| matches!(
                stmt,
                Stmt::Let {
                    expr: Rvalue::ScaleFactor { .. },
                    ..
                }
            )) && body.stmts.iter().any(|stmt| matches!(
                stmt,
                Stmt::Let {
                    expr: Rvalue::Weight {
                        scale_factor: Some(_),
                        ..
                    },
                    ..
                }
            ))
        )));
    }
}
