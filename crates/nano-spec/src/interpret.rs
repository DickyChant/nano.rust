//! Runtime interpreter for the validated semantic IR.
//!
//! This is the dynamic counterpart to [`crate::codegen`]: it executes the same
//! object cuts, region requirements, and output expressions directly over an
//! event instead of requiring a compiled producer.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt;

use nano_analysis::{EventWeight, HistSet1D};
use nano_core::{BranchType, Event, ObjectView};
use nano_inference::{mock_scores, InferRequest, Tensor, TensorData};

use crate::kir::{
    Block, ForEachAxis, KirObject, KirProgram, KirShapeCorrection, Rvalue, Stmt, ValueId,
};
use crate::{
    ArithOp, CmpOp, Cut, DerivedObjectDef, DerivedSource, Expr, ModelDef, ModelProviderKind,
    ObjectCandidateDef, ObjectPairDef, PairConstraint, PairSelection, ResolvedPlan, SystematicDef,
};

/// One typed output cell produced by the interpreter.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    F64(f64),
    I64(i64),
    U32(u32),
    Bool(bool),
}

/// A selected event row, preserving the output declaration order from the spec.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct OutputRow {
    pub values: Vec<(String, Value)>,
}

impl OutputRow {
    pub fn new(values: Vec<(String, Value)>) -> Self {
        Self { values }
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        self.values
            .iter()
            .find_map(|(field, value)| (field == name).then_some(*value))
    }
}

/// One selected channel row produced by a multi-channel union spec.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ChannelOutputRow {
    pub channel: String,
    pub row: OutputRow,
}

/// Interpreter-owned histogram outputs keyed by histogram name.
#[derive(Debug, Clone, PartialEq)]
pub struct InterpretedHistograms {
    histograms: BTreeMap<String, HistSet1D<String>>,
}

impl InterpretedHistograms {
    pub fn new(plan: &ResolvedPlan) -> Self {
        let histograms = plan
            .spec
            .histograms
            .iter()
            .map(|histogram| {
                (
                    histogram.name.clone(),
                    HistSet1D::new(
                        interpreted_systematic_variants(plan),
                        histogram.bins,
                        histogram.range[0],
                        histogram.range[1],
                    ),
                )
            })
            .collect();
        Self { histograms }
    }

    pub fn get(&self, name: &str) -> Option<&HistSet1D<String>> {
        self.histograms.get(name)
    }
}

/// Errors reported while interpreting a validated plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpretError {
    Unsupported(String),
    Event(String),
    InvalidExpression(String),
    MissingObject(String),
    MissingBranch(String),
    TypeMismatch {
        branch: String,
        branch_type: BranchType,
        expected: &'static str,
    },
    NumericConversion(String),
}

impl fmt::Display for InterpretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(detail) => f.write_str(detail),
            Self::Event(detail) => f.write_str(detail),
            Self::InvalidExpression(detail) => f.write_str(detail),
            Self::MissingObject(object) => write!(f, "object `{object}` is not defined"),
            Self::MissingBranch(branch) => write!(f, "branch `{branch}` is missing from the plan"),
            Self::TypeMismatch {
                branch,
                branch_type,
                expected,
            } => write!(
                f,
                "branch `{branch}` has type {branch_type:?}, expected {expected}"
            ),
            Self::NumericConversion(detail) => f.write_str(detail),
        }
    }
}

impl Error for InterpretError {}

impl From<nano_core::NanoError> for InterpretError {
    fn from(error: nano_core::NanoError) -> Self {
        Self::Event(error.to_string())
    }
}

impl From<crate::kir::KirError> for InterpretError {
    fn from(error: crate::kir::KirError) -> Self {
        Self::InvalidExpression(format!("KIR verification failed: {error}"))
    }
}

type Result<T> = std::result::Result<T, InterpretError>;
type SelectedObjects = HashMap<String, Vec<SelectedObject>>;
type DerivedObjects = HashMap<String, Option<DerivedObject>>;
type ModelOutputs = HashMap<String, Vec<f32>>;
type RuntimeValues = HashMap<ValueId, RuntimeValue>;

#[derive(Debug, Clone, PartialEq)]
struct SelectedObject {
    source_index: usize,
    leading_values: HashMap<String, NumericValue>,
}

#[derive(Debug, Clone, PartialEq)]
struct DerivedObject {
    mass: f64,
    pt: f64,
    min_delta_r: f64,
    energy: f64,
    px: f64,
    py: f64,
    pz: f64,
    constituents: Vec<Constituent>,
}

#[derive(Debug, Clone, PartialEq)]
struct Constituent {
    object: String,
    index: usize,
    pt: NumericValue,
    eta: NumericValue,
    phi: NumericValue,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum NumericValue {
    F64(f64),
    I64(i64),
    U64(u64),
}

#[derive(Debug, Clone, PartialEq)]
enum RuntimeValue {
    ObjectSet,
    Candidate,
    Bool(bool),
    Output(Option<Value>),
    Histogram(String),
    Systematic(String),
    Weight(EventWeight),
    Numeric(f64),
}

#[derive(Debug, Clone, PartialEq)]
enum BlockOutcome {
    Continue,
    Return(Option<OutputRow>),
}

impl NumericValue {
    fn as_f64(self) -> f64 {
        match self {
            Self::F64(value) => value,
            Self::I64(value) => value as f64,
            Self::U64(value) => value as f64,
        }
    }

