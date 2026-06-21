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

use crate::core::{
    self, CoreIr, ExprKind, PrimitiveArg, PrimitiveError, PrimitiveRegistry, Type,
};
use crate::{
    Catalogue, CatalogueBranch, CmpOp, Cut, DerivedObjectDef, DerivedSource, Expr,
    HistogramDef, Quantity, ResolvedPlan, SpecError,
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
    pub objects: Vec<KirObject>,
    pub derived_objects: Vec<KirDerivedObject>,
    pub regions: Vec<KirRegion>,
    pub outputs: Vec<KirOutput>,
    pub histograms: Vec<KirHistogram>,
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
        collection: ValueId,
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
    Output {
        expr: crate::Expr,
        ty: Type,
    },
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
    };
    verify(&program)?;
    Ok(program)
}

pub(crate) fn lower_plan_to_kir(plan: &ResolvedPlan) -> Result<KirProgram, KirError> {
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
    program.read_branches = plan.read_branches.specs().to_vec();
    program.block = executable_block(&program)?;
    verify(&program)?;
    Ok(program)
}

fn executable_block(program: &KirProgram) -> Result<Block, KirError> {
    let mut block = Block::default();
    let mut next_value = 0_usize;

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
    block.stmts.push(Stmt::Return { values: returned });

    Ok(block)
}

fn ordered_derived_objects(program: &KirProgram) -> Result<Vec<&KirDerivedObject>, KirError> {
    let mut ordered: Vec<&KirDerivedObject> = Vec::with_capacity(program.derived_objects.len());
    let mut pending = program.derived_objects.iter().collect::<Vec<_>>();

    while !pending.is_empty() {
        let Some(index) = pending.iter().position(|object| {
            derived_dependencies(&object.def).into_iter().all(|dependency| {
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
        Expr::All { .. } | Expr::Any { .. } | Expr::EitherPairPt { .. } => Type::Bool,
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
    let registry = PrimitiveRegistry::standard();
    let mut values = BTreeMap::new();
    verify_block(&program.block, &registry, &mut values)
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
            Stmt::ForEach {
                collection,
                item,
                body,
            } => {
                require_type(*collection, Type::ObjectSet, values)?;
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
        Rvalue::Output { ty, .. } => Ok(ty.clone()),
    }
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
}
