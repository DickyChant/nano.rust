//! Typed Core IR and primitive registry for nano-spec.
//!
//! This module is the stable internal source of truth introduced before the
//! KIR/emitter rewrite. The current interpreter and Rust emitter still execute
//! the surface spec, but validation lowers through these typed nodes first.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::{ArithOp, CmpOp, Dimension, Quantity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExprId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VariationAxisId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Type {
    Bool,
    Int,
    Float,
    Quantity(Dimension),
    ObjectSet,
    Candidate,
    Event,
    Weight,
    Systematic,
    Tensor,
    Histogram,
}

impl Type {
    pub fn numeric_dimension(&self) -> Option<Dimension> {
        match self {
            Self::Int | Self::Float => Some(Dimension::Dimensionless),
            Self::Quantity(dimension) => Some(*dimension),
            Self::Bool
            | Self::ObjectSet
            | Self::Candidate
            | Self::Event
            | Self::Weight
            | Self::Systematic
            | Self::Tensor
            | Self::Histogram => None,
        }
    }

    fn is_numeric(&self) -> bool {
        self.numeric_dimension().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NumericCompatMode {
    RootDf103,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Effect {
    ReadsBranch(String),
    RequiresModel(ModelId),
    ProducesScore(String),
    ShapeDependsOn(VariationAxisId),
    RequiresCompat(NumericCompatMode),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExprNode {
    pub id: ExprId,
    pub kind: ExprKind,
    pub ty: Type,
    pub effects: BTreeSet<Effect>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Literal(f64),
    Quantity(Quantity),
    Attr {
        object: ObjectId,
        attr: String,
        branch: Option<String>,
    },
    DerivedAttr {
        object: ObjectId,
        attr: String,
    },
    Call {
        primitive: &'static str,
        args: Vec<ExprId>,
    },
    Compare {
        op: CmpOp,
        lhs: ExprId,
        rhs: ExprId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectNode {
    pub id: ObjectId,
    pub name: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionNode {
    pub id: RegionId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelNode {
    pub id: ModelId,
    pub name: String,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CoreIr {
    pub name: String,
    pub objects: Vec<ObjectNode>,
    pub regions: Vec<RegionNode>,
    pub models: Vec<ModelNode>,
    pub exprs: Vec<ExprNode>,
    pub outputs: Vec<(String, ExprId)>,
    pub histograms: Vec<(String, ExprId)>,
    pub effects: BTreeSet<Effect>,
    effect_order: Vec<Effect>,
}

impl CoreIr {
    pub fn expr(&self, id: ExprId) -> &ExprNode {
        &self.exprs[id.0]
    }

    pub fn read_branches(&self) -> BTreeSet<&str> {
        self.effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::ReadsBranch(branch) => Some(branch.as_str()),
                Effect::RequiresModel(_)
                | Effect::ProducesScore(_)
                | Effect::ShapeDependsOn(_)
                | Effect::RequiresCompat(_) => None,
            })
            .collect()
    }

    pub fn read_branches_ordered(&self) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        self.effect_order
            .iter()
            .filter_map(|effect| match effect {
                Effect::ReadsBranch(branch) if seen.insert(branch.as_str()) => {
                    Some(branch.as_str())
                }
                Effect::ReadsBranch(_)
                | Effect::RequiresModel(_)
                | Effect::ProducesScore(_)
                | Effect::ShapeDependsOn(_)
                | Effect::RequiresCompat(_) => None,
            })
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct CoreBuilder {
    name: String,
    objects: Vec<ObjectNode>,
    regions: Vec<RegionNode>,
    models: Vec<ModelNode>,
    exprs: Vec<ExprNode>,
    outputs: Vec<(String, ExprId)>,
    histograms: Vec<(String, ExprId)>,
    effects: BTreeSet<Effect>,
    effect_order: Vec<Effect>,
}

impl CoreBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn add_object(&mut self, name: impl Into<String>, source: Option<String>) -> ObjectId {
        let id = ObjectId(self.objects.len());
        self.objects.push(ObjectNode {
            id,
            name: name.into(),
            source,
        });
        id
    }

    pub fn add_region(&mut self, name: impl Into<String>) -> RegionId {
        let id = RegionId(self.regions.len());
        self.regions.push(RegionNode {
            id,
            name: name.into(),
        });
        id
    }

    pub fn add_model(&mut self, name: impl Into<String>, output: impl Into<String>) -> ModelId {
        let id = ModelId(self.models.len());
        self.models.push(ModelNode {
            id,
            name: name.into(),
            output: output.into(),
        });
        id
    }

    pub fn add_expr(&mut self, kind: ExprKind, ty: Type, effects: BTreeSet<Effect>) -> ExprId {
        let id = ExprId(self.exprs.len());
        for effect in &effects {
            self.add_effect(effect.clone());
        }
        self.exprs.push(ExprNode {
            id,
            kind,
            ty,
            effects,
        });
        id
    }

    pub fn add_effect(&mut self, effect: Effect) {
        if self.effects.insert(effect.clone()) {
            self.effect_order.push(effect);
        }
    }

    pub fn add_output(&mut self, name: impl Into<String>, expr: ExprId) {
        self.outputs.push((name.into(), expr));
    }

    pub fn add_histogram(&mut self, name: impl Into<String>, expr: ExprId) {
        self.histograms.push((name.into(), expr));
    }

    pub fn expr(&self, id: ExprId) -> &ExprNode {
        &self.exprs[id.0]
    }

    pub fn finish(self) -> CoreIr {
        CoreIr {
            name: self.name,
            objects: self.objects,
            regions: self.regions,
            models: self.models,
            exprs: self.exprs,
            outputs: self.outputs,
            histograms: self.histograms,
            effects: self.effects,
            effect_order: self.effect_order,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimitiveSpec {
    pub name: &'static str,
    pub signature: Signature,
    pub dimension_rule: DimensionRule,
    pub effect_rule: EffectRule,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signature {
    Exact(Vec<TypeConstraint>),
    Variadic { min: usize, each: TypeConstraint },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeConstraint {
    Any,
    Numeric,
    Exact(Type),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DimensionRule {
    Fixed(Type),
    UnaryNumeric,
    Argument(usize),
    Arithmetic(ArithOp),
    Compare,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectRule {
    Pure,
    UnionArgs,
    Add(Vec<Effect>),
    UnionArgsAndAdd(Vec<Effect>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimitiveCall {
    pub ty: Type,
    pub effects: BTreeSet<Effect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimitiveArg {
    pub ty: Type,
    pub effects: BTreeSet<Effect>,
}

impl PrimitiveArg {
    pub fn new(ty: Type) -> Self {
        Self {
            ty,
            effects: BTreeSet::new(),
        }
    }

    pub fn with_effects(ty: Type, effects: BTreeSet<Effect>) -> Self {
        Self { ty, effects }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimitiveError {
    UnknownPrimitive {
        name: String,
    },
    WrongArity {
        name: &'static str,
        expected: String,
        actual: usize,
    },
    TypeMismatch {
        name: &'static str,
        arg: usize,
        expected: String,
        actual: Type,
    },
    DimensionMismatch {
        name: &'static str,
        detail: String,
    },
}

impl fmt::Display for PrimitiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPrimitive { name } => write!(f, "unknown primitive `{name}`"),
            Self::WrongArity {
                name,
                expected,
                actual,
            } => write!(
                f,
                "primitive `{name}` expected {expected} arguments, got {actual}"
            ),
            Self::TypeMismatch {
                name,
                arg,
                expected,
                actual,
            } => write!(
                f,
                "primitive `{name}` argument {} expected {expected}, got {actual:?}",
                arg + 1
            ),
            Self::DimensionMismatch { name, detail } => {
                write!(f, "primitive `{name}` dimension mismatch: {detail}")
            }
        }
    }
}

impl Error for PrimitiveError {}

#[derive(Debug, Clone)]
pub struct PrimitiveRegistry {
    primitives: BTreeMap<&'static str, PrimitiveSpec>,
}

impl PrimitiveRegistry {
    pub fn standard() -> Self {
        let mut registry = Self {
            primitives: BTreeMap::new(),
        };
        registry.register_standard();
        registry
    }

    pub fn get(&self, name: &str) -> Option<&PrimitiveSpec> {
        self.primitives.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.primitives.contains_key(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.primitives.keys().copied()
    }

    pub fn validate_call(
        &self,
        name: &str,
        args: &[PrimitiveArg],
    ) -> Result<PrimitiveCall, PrimitiveError> {
        let Some(spec) = self.get(name) else {
            return Err(PrimitiveError::UnknownPrimitive {
                name: name.to_string(),
            });
        };
        spec.validate(args)
    }

    fn register(&mut self, spec: PrimitiveSpec) {
        self.primitives.insert(spec.name, spec);
    }

    fn register_standard(&mut self) {
        for spec in standard_primitives() {
            self.register(spec);
        }
    }
}

impl PrimitiveSpec {
    fn validate(&self, args: &[PrimitiveArg]) -> Result<PrimitiveCall, PrimitiveError> {
        self.signature.validate(self.name, args)?;
        let ty = self.dimension_rule.apply(self.name, args)?;
        let effects = self.effect_rule.apply(args);
        Ok(PrimitiveCall { ty, effects })
    }
}

impl Signature {
    fn validate(&self, name: &'static str, args: &[PrimitiveArg]) -> Result<(), PrimitiveError> {
        match self {
            Self::Exact(expected) => {
                if expected.len() != args.len() {
                    return Err(PrimitiveError::WrongArity {
                        name,
                        expected: expected.len().to_string(),
                        actual: args.len(),
                    });
                }
                for (index, (constraint, arg)) in expected.iter().zip(args).enumerate() {
                    constraint.validate(name, index, &arg.ty)?;
                }
                Ok(())
            }
            Self::Variadic { min, each } => {
                if args.len() < *min {
                    return Err(PrimitiveError::WrongArity {
                        name,
                        expected: format!("at least {min}"),
                        actual: args.len(),
                    });
                }
                for (index, arg) in args.iter().enumerate() {
                    each.validate(name, index, &arg.ty)?;
                }
                Ok(())
            }
        }
    }
}

impl TypeConstraint {
    fn validate(
        &self,
        name: &'static str,
        index: usize,
        actual: &Type,
    ) -> Result<(), PrimitiveError> {
        match self {
            Self::Any => Ok(()),
            Self::Numeric if actual.is_numeric() => Ok(()),
            Self::Numeric => Err(PrimitiveError::TypeMismatch {
                name,
                arg: index,
                expected: "numeric".to_string(),
                actual: actual.clone(),
            }),
            Self::Exact(expected) if expected == actual => Ok(()),
            Self::Exact(expected) => Err(PrimitiveError::TypeMismatch {
                name,
                arg: index,
                expected: format!("{expected:?}"),
                actual: actual.clone(),
            }),
        }
    }
}

impl DimensionRule {
    fn apply(&self, name: &'static str, args: &[PrimitiveArg]) -> Result<Type, PrimitiveError> {
        match self {
            Self::Fixed(ty) => Ok(ty.clone()),
            Self::UnaryNumeric => Ok(args[0].ty.clone()),
            Self::Argument(index) => Ok(args[*index].ty.clone()),
            Self::Arithmetic(op) => {
                let lhs = args[0]
                    .ty
                    .numeric_dimension()
                    .expect("signature requires numeric lhs");
                let rhs = args[1]
                    .ty
                    .numeric_dimension()
                    .expect("signature requires numeric rhs");
                arithmetic_type(*op, lhs, rhs).ok_or_else(|| PrimitiveError::DimensionMismatch {
                    name,
                    detail: format!("{op:?} cannot combine {lhs:?} with {rhs:?}"),
                })
            }
            Self::Compare => {
                if args[0].ty == Type::Bool && args[1].ty == Type::Bool {
                    return Ok(Type::Bool);
                }
                if args[0].ty == Type::Bool
                    && args[1].ty.numeric_dimension() == Some(Dimension::Dimensionless)
                {
                    return Ok(Type::Bool);
                }
                if args[1].ty == Type::Bool
                    && args[0].ty.numeric_dimension() == Some(Dimension::Dimensionless)
                {
                    return Ok(Type::Bool);
                }
                let lhs =
                    args[0]
                        .ty
                        .numeric_dimension()
                        .ok_or_else(|| PrimitiveError::TypeMismatch {
                            name,
                            arg: 0,
                            expected: "numeric or bool".to_string(),
                            actual: args[0].ty.clone(),
                        })?;
                let rhs =
                    args[1]
                        .ty
                        .numeric_dimension()
                        .ok_or_else(|| PrimitiveError::TypeMismatch {
                            name,
                            arg: 1,
                            expected: "numeric or bool".to_string(),
                            actual: args[1].ty.clone(),
                        })?;
                if lhs == rhs {
                    Ok(Type::Bool)
                } else {
                    Err(PrimitiveError::DimensionMismatch {
                        name,
                        detail: format!("cannot compare {lhs:?} with {rhs:?}"),
                    })
                }
            }
        }
    }
}

impl EffectRule {
    fn apply(&self, args: &[PrimitiveArg]) -> BTreeSet<Effect> {
        let mut effects = BTreeSet::new();
        match self {
            Self::Pure => {}
            Self::UnionArgs => {
                for arg in args {
                    effects.extend(arg.effects.iter().cloned());
                }
            }
            Self::Add(extra) => {
                effects.extend(extra.iter().cloned());
            }
            Self::UnionArgsAndAdd(extra) => {
                for arg in args {
                    effects.extend(arg.effects.iter().cloned());
                }
                effects.extend(extra.iter().cloned());
            }
        }
        effects
    }
}

fn arithmetic_type(op: ArithOp, lhs: Dimension, rhs: Dimension) -> Option<Type> {
    match op {
        ArithOp::Add | ArithOp::Sub if lhs == rhs => Some(Type::Quantity(lhs)),
        ArithOp::Add | ArithOp::Sub => None,
        ArithOp::Mul if lhs == Dimension::Dimensionless => Some(Type::Quantity(rhs)),
        ArithOp::Mul if rhs == Dimension::Dimensionless => Some(Type::Quantity(lhs)),
        ArithOp::Mul => Some(Type::Quantity(Dimension::Dimensionless)),
        ArithOp::Div if rhs == Dimension::Dimensionless => Some(Type::Quantity(lhs)),
        ArithOp::Div => Some(Type::Quantity(Dimension::Dimensionless)),
        ArithOp::Pow if rhs == Dimension::Dimensionless => Some(Type::Quantity(lhs)),
        ArithOp::Pow => None,
    }
}

fn primitive(
    name: &'static str,
    signature: Signature,
    dimension_rule: DimensionRule,
    effect_rule: EffectRule,
) -> PrimitiveSpec {
    PrimitiveSpec {
        name,
        signature,
        dimension_rule,
        effect_rule,
    }
}

fn exact(args: Vec<TypeConstraint>) -> Signature {
    Signature::Exact(args)
}

fn numeric() -> TypeConstraint {
    TypeConstraint::Numeric
}

fn object_set() -> TypeConstraint {
    TypeConstraint::Exact(Type::ObjectSet)
}

fn candidate() -> TypeConstraint {
    TypeConstraint::Exact(Type::Candidate)
}

fn quantity(dimension: Dimension) -> Type {
    Type::Quantity(dimension)
}

fn standard_primitives() -> Vec<PrimitiveSpec> {
    vec![
        primitive(
            "object",
            exact(vec![]),
            DimensionRule::Fixed(Type::ObjectSet),
            EffectRule::Pure,
        ),
        primitive(
            "candidate",
            exact(vec![]),
            DimensionRule::Fixed(Type::Candidate),
            EffectRule::Pure,
        ),
        primitive(
            "attr",
            exact(vec![object_set()]),
            DimensionRule::UnaryNumeric,
            EffectRule::UnionArgs,
        ),
        primitive(
            "derived_attr",
            exact(vec![candidate()]),
            DimensionRule::UnaryNumeric,
            EffectRule::UnionArgs,
        ),
        primitive(
            "literal",
            exact(vec![]),
            DimensionRule::Fixed(quantity(Dimension::Dimensionless)),
            EffectRule::Pure,
        ),
        primitive(
            "count",
            exact(vec![object_set()]),
            DimensionRule::Fixed(Type::Int),
            EffectRule::UnionArgs,
        ),
        primitive(
            "count_where",
            exact(vec![object_set(), TypeConstraint::Exact(Type::Bool)]),
            DimensionRule::Fixed(Type::Int),
            EffectRule::UnionArgs,
        ),
        primitive(
            "sum",
            exact(vec![object_set(), numeric()]),
            DimensionRule::Argument(1),
            EffectRule::UnionArgs,
        ),
        primitive(
            "all",
            exact(vec![object_set(), TypeConstraint::Exact(Type::Bool)]),
            DimensionRule::Fixed(Type::Bool),
            EffectRule::UnionArgs,
        ),
        primitive(
            "any",
            exact(vec![object_set(), TypeConstraint::Exact(Type::Bool)]),
            DimensionRule::Fixed(Type::Bool),
            EffectRule::UnionArgs,
        ),
        primitive(
            "abs",
            exact(vec![numeric()]),
            DimensionRule::UnaryNumeric,
            EffectRule::UnionArgs,
        ),
        primitive(
            "sqrt",
            exact(vec![numeric()]),
            DimensionRule::UnaryNumeric,
            EffectRule::UnionArgs,
        ),
        primitive(
            "add",
            exact(vec![numeric(), numeric()]),
            DimensionRule::Arithmetic(ArithOp::Add),
            EffectRule::UnionArgs,
        ),
        primitive(
            "sub",
            exact(vec![numeric(), numeric()]),
            DimensionRule::Arithmetic(ArithOp::Sub),
            EffectRule::UnionArgs,
        ),
        primitive(
            "mul",
            exact(vec![numeric(), numeric()]),
            DimensionRule::Arithmetic(ArithOp::Mul),
            EffectRule::UnionArgs,
        ),
        primitive(
            "div",
            exact(vec![numeric(), numeric()]),
            DimensionRule::Arithmetic(ArithOp::Div),
            EffectRule::UnionArgs,
        ),
        primitive(
            "pow",
            exact(vec![numeric(), numeric()]),
            DimensionRule::Arithmetic(ArithOp::Pow),
            EffectRule::UnionArgs,
        ),
        primitive(
            "gt",
            exact(vec![TypeConstraint::Any, TypeConstraint::Any]),
            DimensionRule::Compare,
            EffectRule::UnionArgs,
        ),
        primitive(
            "ge",
            exact(vec![TypeConstraint::Any, TypeConstraint::Any]),
            DimensionRule::Compare,
            EffectRule::UnionArgs,
        ),
        primitive(
            "lt",
            exact(vec![TypeConstraint::Any, TypeConstraint::Any]),
            DimensionRule::Compare,
            EffectRule::UnionArgs,
        ),
        primitive(
            "le",
            exact(vec![TypeConstraint::Any, TypeConstraint::Any]),
            DimensionRule::Compare,
            EffectRule::UnionArgs,
        ),
        primitive(
            "eq",
            exact(vec![TypeConstraint::Any, TypeConstraint::Any]),
            DimensionRule::Compare,
            EffectRule::UnionArgs,
        ),
        primitive(
            "ne",
            exact(vec![TypeConstraint::Any, TypeConstraint::Any]),
            DimensionRule::Compare,
            EffectRule::UnionArgs,
        ),
        primitive(
            "leading_pt",
            exact(vec![object_set()]),
            DimensionRule::Fixed(quantity(Dimension::Momentum)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "leading_attr",
            exact(vec![object_set(), numeric()]),
            DimensionRule::Argument(1),
            EffectRule::UnionArgs,
        ),
        primitive(
            "pair",
            exact(vec![object_set()]),
            DimensionRule::Fixed(Type::Candidate),
            EffectRule::UnionArgs,
        ),
        primitive(
            "nearest_mass",
            exact(vec![
                object_set(),
                TypeConstraint::Exact(quantity(Dimension::Momentum)),
            ]),
            DimensionRule::Fixed(Type::Candidate),
            EffectRule::UnionArgs,
        ),
        primitive(
            "nearest_mass_truncated",
            exact(vec![
                object_set(),
                TypeConstraint::Exact(quantity(Dimension::Momentum)),
            ]),
            DimensionRule::Fixed(Type::Candidate),
            EffectRule::UnionArgsAndAdd(vec![Effect::RequiresCompat(NumericCompatMode::RootDf103)]),
        ),
        primitive(
            "closest_mass",
            exact(vec![
                candidate(),
                candidate(),
                TypeConstraint::Exact(quantity(Dimension::Momentum)),
            ]),
            DimensionRule::Fixed(quantity(Dimension::Momentum)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "other_mass",
            exact(vec![
                candidate(),
                candidate(),
                TypeConstraint::Exact(quantity(Dimension::Momentum)),
            ]),
            DimensionRule::Fixed(quantity(Dimension::Momentum)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "combine",
            Signature::Variadic {
                min: 1,
                each: TypeConstraint::Any,
            },
            DimensionRule::Fixed(Type::Candidate),
            EffectRule::UnionArgs,
        ),
        primitive(
            "invariant_mass",
            exact(vec![candidate()]),
            DimensionRule::Fixed(quantity(Dimension::Momentum)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "min_delta_r",
            exact(vec![candidate()]),
            DimensionRule::Fixed(quantity(Dimension::Dimensionless)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "pair_delta_r",
            exact(vec![candidate()]),
            DimensionRule::Fixed(quantity(Dimension::Dimensionless)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "pair_leading_pt",
            exact(vec![candidate()]),
            DimensionRule::Fixed(quantity(Dimension::Momentum)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "pair_subleading_pt",
            exact(vec![candidate()]),
            DimensionRule::Fixed(quantity(Dimension::Momentum)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "candidate_leading_pt",
            exact(vec![candidate()]),
            DimensionRule::Fixed(quantity(Dimension::Momentum)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "candidate_subleading_pt",
            exact(vec![candidate()]),
            DimensionRule::Fixed(quantity(Dimension::Momentum)),
            EffectRule::UnionArgs,
        ),
        primitive(
            "either_pair_pt",
            exact(vec![
                object_set(),
                object_set(),
                TypeConstraint::Exact(quantity(Dimension::Momentum)),
                TypeConstraint::Exact(quantity(Dimension::Momentum)),
            ]),
            DimensionRule::Fixed(Type::Bool),
            EffectRule::UnionArgs,
        ),
        primitive(
            "exclude",
            exact(vec![object_set(), candidate()]),
            DimensionRule::Fixed(Type::ObjectSet),
            EffectRule::UnionArgs,
        ),
        primitive(
            "model",
            exact(vec![TypeConstraint::Exact(Type::Tensor)]),
            DimensionRule::Fixed(Type::Tensor),
            EffectRule::UnionArgs,
        ),
        primitive(
            "infer",
            exact(vec![TypeConstraint::Exact(Type::Tensor)]),
            DimensionRule::Fixed(quantity(Dimension::Dimensionless)),
            EffectRule::UnionArgs,
        ),
    ]
}

fn primitive_name_for_arith(op: ArithOp) -> &'static str {
    match op {
        ArithOp::Add => "add",
        ArithOp::Sub => "sub",
        ArithOp::Mul => "mul",
        ArithOp::Div => "div",
        ArithOp::Pow => "pow",
    }
}

pub fn primitive_name_for_cmp(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Gt => "gt",
        CmpOp::Ge => "ge",
        CmpOp::Lt => "lt",
        CmpOp::Le => "le",
        CmpOp::Eq => "eq",
        CmpOp::Ne => "ne",
    }
}

pub fn primitive_name_for_arithmetic(op: ArithOp) -> &'static str {
    primitive_name_for_arith(op)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_rejects_unknown_primitive() {
        let registry = PrimitiveRegistry::standard();
        let error = registry
            .validate_call("ghost", &[])
            .expect_err("unknown primitive should fail");

        assert_eq!(
            error,
            PrimitiveError::UnknownPrimitive {
                name: "ghost".to_string(),
            }
        );
    }

    #[test]
    fn registry_rejects_wrong_arity() {
        let registry = PrimitiveRegistry::standard();
        let error = registry
            .validate_call("count", &[])
            .expect_err("count requires one object set");

        assert!(matches!(
            error,
            PrimitiveError::WrongArity {
                name: "count",
                actual: 0,
                ..
            }
        ));
    }

    #[test]
    fn registry_rejects_dimension_mismatch() {
        let registry = PrimitiveRegistry::standard();
        let args = [
            PrimitiveArg::new(Type::Quantity(Dimension::Momentum)),
            PrimitiveArg::new(Type::Quantity(Dimension::Dimensionless)),
        ];
        let error = registry
            .validate_call("add", &args)
            .expect_err("add requires compatible dimensions");

        assert!(matches!(
            error,
            PrimitiveError::DimensionMismatch { name: "add", .. }
        ));
    }
}