    fn abs(self) -> Self {
        match self {
            Self::F64(value) => Self::F64(value.abs()),
            Self::I64(value) => Self::I64(value.abs()),
            Self::U64(value) => Self::U64(value),
        }
    }
}

/// Interpret one event with a validated semantic plan.
///
/// `Ok(None)` means the event failed a region requirement or a required
/// `leading(...)` output had no selected object. Only `provider = "mock"`
/// model specs are interpreted; real inference providers stay on the compiled
/// path for now.
pub fn interpret(plan: &ResolvedPlan, event: &Event) -> Result<Option<OutputRow>> {
    interpret_systematic(plan, event, "Nominal")
}

/// Interpret one event for a concrete systematic variation.
pub fn interpret_systematic(
    plan: &ResolvedPlan,
    event: &Event,
    systematic: &str,
) -> Result<Option<OutputRow>> {
    if !plan.spec.channels.is_empty() {
        return Err(InterpretError::Unsupported(
            "use interpret_union for multi-channel union specs".to_string(),
        ));
    }
    ensure_interpretable_models(&plan.spec.models)?;

    let kir = crate::kir::lower_plan_to_kir(plan)?;
    crate::kir::verify(&kir)?;
    let model_outputs = evaluate_mock_models(&plan.spec.models, &kir, event, systematic)?;
    execute_verified_kir(&kir, event, systematic.to_string(), model_outputs)
}

/// Interpret one event and execute KIR histogram fills into `histograms`.
pub fn interpret_and_fill(
    plan: &ResolvedPlan,
    event: &Event,
    histograms: &mut InterpretedHistograms,
) -> Result<Option<OutputRow>> {
    interpret_and_fill_systematic(plan, event, histograms, "Nominal")
}

/// Interpret one event for a concrete systematic variation and fill histograms.
pub fn interpret_and_fill_systematic(
    plan: &ResolvedPlan,
    event: &Event,
    histograms: &mut InterpretedHistograms,
    systematic: &str,
) -> Result<Option<OutputRow>> {
    if !plan.spec.channels.is_empty() {
        return Err(InterpretError::Unsupported(
            "interpret_and_fill currently supports flat specs".to_string(),
        ));
    }
    ensure_interpretable_models(&plan.spec.models)?;

    let kir = crate::kir::lower_plan_to_kir(plan)?;
    crate::kir::verify(&kir)?;
    let model_outputs = evaluate_mock_models(&plan.spec.models, &kir, event, systematic)?;
    let mut evaluator = KirEvaluator::new(&kir, event, systematic.to_string(), model_outputs);
    evaluator.histograms = Some(&mut histograms.histograms);
    match evaluator.execute_block(&kir.block)? {
        BlockOutcome::Continue => Err(InterpretError::InvalidExpression(
            "KIR program completed without returning outputs".to_string(),
        )),
        BlockOutcome::Return(row) => {
            if row.is_some()
                && (!plan.spec.has_weight_systematic() || plan.spec.has_shape_correction())
                && !evaluator.fill_current_histograms()?
            {
                return Ok(None);
            }
            Ok(row)
        }
    }
}

fn execute_verified_kir(
    program: &KirProgram,
    event: &Event,
    systematic: String,
    model_outputs: ModelOutputs,
) -> Result<Option<OutputRow>> {
    let mut evaluator = KirEvaluator::new(program, event, systematic, model_outputs);
    match evaluator.execute_block(&program.block)? {
        BlockOutcome::Continue => Err(InterpretError::InvalidExpression(
            "KIR program completed without returning outputs".to_string(),
        )),
        BlockOutcome::Return(row) => Ok(row),
    }
}

fn ensure_interpretable_models(models: &[ModelDef]) -> Result<()> {
    for model in models {
        if !matches!(model.provider.kind, ModelProviderKind::Mock) {
            return Err(InterpretError::Unsupported(format!(
                "model `{}` provider `{}` is unsupported in interpreter; only mock provider is interpreted",
                model.name,
                provider_kind_name(&model.provider.kind)
            )));
        }
    }
    Ok(())
}

fn evaluate_mock_models(
    models: &[ModelDef],
    program: &KirProgram,
    event: &Event,
    systematic: &str,
) -> Result<ModelOutputs> {
    let mut outputs = ModelOutputs::with_capacity(models.len());
    for model in models {
        let batch = model_batch_source(model, program)?;
        let collection = event.collection(&batch)?;
        let mut values = Vec::with_capacity(collection.len() * model.inputs.len());
        for item in collection.iter() {
            for input in &model.inputs {
                values.push(model_input_value(
                    model, program, event, item, input, &batch, systematic,
                )?);
            }
        }
        let scores = mock_scores(&InferRequest {
            model: model.name.clone(),
            inputs: vec![Tensor {
                name: "features".to_string(),
                shape: vec![collection.len(), model.inputs.len()],
                data: TensorData::F32(values),
            }],
        })
        .map_err(|error| {
            InterpretError::Event(format!(
                "mock inference for model `{}` failed: {error}",
                model.name
            ))
        })?;
        if scores.len() != collection.len() {
            return Err(InterpretError::InvalidExpression(format!(
                "mock model `{}` returned {} scores for {} `{batch}` objects",
                model.name,
                scores.len(),
                collection.len()
            )));
        }
        outputs.insert(model.output.clone(), scores);
    }
    Ok(outputs)
}

fn model_input_value(
    model: &ModelDef,
    program: &KirProgram,
    event: &Event,
    item: &ObjectView<'_>,
    input: &str,
    batch: &str,
    systematic: &str,
) -> Result<f32> {
    let branch_type = branch_type(program, input)?;
    if branch_type.is_vector() {
        let Some((source, attr)) = input.split_once('_') else {
            return Err(InterpretError::Unsupported(format!(
                "model `{}` input `{input}` is not a supported object attribute branch",
                model.name
            )));
        };
        if source != batch {
            return Err(InterpretError::Unsupported(format!(
                "model `{}` input `{input}` is a vector branch outside batch `{batch}`",
                model.name
            )));
        }
        let value = object_input_value(item, attr, branch_type)?;
        let factor = shape_factor_for_source(program, batch, attr, systematic);
        Ok((f64::from(value) * factor) as f32)
    } else {
        scalar_input_value(event, input, branch_type)
    }
}

fn object_input_value(item: &ObjectView<'_>, attr: &str, branch_type: BranchType) -> Result<f32> {
    match branch_type {
        BranchType::VecI8 => Ok(f32::from(item.get::<i8>(attr)?)),
        BranchType::VecU8 => Ok(f32::from(item.get::<u8>(attr)?)),
        BranchType::VecI16 => Ok(f32::from(item.get::<i16>(attr)?)),
        BranchType::VecU16 => Ok(f32::from(item.get::<u16>(attr)?)),
        BranchType::VecI32 => Ok(item.get::<i32>(attr)? as f32),
        BranchType::VecU32 => Ok(item.get::<u32>(attr)? as f32),
        BranchType::VecI64 => Ok(item.get::<i64>(attr)? as f32),
        BranchType::VecU64 => Ok(item.get::<u64>(attr)? as f32),
        BranchType::VecF32 => Ok(item.get::<f32>(attr)?),
        other => Err(InterpretError::TypeMismatch {
            branch: attr.to_string(),
            branch_type: other,
            expected: "numeric vector branch",
        }),
    }
}

fn scalar_input_value(event: &Event, input: &str, branch_type: BranchType) -> Result<f32> {
    match branch_type {
        BranchType::I8 => Ok(f32::from(event.scalar::<i8>(input)?)),
        BranchType::U8 => Ok(f32::from(event.scalar::<u8>(input)?)),
        BranchType::I16 => Ok(f32::from(event.scalar::<i16>(input)?)),
        BranchType::U16 => Ok(f32::from(event.scalar::<u16>(input)?)),
        BranchType::I32 => Ok(event.scalar::<i32>(input)? as f32),
        BranchType::U32 => Ok(event.scalar::<u32>(input)? as f32),
        BranchType::I64 => Ok(event.scalar::<i64>(input)? as f32),
        BranchType::U64 => Ok(event.scalar::<u64>(input)? as f32),
        BranchType::F32 => Ok(event.scalar::<f32>(input)?),
        other => Err(InterpretError::TypeMismatch {
            branch: input.to_string(),
            branch_type: other,
            expected: "numeric scalar branch",
        }),
    }
}

fn model_batch_source(model: &ModelDef, program: &KirProgram) -> Result<String> {
    if let Some(object) = program
        .objects
        .iter()
        .find(|object| object.name == model.batch)
    {
        return Ok(object.source.clone());
    }
    if program
        .objects
        .iter()
        .any(|object| object.source == model.batch)
    {
        return Ok(model.batch.clone());
    }
    Err(InterpretError::Unsupported(format!(
        "model `{}` batch `{}` is not defined",
        model.name, model.batch
    )))
}

fn branch_type(program: &KirProgram, branch: &str) -> Result<BranchType> {
    program
        .read_branches
        .iter()
        .find(|spec| spec.name == branch)
        .map(|spec| spec.branch_type)
        .ok_or_else(|| InterpretError::MissingBranch(branch.to_string()))
}

fn provider_kind_name(kind: &ModelProviderKind) -> &str {
    match kind {
        ModelProviderKind::Mock => "mock",
        ModelProviderKind::InProcess => "inproc",
        ModelProviderKind::Remote => "remote",
        ModelProviderKind::Managed => "managed",
        ModelProviderKind::Other(kind) => kind.as_str(),
    }
}

fn interpreted_systematic_variants(plan: &ResolvedPlan) -> Vec<String> {
    interpreted_systematic_variants_from_parts(&plan.spec.systematics, &plan.spec.shape_corrections)
}

fn interpreted_systematic_variants_from_parts(
    systematics: &[SystematicDef],
    shape_corrections: &[impl NamedSystematicCorrection],
) -> Vec<String> {
    let mut variants = vec!["Nominal".to_string()];
    for systematic in systematics {
        if let SystematicDef::Weight(systematic) = systematic {
            variants.push(interpreted_variant_name(&systematic.name, "Up"));
            variants.push(interpreted_variant_name(&systematic.name, "Down"));
        }
    }
    for correction in shape_corrections {
        variants.push(interpreted_variant_name(correction.name(), "Up"));
        variants.push(interpreted_variant_name(correction.name(), "Down"));
    }
    variants
}

trait NamedSystematicCorrection {
    fn name(&self) -> &str;
}

impl NamedSystematicCorrection for crate::ShapeCorrectionDef {
    fn name(&self) -> &str {
        &self.name
    }
}

impl NamedSystematicCorrection for KirShapeCorrection {
    fn name(&self) -> &str {
        &self.name
    }
}

fn interpreted_variant_name(name: &str, direction: &str) -> String {
    format!("{}{direction}", interpreted_upper_camel(name))
}

fn interpreted_upper_camel(name: &str) -> String {
    let mut ident = String::new();
    for part in name.split('_') {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            ident.push(first.to_ascii_uppercase());
            ident.extend(chars);
        }
    }
    ident
}

struct KirEvaluator<'a> {
    program: &'a KirProgram,
    event: &'a Event,
    systematic: String,
    values: RuntimeValues,
    selected: SelectedObjects,
    derived: DerivedObjects,
    model_outputs: ModelOutputs,
    histograms: Option<&'a mut BTreeMap<String, HistSet1D<String>>>,
}

impl<'a> KirEvaluator<'a> {
    fn new(
        program: &'a KirProgram,
        event: &'a Event,
        systematic: String,
        model_outputs: ModelOutputs,
    ) -> Self {
        Self {
            program,
            event,
            systematic,
            values: HashMap::new(),
            selected: HashMap::with_capacity(program.objects.len()),
            derived: HashMap::with_capacity(program.derived_objects.len()),
            model_outputs,
            histograms: None,
        }
    }

    fn execute_block(&mut self, block: &Block) -> Result<BlockOutcome> {
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let { value, expr } => {
                    let runtime = self.eval_rvalue(expr)?;
                    if matches!(runtime, RuntimeValue::Output(None)) {
                        return Ok(BlockOutcome::Return(None));
                    }
                    self.values.insert(value.id, runtime);
                }
                Stmt::Require { condition } => {
                    let RuntimeValue::Bool(passed) = self.value(*condition)? else {
                        return Err(InterpretError::InvalidExpression(format!(
                            "KIR require expected bool value {condition:?}"
                        )));
                    };
                    if !passed {
                        return Ok(BlockOutcome::Return(None));
                    }
                }
                Stmt::Return { values } => {
                    let mut row = Vec::with_capacity(values.len());
                    for returned in values {
                        let RuntimeValue::Output(value) = self.value(returned.value)? else {
                            return Err(InterpretError::InvalidExpression(format!(
                                "KIR return expected output value {:?}",
                                returned.value
                            )));
                        };
                        let Some(value) = value else {
                            return Ok(BlockOutcome::Return(None));
                        };
                        row.push((returned.name.clone(), value));
                    }
                    return Ok(BlockOutcome::Return(Some(OutputRow::new(row))));
                }
                Stmt::ForEach { axis, item, body } => {
                    self.execute_for_each(*axis, item.id, body)?;
                }
                Stmt::Fill {
                    histogram,
                    value,
                    weight,
                } => {
                    self.execute_fill(*histogram, *value, *weight)?;
                }
                Stmt::If { .. } => {
                    return Err(InterpretError::Unsupported(
                        "KIR if control is reserved for a later interpreter move".to_string(),
                    ));
                }
            }
        }
        Ok(BlockOutcome::Continue)
    }

    fn execute_for_each(&mut self, axis: ForEachAxis, item: ValueId, body: &Block) -> Result<()> {
        match axis {
            ForEachAxis::Systematic => {
                for systematic in self.active_systematics() {
                    self.values
                        .insert(item, RuntimeValue::Systematic(systematic.clone()));
                    match self.execute_block(body)? {
                        BlockOutcome::Continue => {}
                        BlockOutcome::Return(_) => {
                            return Err(InterpretError::InvalidExpression(
                                "KIR systematic loop body returned unexpectedly".to_string(),
                            ));
                        }
                    }
                }
                self.values.remove(&item);
            }
        }
        Ok(())
    }

    fn execute_fill(
        &mut self,
        histogram: ValueId,
        value: ValueId,
        weight: Option<ValueId>,
    ) -> Result<()> {
        let RuntimeValue::Histogram(histogram) = self.value(histogram)? else {
            return Err(InterpretError::InvalidExpression(format!(
                "KIR fill expected histogram value {histogram:?}"
            )));
        };
        let RuntimeValue::Numeric(value) = self.value(value)? else {
            return Err(InterpretError::InvalidExpression(format!(
                "KIR fill expected numeric value {value:?}"
            )));
        };
        let Some(weight) = weight else {
            return Err(InterpretError::InvalidExpression(
                "KIR fill requires a weight".to_string(),
            ));
        };
        let RuntimeValue::Weight(weight) = self.value(weight)? else {
            return Err(InterpretError::InvalidExpression(format!(
                "KIR fill expected weight value {weight:?}"
            )));
        };
        let systematic = self.current_systematic()?;
        let Some(histograms) = &mut self.histograms else {
            return Ok(());
        };
        let Some(histogram) = histograms.get_mut(&histogram) else {
            return Err(InterpretError::InvalidExpression(format!(
                "histogram `{histogram}` was not initialized"
            )));
        };
        histogram
            .get_mut(systematic)
            .fill_weighted(value, weight.value());
        Ok(())
    }

    fn fill_current_histograms(&mut self) -> Result<bool> {
        let systematic = self.systematic.clone();
        let weight = self.weight_for(&systematic);
        for histogram in &self.program.histograms {
            if expr_has_missing_value(&histogram.def.expr, &self.selected, &self.derived)? {
                return Ok(false);
            }
            let value =
                eval_numeric_expr(&histogram.def.expr, &self.selected, &self.derived, None)?
                    .as_f64();
            let Some(histograms) = &mut self.histograms else {
                return Ok(true);
            };
            let Some(output) = histograms.get_mut(&histogram.name) else {
                return Err(InterpretError::InvalidExpression(format!(
                    "histogram `{}` was not initialized",
                    histogram.name
                )));
            };
            output
                .get_mut(systematic.clone())
                .fill_weighted(value, weight.value());
        }
        Ok(true)
    }

    fn eval_rvalue(&mut self, expr: &Rvalue) -> Result<RuntimeValue> {
        match expr {
            Rvalue::SelectObjects { object } => {
                let selected = select_object(
                    self.program,
                    self.event,
                    object,
                    &self.systematic,
                    &self.model_outputs,
                )?;
                self.selected.insert(object.name.clone(), selected);
                Ok(RuntimeValue::ObjectSet)
            }
            Rvalue::DeriveObject { object } => {
                let value = derive_object(&object.def, &self.selected, &self.derived)?;
                self.derived.insert(object.name.clone(), value);
                Ok(RuntimeValue::Candidate)
            }
            Rvalue::Requirement { requirement } => {
                if expr_has_missing_value(&requirement.lhs, &self.selected, &self.derived)? {
                    return Ok(RuntimeValue::Bool(false));
                }
                let lhs = eval_numeric_expr(&requirement.lhs, &self.selected, &self.derived, None)?;
                Ok(RuntimeValue::Bool(compare(
                    lhs.as_f64(),
                    requirement.op,
                    requirement.rhs.value,
                )))
            }
            Rvalue::Output { expr, .. } => Ok(RuntimeValue::Output(eval_output_expr(
                expr,
                &self.selected,
                &self.derived,
            )?)),
            Rvalue::Histogram { histogram } => Ok(RuntimeValue::Histogram(histogram.name.clone())),
            Rvalue::HistogramValue { expr, .. } => {
                if expr_has_missing_value(expr, &self.selected, &self.derived)? {
                    return Ok(RuntimeValue::Output(None));
                }
                Ok(RuntimeValue::Numeric(
                    eval_numeric_expr(expr, &self.selected, &self.derived, None)?.as_f64(),
                ))
            }
            Rvalue::Weight { systematic } => {
                let RuntimeValue::Systematic(systematic) = self.value(*systematic)? else {
                    return Err(InterpretError::InvalidExpression(format!(
                        "KIR weight expected systematic value {systematic:?}"
                    )));
                };
                Ok(RuntimeValue::Weight(self.weight_for(&systematic)))
            }
            Rvalue::Literal(_)
            | Rvalue::Quantity(_)
            | Rvalue::ObjectRef(_)
            | Rvalue::CandidateRef(_)
            | Rvalue::Attr { .. }
            | Rvalue::DerivedAttr { .. }
            | Rvalue::Call { .. }
            | Rvalue::Compare { .. } => Err(InterpretError::Unsupported(format!(
                "KIR rvalue `{expr:?}` is not part of flat executable interpretation yet"
            ))),
        }
    }

    fn active_systematics(&self) -> Vec<String> {
        interpreted_systematic_variants_from_parts(
            &self.program.systematics,
            &self.program.shape_corrections,
        )
    }

    fn current_systematic(&self) -> Result<String> {
        Ok(self
            .values
            .values()
            .find_map(|value| match value {
                RuntimeValue::Systematic(systematic) => Some(systematic.clone()),
                _ => None,
            })
            .unwrap_or_else(|| self.systematic.clone()))
    }

    fn weight_for(&self, systematic: &str) -> EventWeight {
        let mut weight = self
            .program
            .systematics
            .iter()
            .find_map(|declared| match declared {
                crate::SystematicDef::Weight(systematic) => Some(systematic),
                _ => None,
            })
            .map(|declared| match systematic {
                value if value == interpreted_variant_name(&declared.name, "Up") => {
                    EventWeight::nominal().times(declared.up)
                }
                value if value == interpreted_variant_name(&declared.name, "Down") => {
                    EventWeight::nominal().times(declared.down)
                }
                _ => EventWeight::nominal(),
            })
            .unwrap_or_else(EventWeight::nominal);
        for factor in &self.program.weight.nominal {
            weight = weight.times(*factor);
        }
        weight
    }

    fn value(&self, id: ValueId) -> Result<RuntimeValue> {
        self.values
            .get(&id)
            .cloned()
            .ok_or_else(|| InterpretError::InvalidExpression(format!("KIR value {id:?} missing")))
    }
}

/// Interpret one event with a multi-channel union plan.
///
/// Each matching channel contributes one row, preserving the spec channel order.
pub fn interpret_union(plan: &ResolvedPlan, event: &Event) -> Result<Vec<ChannelOutputRow>> {
    if plan.spec.channels.is_empty() {
        return interpret(plan, event).map(|row| {
            row.into_iter()
                .map(|row| ChannelOutputRow {
                    channel: plan.spec.name.clone(),
                    row,
                })
                .collect()
        });
    }

    let mut rows = Vec::new();
    for channel in &plan.spec.channels {
        let channel_plan = ResolvedPlan {
            spec: channel.as_spec(&plan.spec),
            read_branches: plan.read_branches.clone(),
        };
        if let Some(row) = interpret(&channel_plan, event)? {
            rows.push(ChannelOutputRow {
                channel: channel.name.clone(),
                row,
            });
        }
    }
    Ok(rows)
}

fn passes_object_cuts(
    program: &KirProgram,
    object: &KirObject,
    item: &ObjectView<'_>,
    systematic: &str,
    model_outputs: &ModelOutputs,
) -> Result<bool> {
    for cut in &object.cuts {
        let lhs = eval_object_numeric_expr(
            program,
            &object.name,
            &object.source,
            &cut.lhs,
            item,
            systematic,
            model_outputs,
        )?;
        if !compare(lhs.as_f64(), cut.op, cut.rhs.value) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn select_object(
    program: &KirProgram,
    event: &Event,
    object: &KirObject,
    systematic: &str,
    model_outputs: &ModelOutputs,
) -> Result<Vec<SelectedObject>> {
    let collection = event.collection(&object.source)?;
    let mut objects = Vec::new();

    for item in collection.iter() {
        let mut leading_values = HashMap::new();
        if passes_object_cuts(program, object, item, systematic, model_outputs)? {
            for attr in leading_attrs_for_object(program, &object.name) {
                let value = read_object_attr(
                    program,
                    &object.name,
                    &object.source,
                    item,
                    &attr,
                    systematic,
                    model_outputs,
                )?;
                leading_values.insert(attr, value);
            }
            objects.push(SelectedObject {
                source_index: item.index(),
                leading_values,
            });
        }
    }

    Ok(objects)
}

fn derive_object(
    object: &DerivedObjectDef,
    selected: &SelectedObjects,
    derived: &DerivedObjects,
) -> Result<Option<DerivedObject>> {
    match &object.source {
        DerivedSource::Pair(pair) => derive_pair(pair, selected, derived),
        DerivedSource::Candidate(candidate) => derive_candidate(candidate, selected, derived),
    }
}

fn derive_pair(
    pair: &ObjectPairDef,
    selected: &SelectedObjects,
    derived: &DerivedObjects,
) -> Result<Option<DerivedObject>> {
    let objects = selected
        .get(&pair.object)
        .ok_or_else(|| InterpretError::MissingObject(pair.object.clone()))?;
    let mut excluded = Vec::new();
    for name in &pair.exclude {
        if let Some(object) = derived_object(derived, name)? {
            excluded.extend(
                object
                    .constituents
                    .iter()
                    .filter(|item| item.object == pair.object)
                    .map(|item| item.index),
            );
        }
    }

    let mut order = (0..objects.len()).collect::<Vec<_>>();
    if !matches!(pair.selection, PairSelection::NearestMassTruncated { .. }) {
        order.sort_by(|&left, &right| {
            attr_f64(&objects[right], "pt").total_cmp(&attr_f64(&objects[left], "pt"))
        });
    }

    let target = match &pair.selection {
        PairSelection::LeadingPt => None,
        PairSelection::NearestMass { target } => Some(target.value),
        PairSelection::NearestMassTruncated { .. } => None,
    };
    let truncated_target = match &pair.selection {
        PairSelection::NearestMassTruncated { target } => Some(target.value),
        PairSelection::LeadingPt | PairSelection::NearestMass { .. } => None,
    };
    let mut best = None;
    let mut best_diff = None::<f64>;
    let mut best_mass = -1_i32;
    for (left_pos, &left) in order.iter().enumerate() {
        for &right in &order[left_pos + 1..] {
            let first = &objects[left];
            let second = &objects[right];
            if excluded.contains(&first.source_index) || excluded.contains(&second.source_index) {
                continue;
            }
            if !passes_pair_constraints(pair, first, second)? {
                continue;
            }
            if !passes_pair_filters(pair, first, second)? {
                continue;
            }
            let candidate = combine_selected(&pair.object, [first, second])?;
            if !candidate.mass.is_finite() || candidate.mass <= 0.0 {
                continue;
            }
            if let Some(target) = target {
                let diff = (candidate.mass - target).abs();
                if best_diff.is_none_or(|best| diff < best) {
                    best_diff = Some(diff);
                    best = Some(candidate);
                }
            } else if let Some(target) = truncated_target {
                if (target - candidate.mass).abs() < (target - f64::from(best_mass)).abs() {
                    best_mass = candidate.mass as i32;
                    best = Some(candidate);
                }
            } else {
                return Ok(Some(candidate));
            }
        }
    }
    Ok(best)
}

fn derive_candidate(
    candidate_def: &ObjectCandidateDef,
    selected: &SelectedObjects,
    derived: &DerivedObjects,
) -> Result<Option<DerivedObject>> {
    let mut occurrences = HashMap::<&str, usize>::new();
    let mut energy = 0.0;
    let mut px = 0.0;
    let mut py = 0.0;
    let mut pz = 0.0;
    let mut constituents = Vec::new();

    for item in &candidate_def.items {
        if let Some(objects) = selected.get(item) {
            let occurrence = occurrences.entry(item.as_str()).or_insert(0);
            let Some(object) = objects.get(*occurrence) else {
                return Ok(None);
            };
            let (item_e, item_px, item_py, item_pz) = selected_four_vector(object);
            energy += item_e;
            px += item_px;
            py += item_py;
            pz += item_pz;
            constituents.push(Constituent {
                object: item.clone(),
                index: object.source_index,
                pt: object
                    .leading_values
                    .get("pt")
                    .copied()
                    .unwrap_or(NumericValue::F64(0.0)),
                eta: object
                    .leading_values
                    .get("eta")
                    .copied()
                    .unwrap_or(NumericValue::F64(0.0)),
                phi: object
                    .leading_values
                    .get("phi")
                    .copied()
                    .unwrap_or(NumericValue::F64(0.0)),
            });
            *occurrence += 1;
        } else if derived.contains_key(item) {
            let Some(object) = derived_object(derived, item)? else {
                return Ok(None);
            };
            energy += object.energy;
            px += object.px;
            py += object.py;
            pz += object.pz;
            constituents.extend(object.constituents.iter().cloned());
        } else {
            return Err(InterpretError::MissingObject(item.clone()));
        }
    }

    let (mass, pt) = mass_pt(energy, px, py, pz);
    if mass.is_finite() && mass > 0.0 {
        let candidate = DerivedObject {
            mass,
            pt,
            min_delta_r: candidate_min_delta_r(&constituents),
            energy,
            px,
            py,
            pz,
            constituents,
        };
        if passes_candidate_filters(&candidate, &candidate_def.filters)? {
            Ok(Some(candidate))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

fn passes_pair_constraints(
    pair: &ObjectPairDef,
    first: &SelectedObject,
    second: &SelectedObject,
) -> Result<bool> {
    for constraint in &pair.constraints {
        match constraint {
            PairConstraint::OppositeCharge => {
                if attr_f64(first, "charge") * attr_f64(second, "charge") >= 0.0 {
                    return Ok(false);
                }
            }
            PairConstraint::SameFlavor => {}
        }
    }
    Ok(true)
}

fn passes_pair_filters(
    pair: &ObjectPairDef,
    first: &SelectedObject,
    second: &SelectedObject,
) -> Result<bool> {
    for filter in &pair.filters {
        let lhs = eval_pair_filter_expr(&filter.lhs, first, second)?;
        if !compare(lhs, filter.op, filter.rhs.value) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn eval_pair_filter_expr(
    expr: &Expr,
    first: &SelectedObject,
    second: &SelectedObject,
) -> Result<f64> {
    match expr {
        Expr::PairDeltaR => Ok(delta_r(
            attr_f64(first, "eta"),
            attr_f64(first, "phi"),
            attr_f64(second, "eta"),
            attr_f64(second, "phi"),
        )),
        Expr::PairLeadingPt => Ok(attr_f64(first, "pt").max(attr_f64(second, "pt"))),
        Expr::PairSubleadingPt => Ok(attr_f64(first, "pt").min(attr_f64(second, "pt"))),
        other => Err(InterpretError::InvalidExpression(format!(
            "unsupported pair filter expression `{other}`"
        ))),
    }
}

fn passes_candidate_filters(candidate: &DerivedObject, filters: &[Cut]) -> Result<bool> {
    for filter in filters {
        let lhs = eval_candidate_filter_expr(&filter.lhs, candidate)?;
        if !compare(lhs, filter.op, filter.rhs.value) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn eval_candidate_filter_expr(expr: &Expr, candidate: &DerivedObject) -> Result<f64> {
    match expr {
        Expr::CandidateLeadingPt => Ok(candidate
            .constituents
            .iter()
            .map(|item| item.pt.as_f64())
            .fold(f64::NEG_INFINITY, f64::max)),
        Expr::CandidateSubleadingPt => {
            let mut pts = candidate
                .constituents
                .iter()
                .map(|item| item.pt.as_f64())
                .collect::<Vec<_>>();
            pts.sort_by(|left, right| right.total_cmp(left));
            Ok(pts.get(1).copied().unwrap_or(f64::NEG_INFINITY))
        }
        Expr::CandidateMinDeltaR => {
            let mut min = f64::INFINITY;
            for (left_pos, left) in candidate.constituents.iter().enumerate() {
                for right in &candidate.constituents[left_pos + 1..] {
                    min = min.min(delta_r(
                        left.eta.as_f64(),
                        left.phi.as_f64(),
                        right.eta.as_f64(),
                        right.phi.as_f64(),
                    ));
                }
            }
            Ok(min)
        }
        other => Err(InterpretError::InvalidExpression(format!(
            "unsupported candidate filter expression `{other}`"
        ))),
    }
}

fn combine_selected<'a>(
    object: &str,
    items: impl IntoIterator<Item = &'a SelectedObject>,
) -> Result<DerivedObject> {
    let mut energy = 0.0;
    let mut px = 0.0;
    let mut py = 0.0;
    let mut pz = 0.0;
    let mut constituents = Vec::new();
    for item in items {
        let (item_e, item_px, item_py, item_pz) = selected_four_vector(item);
        energy += item_e;
        px += item_px;
        py += item_py;
        pz += item_pz;
        constituents.push(Constituent {
            object: object.to_string(),
            index: item.source_index,
            pt: item
                .leading_values
                .get("pt")
                .copied()
                .unwrap_or(NumericValue::F64(0.0)),
            eta: item
                .leading_values
                .get("eta")
                .copied()
                .unwrap_or(NumericValue::F64(0.0)),
            phi: item
                .leading_values
                .get("phi")
                .copied()
                .unwrap_or(NumericValue::F64(0.0)),
        });
    }
    let (mass, pt) = mass_pt(energy, px, py, pz);
    Ok(DerivedObject {
        mass,
        pt,
        min_delta_r: candidate_min_delta_r(&constituents),
        energy,
        px,
        py,
        pz,
        constituents,
    })
}

fn selected_four_vector(item: &SelectedObject) -> (f64, f64, f64, f64) {
    let pt = attr_f64(item, "pt");
    let eta = attr_f64(item, "eta");
    let phi = attr_f64(item, "phi");
    let mass = attr_f64(item, "mass");
    let px = pt * phi.cos();
    let py = pt * phi.sin();
    let pz = pt * eta.sinh();
    let energy = (px * px + py * py + pz * pz + mass * mass).sqrt();
    (energy, px, py, pz)
}

fn attr_f64(item: &SelectedObject, attr: &str) -> f64 {
    item.leading_values
        .get(attr)
        .copied()
        .map(NumericValue::as_f64)
        .unwrap_or(0.0)
}

fn mass_pt(energy: f64, px: f64, py: f64, pz: f64) -> (f64, f64) {
    (
        (energy * energy - px * px - py * py - pz * pz)
            .max(0.0)
            .sqrt(),
        (px * px + py * py).sqrt(),
    )
}

fn delta_r(left_eta: f64, left_phi: f64, right_eta: f64, right_phi: f64) -> f64 {
    let deta = left_eta - right_eta;
    let mut dphi = left_phi - right_phi;
    while dphi > std::f64::consts::PI {
        dphi -= 2.0 * std::f64::consts::PI;
    }
    while dphi <= -std::f64::consts::PI {
        dphi += 2.0 * std::f64::consts::PI;
    }
    (deta * deta + dphi * dphi).sqrt()
}

fn candidate_min_delta_r(constituents: &[Constituent]) -> f64 {
    let mut min = f64::INFINITY;
    for (left_pos, left) in constituents.iter().enumerate() {
        for right in &constituents[left_pos + 1..] {
            min = min.min(delta_r(
                left.eta.as_f64(),
                left.phi.as_f64(),
                right.eta.as_f64(),
                right.phi.as_f64(),
            ));
        }
    }
    min
}

fn derived_object<'a>(
    derived: &'a DerivedObjects,
    name: &str,
) -> Result<Option<&'a DerivedObject>> {
    derived
        .get(name)
        .map(Option::as_ref)
        .ok_or_else(|| InterpretError::MissingObject(name.to_string()))
}

fn leading_attrs_for_object(program: &KirProgram, object_name: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    for output in &program.outputs {
        if let Expr::LeadingAttr { object, attr } = &output.expr {
            if object == object_name && !attrs.contains(attr) {
                attrs.push(attr.clone());
            }
        }
    }
    for region in &program.regions {
        for requirement in &region.requirements {
            collect_leading_attrs(&requirement.lhs, object_name, &mut attrs);
            collect_selected_attrs(&requirement.lhs, object_name, &mut attrs);
        }
    }
    for output in &program.outputs {
        collect_selected_attrs(&output.expr, object_name, &mut attrs);
    }
    for derived in &program.derived_objects {
        match &derived.def.source {
            DerivedSource::Pair(pair) if pair.object == object_name => {
                for attr in ["pt", "eta", "phi", "mass"] {
                    push_attr(&mut attrs, attr);
                }
                for constraint in &pair.constraints {
                    if matches!(constraint, PairConstraint::OppositeCharge) {
                        push_attr(&mut attrs, "charge");
                    }
                }
                for filter in &pair.filters {
                    collect_pair_filter_attrs(&filter.lhs, &mut attrs);
                }
            }
            DerivedSource::Candidate(candidate)
                if candidate.items.iter().any(|item| item == object_name) =>
            {
                for attr in ["pt", "eta", "phi", "mass"] {
                    push_attr(&mut attrs, attr);
                }
                for filter in &candidate.filters {
                    collect_candidate_filter_attrs(&filter.lhs, &mut attrs);
                }
            }
            _ => {}
        }
    }
    attrs
}

fn push_attr(attrs: &mut Vec<String>, attr: &str) {
    if !attrs.iter().any(|value| value == attr) {
        attrs.push(attr.to_string());
    }
}

fn collect_leading_attrs(expr: &Expr, object_name: &str, attrs: &mut Vec<String>) {
    match expr {
        Expr::LeadingAttr { object, attr } if object == object_name && !attrs.contains(attr) => {
            attrs.push(attr.clone());
        }
        Expr::Abs(inner) => collect_leading_attrs(inner, object_name, attrs),
        Expr::Sqrt(inner) => collect_leading_attrs(inner, object_name, attrs),
        Expr::Binary { lhs, rhs, .. } => {
            collect_leading_attrs(lhs, object_name, attrs);
            collect_leading_attrs(rhs, object_name, attrs);
        }
        _ => {}
    }
}

fn collect_selected_attrs(expr: &Expr, object_name: &str, attrs: &mut Vec<String>) {
    match expr {
        Expr::Attr { object, attr } if object == object_name => push_attr(attrs, attr),
        Expr::Binary { lhs, rhs, .. } => {
            collect_selected_attrs(lhs, object_name, attrs);
            collect_selected_attrs(rhs, object_name, attrs);
        }
        Expr::Abs(inner) | Expr::Sqrt(inner) => collect_selected_attrs(inner, object_name, attrs),
        Expr::CountWhere { object, predicate }
        | Expr::All { object, predicate }
        | Expr::Any { object, predicate }
            if object == object_name =>
        {
            collect_selected_attrs(&predicate.lhs, object_name, attrs);
        }
        Expr::SumAttr { object, attr } if object == object_name => push_attr(attrs, attr),
        _ => {}
    }
}

fn collect_pair_filter_attrs(expr: &Expr, attrs: &mut Vec<String>) {
    match expr {
        Expr::PairDeltaR => {
            push_attr(attrs, "eta");
            push_attr(attrs, "phi");
        }
        Expr::PairLeadingPt | Expr::PairSubleadingPt => push_attr(attrs, "pt"),
        _ => {}
    }
}

fn collect_candidate_filter_attrs(expr: &Expr, attrs: &mut Vec<String>) {
    match expr {
        Expr::CandidateMinDeltaR => {
            push_attr(attrs, "eta");
            push_attr(attrs, "phi");
        }
        Expr::CandidateLeadingPt | Expr::CandidateSubleadingPt => push_attr(attrs, "pt"),
        _ => {}
    }
}

fn eval_object_numeric_expr(
    program: &KirProgram,
    current_object: &str,
    source: &str,
    expr: &Expr,
    item: &ObjectView<'_>,
    systematic: &str,
    model_outputs: &ModelOutputs,
) -> Result<NumericValue> {
    match expr {
        Expr::Attr { object, attr } if object == current_object => {
            read_object_attr(
                program,
                current_object,
                source,
                item,
                attr,
                systematic,
                model_outputs,
            )
        }
        Expr::Attr { object, .. } => Err(InterpretError::Unsupported(format!(
            "object `{current_object}` cut references `{object}`; this slice only supports cuts on the object being selected"
        ))),
        Expr::Literal(value) => Ok(NumericValue::F64(*value)),
        Expr::Binary { op, lhs, rhs } => {
            let lhs =
                eval_object_numeric_expr(
                    program,
                    current_object,
                    source,
                    lhs,
                    item,
                    systematic,
                    model_outputs,
                )?
                    .as_f64();
            let rhs =
                eval_object_numeric_expr(
                    program,
                    current_object,
                    source,
                    rhs,
                    item,
                    systematic,
                    model_outputs,
                )?
                    .as_f64();
            Ok(NumericValue::F64(eval_arithmetic(*op, lhs, rhs)))
        }
        Expr::Abs(inner) => Ok(eval_object_numeric_expr(
            program,
            current_object,
            source,
            inner,
            item,
            systematic,
            model_outputs,
        )?
        .abs()),
        Expr::Sqrt(inner) => Ok(NumericValue::F64(
            eval_object_numeric_expr(
                program,
                current_object,
                source,
                inner,
                item,
                systematic,
                model_outputs,
            )?
                .as_f64()
                .sqrt(),
        )),
        other => Err(InterpretError::Unsupported(format!(
            "object cut expression `{other}` is not supported by the interpreter"
        ))),
    }
}

fn read_object_attr(
    program: &KirProgram,
    collection: &str,
    source: &str,
    item: &ObjectView<'_>,
    attr: &str,
    systematic: &str,
    model_outputs: &ModelOutputs,
) -> Result<NumericValue> {
    let branch = format!("{source}_{attr}");
    if let Some(values) = model_outputs.get(&branch) {
        return values
            .get(item.index())
            .copied()
            .map(|value| NumericValue::F64(f64::from(value)))
            .ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "model output `{branch}` has no value for source index {}",
                    item.index()
                ))
            });
    }
    let branch_type = program
        .read_branches
        .iter()
        .find(|spec| spec.name == branch)
        .ok_or_else(|| InterpretError::MissingBranch(branch.clone()))?
        .branch_type;

    match branch_type {
        BranchType::VecI8 => Ok(NumericValue::I64(i64::from(item.get::<i8>(attr)?))),
        BranchType::VecU8 => Ok(NumericValue::U64(u64::from(item.get::<u8>(attr)?))),
        BranchType::VecI16 => Ok(NumericValue::I64(i64::from(item.get::<i16>(attr)?))),
        BranchType::VecU16 => Ok(NumericValue::U64(u64::from(item.get::<u16>(attr)?))),
        BranchType::VecI32 => Ok(NumericValue::I64(i64::from(item.get::<i32>(attr)?))),
        BranchType::VecU32 => Ok(NumericValue::U64(u64::from(item.get::<u32>(attr)?))),
        BranchType::VecI64 => Ok(NumericValue::I64(item.get::<i64>(attr)?)),
        BranchType::VecU64 => Ok(NumericValue::U64(item.get::<u64>(attr)?)),
        BranchType::VecF32 => {
            let value = item.get::<f32>(attr)?;
            let factor = shape_factor(program, collection, attr, systematic);
            Ok(NumericValue::F64(f64::from(
                (f64::from(value) * factor) as f32,
            )))
        }
        other => Err(InterpretError::TypeMismatch {
            branch,
            branch_type: other,
            expected: "numeric vector branch",
        }),
    }
}

fn shape_factor(program: &KirProgram, collection: &str, attr: &str, systematic: &str) -> f64 {
    program
        .shape_corrections
        .iter()
        .find(|correction| correction.collection == collection && correction.attr == attr)
        .map(|correction| match systematic {
            value if value == interpreted_variant_name(&correction.name, "Up") => correction.up,
            value if value == interpreted_variant_name(&correction.name, "Down") => correction.down,
            _ => 1.0,
        })
        .unwrap_or(1.0)
}

fn shape_factor_for_source(
    program: &KirProgram,
    source: &str,
    attr: &str,
    systematic: &str,
) -> f64 {
    program
        .shape_corrections
        .iter()
        .filter(|correction| {
            correction.attr == attr
                && program
                    .objects
                    .iter()
                    .any(|object| object.name == correction.collection && object.source == source)
        })
        .map(|correction| match systematic {
            value if value == interpreted_variant_name(&correction.name, "Up") => correction.up,
            value if value == interpreted_variant_name(&correction.name, "Down") => correction.down,
            _ => 1.0,
        })
        .product()
}

fn expr_has_missing_value(
    expr: &Expr,
    selected: &SelectedObjects,
    derived: &DerivedObjects,
) -> Result<bool> {
    match expr {
        Expr::Attr { object, .. } if derived.contains_key(object) => {
            Ok(derived_object(derived, object)?.is_none())
        }
        Expr::Attr { .. } | Expr::Literal(_) | Expr::Count(_) | Expr::SumAttr { .. } => Ok(false),
        Expr::Binary { lhs, rhs, .. } => Ok(expr_has_missing_value(lhs, selected, derived)?
            || expr_has_missing_value(rhs, selected, derived)?),
        Expr::Abs(inner) | Expr::Sqrt(inner) => expr_has_missing_value(inner, selected, derived),
        Expr::CountWhere { predicate, .. }
        | Expr::All { predicate, .. }
        | Expr::Any { predicate, .. } => expr_has_missing_value(&predicate.lhs, selected, derived),
        Expr::EitherPairPt { .. } => Ok(false),
        Expr::ClosestMass { left, right, .. } | Expr::OtherMass { left, right, .. } => {
            Ok(derived_object(derived, left)?.is_none()
                || derived_object(derived, right)?.is_none())
        }
        Expr::LeadingAttr { object, attr } => Ok(leading_value(selected, object, attr)?.is_none()),
        Expr::PairDeltaR
        | Expr::PairLeadingPt
        | Expr::PairSubleadingPt
        | Expr::CandidateMinDeltaR
        | Expr::CandidateLeadingPt
        | Expr::CandidateSubleadingPt => Ok(false),
    }
}

fn eval_output_expr(
    expr: &Expr,
    selected: &SelectedObjects,
    derived: &DerivedObjects,
) -> Result<Option<Value>> {
    match expr {
        Expr::Count(object) => {
            let count = selected
                .get(object)
                .ok_or_else(|| InterpretError::MissingObject(object.clone()))?
                .len();
            let count = u32::try_from(count).map_err(|error| {
                InterpretError::NumericConversion(format!(
                    "count({object}) cannot fit into u32: {error}"
                ))
            })?;
            Ok(Some(Value::U32(count)))
        }
        Expr::CountWhere { object, predicate } => {
            let count = count_where(selected, derived, object, predicate)?;
            Ok(Some(Value::U32(count)))
        }
        Expr::SumAttr { object, attr } => Ok(Some(Value::F64(sum_attr(selected, object, attr)?))),
        Expr::LeadingAttr { object, attr } => {
            let Some(value) = leading_value(selected, object, attr)? else {
                return Ok(None);
            };
            Ok(Some(match value {
                NumericValue::F64(value) => Value::F64(value),
                NumericValue::I64(value) => Value::I64(value),
                NumericValue::U64(value) => Value::I64(i64::try_from(value).map_err(|error| {
                    InterpretError::NumericConversion(format!(
                        "leading({object}).{attr} cannot fit into i64: {error}"
                    ))
                })?),
            }))
        }
        Expr::Attr { object, attr } => {
            let Some(candidate) = derived_object(derived, object)? else {
                return Ok(None);
            };
            match attr.as_str() {
                "mass" => Ok(Some(Value::F64(candidate.mass))),
                "pt" => Ok(Some(Value::F64(candidate.pt))),
                "min_delta_r" | "dR" | "dr" => Ok(Some(Value::F64(candidate.min_delta_r))),
                other => Err(InterpretError::InvalidExpression(format!(
                    "derived object `{object}` has no interpreted attribute `{other}`"
                ))),
            }
        }
        Expr::All { object, predicate } => Ok(Some(Value::Bool(collection_all(
            selected, derived, object, predicate,
        )?))),
        Expr::Any { object, predicate } => Ok(Some(Value::Bool(collection_any(
            selected, derived, object, predicate,
        )?))),
        Expr::EitherPairPt {
            left,
            right,
            leading,
            subleading,
        } => Ok(Some(Value::Bool(either_pair_pt(
            selected,
            left,
            right,
            leading.value,
            subleading.value,
        )?))),
        Expr::ClosestMass {
            left,
            right,
            target,
        } => Ok(Some(Value::F64(ordered_mass(
            derived,
            left,
            right,
            target.value,
            true,
        )?))),
        Expr::OtherMass {
            left,
            right,
            target,
        } => Ok(Some(Value::F64(ordered_mass(
            derived,
            left,
            right,
            target.value,
            false,
        )?))),
        other => Err(InterpretError::Unsupported(format!(
            "output expression `{other}` is not supported by the interpreter"
        ))),
    }
}

fn eval_numeric_expr(
    expr: &Expr,
    selected: &SelectedObjects,
    derived: &DerivedObjects,
    current: Option<(&str, &SelectedObject)>,
) -> Result<NumericValue> {
    match expr {
        Expr::Attr { object, attr } => {
            if let Some((current_object, selected_object)) = current {
                if object != current_object {
                    return Err(InterpretError::Unsupported(format!(
                        "expression `{expr}` references `{object}` outside the current object `{current_object}`"
                    )));
                }
                return selected_object
                    .leading_values
                    .get(attr)
                    .copied()
                    .ok_or_else(|| {
                        InterpretError::InvalidExpression(format!(
                            "attribute `{attr}` was not materialized for `{object}`"
                        ))
                    });
            }
            let candidate = derived_object(derived, object)?.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "derived object `{object}` has no selected candidate"
                ))
            })?;
            match attr.as_str() {
                "mass" => Ok(NumericValue::F64(candidate.mass)),
                "pt" => Ok(NumericValue::F64(candidate.pt)),
                "min_delta_r" | "dR" | "dr" => Ok(NumericValue::F64(candidate.min_delta_r)),
                other => Err(InterpretError::InvalidExpression(format!(
                    "derived object `{object}` has no interpreted attribute `{other}`"
                ))),
            }
        }
        Expr::Literal(value) => Ok(NumericValue::F64(*value)),
        Expr::Binary { op, lhs, rhs } => {
            let lhs = eval_numeric_expr(lhs, selected, derived, current)?.as_f64();
            let rhs = eval_numeric_expr(rhs, selected, derived, current)?.as_f64();
            Ok(NumericValue::F64(eval_arithmetic(*op, lhs, rhs)))
        }
        Expr::Abs(inner) => Ok(eval_numeric_expr(inner, selected, derived, current)?.abs()),
        Expr::Sqrt(inner) => Ok(NumericValue::F64(
            eval_numeric_expr(inner, selected, derived, current)?
                .as_f64()
                .sqrt(),
        )),
        Expr::Count(object) => {
            let count = selected
                .get(object)
                .ok_or_else(|| InterpretError::MissingObject(object.clone()))?
                .len();
            Ok(NumericValue::U64(count as u64))
        }
        Expr::CountWhere { object, predicate } => Ok(NumericValue::U64(u64::from(count_where(
            selected, derived, object, predicate,
        )?))),
        Expr::SumAttr { object, attr } => Ok(NumericValue::F64(sum_attr(selected, object, attr)?)),
        Expr::All { object, predicate } => Ok(NumericValue::U64(
            if collection_all(selected, derived, object, predicate)? {
                1
            } else {
                0
            },
        )),
        Expr::Any { object, predicate } => Ok(NumericValue::U64(
            if collection_any(selected, derived, object, predicate)? {
                1
            } else {
                0
            },
        )),
        Expr::EitherPairPt {
            left,
            right,
            leading,
            subleading,
        } => Ok(NumericValue::U64(
            if either_pair_pt(selected, left, right, leading.value, subleading.value)? {
                1
            } else {
                0
            },
        )),
        Expr::ClosestMass {
            left,
            right,
            target,
        } => Ok(NumericValue::F64(ordered_mass(
            derived,
            left,
            right,
            target.value,
            true,
        )?)),
        Expr::OtherMass {
            left,
            right,
            target,
        } => Ok(NumericValue::F64(ordered_mass(
            derived,
            left,
            right,
            target.value,
            false,
        )?)),
        Expr::LeadingAttr { object, attr } => {
            leading_value(selected, object, attr)?.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "`leading({object}).{attr}` has no selected object"
                ))
            })
        }
        Expr::PairDeltaR
        | Expr::PairLeadingPt
        | Expr::PairSubleadingPt
        | Expr::CandidateMinDeltaR
        | Expr::CandidateLeadingPt
        | Expr::CandidateSubleadingPt => Err(InterpretError::InvalidExpression(format!(
            "filter-only expression `{expr}` is not valid here"
        ))),
    }
}

fn either_pair_pt(
    selected: &SelectedObjects,
    left: &str,
    right: &str,
    leading: f64,
    subleading: f64,
) -> Result<bool> {
    Ok(pair_pt(selected, left, leading, subleading)?
        || pair_pt(selected, right, leading, subleading)?)
}

fn pair_pt(
    selected: &SelectedObjects,
    object: &str,
    leading: f64,
    subleading: f64,
) -> Result<bool> {
    let objects = selected
        .get(object)
        .ok_or_else(|| InterpretError::MissingObject(object.to_string()))?;
    let mut pts = objects
        .iter()
        .map(|selected_object| attr_f64(selected_object, "pt"))
        .collect::<Vec<_>>();
    pts.sort_by(|left, right| right.total_cmp(left));
    Ok(
        pts.first().is_some_and(|pt| *pt > leading)
            && pts.get(1).is_some_and(|pt| *pt > subleading),
    )
}

fn ordered_mass(
    derived: &DerivedObjects,
    left: &str,
    right: &str,
    target: f64,
    closest: bool,
) -> Result<f64> {
    let left_mass = derived_object(derived, left)?
        .ok_or_else(|| {
            InterpretError::InvalidExpression(format!("derived object `{left}` has no candidate"))
        })?
        .mass;
    let right_mass = derived_object(derived, right)?
        .ok_or_else(|| {
            InterpretError::InvalidExpression(format!("derived object `{right}` has no candidate"))
        })?
        .mass;
    let left_is_closest = (left_mass - target).abs() < (right_mass - target).abs();
    Ok(match (closest, left_is_closest) {
        (true, true) | (false, false) => left_mass,
        (true, false) | (false, true) => right_mass,
    })
}

fn leading_value(
    selected: &SelectedObjects,
    object: &str,
    attr: &str,
) -> Result<Option<NumericValue>> {
    let objects = selected
        .get(object)
        .ok_or_else(|| InterpretError::MissingObject(object.to_string()))?;

    Ok(objects
        .iter()
        .filter_map(|selected_object| {
            selected_object
                .leading_values
                .get(attr)
                .copied()
                .map(|value| (selected_object.source_index, value))
        })
        .max_by(|(_, left), (_, right)| {
            left.as_f64()
                .partial_cmp(&right.as_f64())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, value)| value))
}

fn count_where(
    selected: &SelectedObjects,
    derived: &DerivedObjects,
    object: &str,
    predicate: &Cut,
) -> Result<u32> {
    let objects = selected
        .get(object)
        .ok_or_else(|| InterpretError::MissingObject(object.to_string()))?;
    let mut count = 0_u32;
    for selected_object in objects {
        if eval_collection_predicate(selected, derived, object, selected_object, predicate)? {
            count = count.checked_add(1).ok_or_else(|| {
                InterpretError::NumericConversion(format!("count({object}, ...) overflowed u32"))
            })?;
        }
    }
    Ok(count)
}

fn sum_attr(selected: &SelectedObjects, object: &str, attr: &str) -> Result<f64> {
    let objects = selected
        .get(object)
        .ok_or_else(|| InterpretError::MissingObject(object.to_string()))?;
    Ok(objects
        .iter()
        .map(|selected_object| {
            selected_object
                .leading_values
                .get(attr)
                .copied()
                .map(NumericValue::as_f64)
                .ok_or_else(|| {
                    InterpretError::InvalidExpression(format!(
                        "attribute `{attr}` was not materialized for `{object}`"
                    ))
                })
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum())
}

fn collection_all(
    selected: &SelectedObjects,
    derived: &DerivedObjects,
    object: &str,
    predicate: &Cut,
) -> Result<bool> {
    let objects = selected
        .get(object)
        .ok_or_else(|| InterpretError::MissingObject(object.to_string()))?;
    for selected_object in objects {
        if !eval_collection_predicate(selected, derived, object, selected_object, predicate)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn collection_any(
    selected: &SelectedObjects,
    derived: &DerivedObjects,
    object: &str,
    predicate: &Cut,
) -> Result<bool> {
    let objects = selected
        .get(object)
        .ok_or_else(|| InterpretError::MissingObject(object.to_string()))?;
    for selected_object in objects {
        if eval_collection_predicate(selected, derived, object, selected_object, predicate)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn eval_collection_predicate(
    selected: &SelectedObjects,
    derived: &DerivedObjects,
    object: &str,
    selected_object: &SelectedObject,
    predicate: &Cut,
) -> Result<bool> {
    let lhs = eval_numeric_expr(
        &predicate.lhs,
        selected,
        derived,
        Some((object, selected_object)),
    )?;
    Ok(compare(lhs.as_f64(), predicate.op, predicate.rhs.value))
}

fn eval_arithmetic(op: ArithOp, lhs: f64, rhs: f64) -> f64 {
    match op {
        ArithOp::Add => lhs + rhs,
        ArithOp::Sub => lhs - rhs,
        ArithOp::Mul => lhs * rhs,
        ArithOp::Div => lhs / rhs,
        ArithOp::Pow => lhs.powf(rhs),
    }
}

fn compare(lhs: f64, op: CmpOp, rhs: f64) -> bool {
    match op {
        CmpOp::Gt => lhs > rhs,
        CmpOp::Ge => lhs >= rhs,
        CmpOp::Lt => lhs < rhs,
        CmpOp::Le => lhs <= rhs,
        CmpOp::Eq => lhs == rhs,
        CmpOp::Ne => lhs != rhs,
    }
}

#[cfg(test)]
mod tests {
    use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
    use nano_producers::{MuonProducer, MuonSkimRow};

    use super::*;
    use crate::{validate, AnalysisSpec, Catalogue};

    const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
    const MUON_SPEC_TOML: &str = include_str!("../examples/muon.toml");

    #[test]
    fn interpret_muon_plan_matches_handwritten_muon_producer() {
        let plan = muon_plan();
        let events = synthetic_events();

        let interpreted = events
            .iter()
            .map(|event| {
                interpret(&plan, event)
                    .expect("interpret event")
                    .map(row_to_muon)
            })
            .collect::<Vec<_>>();
        let handwritten = events
            .iter()
            .map(|event| MuonProducer::analyze(event).expect("analyze event"))
            .collect::<Vec<_>>();

        assert_eq!(interpreted, handwritten);
    }

    #[test]
    fn missing_derived_requirement_rejects_event_without_error() {
        let plan = derived_absence_plan(
            r#"
[analysis]
name = "missing_derived_requirement"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = []

[derived.z1]
kind = "pair"
object = "good_muon"
constraints = ["opposite_charge"]
selection = "leading_pt"

[derived.z2]
kind = "pair"
object = "good_muon"
constraints = ["opposite_charge"]
selection = "leading_pt"
exclude = ["z1"]

[regions.signal]
require = ["z2.mass > 5 GeV"]

[[outputs]]
name = "z2_mass"
expr = "z2.mass"
"#,
        );
        let event = two_muon_event();

        assert_eq!(interpret(&plan, &event).expect("interpret"), None);
    }

    #[test]
    fn missing_derived_histogram_value_rejects_event_without_error() {
        let plan = derived_absence_plan(
            r#"
[analysis]
name = "missing_derived_histogram"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = []

[derived.z1]
kind = "pair"
object = "good_muon"
constraints = ["opposite_charge"]
selection = "leading_pt"

[derived.z2]
kind = "pair"
object = "good_muon"
constraints = ["opposite_charge"]
selection = "leading_pt"
exclude = ["z1"]

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_muon"
expr = "count(good_muon)"

[[histogram]]
name = "z2_mass"
expr = "z2.mass"
bins = 10
range = [0.0, 100.0]
"#,
        );
        let event = two_muon_event();
        let mut histograms = InterpretedHistograms::new(&plan);

        assert_eq!(
            interpret_and_fill(&plan, &event, &mut histograms).expect("interpret and fill"),
            None
        );
    }

    #[test]
    fn interpret_rejects_non_mock_model_provider() {
        let spec = AnalysisSpec::from_toml_str(
            r#"
[analysis]
name = "remote_model"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = ["pt > 30 GeV"]

[[model]]
name = "muon_tagger"
inputs = ["Muon_pt", "Muon_eta", "Muon_phi"]
output = "Muon_topscore"
batch = "Muon"

[model.provider]
kind = "remote"
endpoint = "http://127.0.0.1:8000"

[regions.signal]
require = ["count(good_muon) >= 1"]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"
"#,
        )
        .expect("parse remote model spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        let plan = validate(&spec, &catalogue).expect("validate remote model spec");

        assert_eq!(
            interpret(&plan, &two_muon_event()).expect_err("remote provider should stay deferred"),
            InterpretError::Unsupported(
                "model `muon_tagger` provider `remote` is unsupported in interpreter; only mock provider is interpreted"
                    .to_string()
            )
        );
    }

    fn muon_plan() -> ResolvedPlan {
        let spec = AnalysisSpec::from_toml_str(MUON_SPEC_TOML).expect("parse muon spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        validate(&spec, &catalogue).expect("validate muon spec")
    }

    fn derived_absence_plan(input: &str) -> ResolvedPlan {
        let spec = AnalysisSpec::from_toml_str(input).expect("parse derived absence spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        validate(&spec, &catalogue).expect("validate derived absence spec")
    }

    fn two_muon_event() -> Event {
        let schema = BranchSchema::new([
            BranchSpec::new("nMuon", BranchType::U32),
            BranchSpec::new("Muon_charge", BranchType::VecI32),
            BranchSpec::new("Muon_eta", BranchType::VecF32),
            BranchSpec::new("Muon_mass", BranchType::VecF32),
            BranchSpec::new("Muon_phi", BranchType::VecF32),
            BranchSpec::new("Muon_pt", BranchType::VecF32),
        ])
        .expect("schema");
        Event::from_columns(
            schema,
            [
                ("nMuon", BranchColumn::U32(vec![2])),
                ("Muon_charge", BranchColumn::VecI32(vec![vec![1, -1]])),
                ("Muon_eta", BranchColumn::VecF32(vec![vec![0.1, -0.2]])),
                ("Muon_mass", BranchColumn::VecF32(vec![vec![0.105, 0.105]])),
                ("Muon_phi", BranchColumn::VecF32(vec![vec![0.3, -0.4]])),
                ("Muon_pt", BranchColumn::VecF32(vec![vec![40.0, 35.0]])),
            ],
            0,
        )
        .expect("event")
    }

    fn synthetic_events() -> Vec<Event> {
        let schema = BranchSchema::new([
            BranchSpec::new("nMuon", BranchType::U32),
            BranchSpec::new("Muon_eta", BranchType::VecF32),
            BranchSpec::new("Muon_pt", BranchType::VecF32),
        ])
        .expect("schema");
        (0..5)
            .map(|entry| {
                Event::from_columns(
                    schema.clone(),
                    [
                        ("nMuon", BranchColumn::U32(vec![2, 1, 2, 0, 1])),
                        (
                            "Muon_eta",
                            BranchColumn::VecF32(vec![
                                vec![0.1, 0.2],
                                vec![0.0],
                                vec![2.39, -2.0],
                                vec![],
                                vec![2.39],
                            ]),
                        ),
                        (
                            "Muon_pt",
                            BranchColumn::VecF32(vec![
                                vec![31.0, 10.0],
                                vec![29.9],
                                vec![45.0, 35.0],
                                vec![],
                                vec![60.0],
                            ]),
                        ),
                    ],
                    entry,
                )
                .expect("event")
            })
            .collect()
    }

    fn row_to_muon(row: OutputRow) -> MuonSkimRow {
        let n_good_muon = match row.get("n_good_muon").expect("n_good_muon") {
            Value::U32(value) => value,
            value => panic!("unexpected n_good_muon value {value:?}"),
        };
        let lead_muon_pt = match row.get("lead_muon_pt").expect("lead_muon_pt") {
            Value::F64(value) => value as f32,
            value => panic!("unexpected lead_muon_pt value {value:?}"),
        };
        MuonSkimRow {
            n_good_muon,
            lead_muon_pt,
        }
    }
}
