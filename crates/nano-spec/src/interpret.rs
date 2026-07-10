//! Runtime interpreter for the validated semantic IR.
//!
//! This is the dynamic counterpart to [`crate::codegen`]: it executes the same
//! object cuts, region requirements, and output expressions directly over an
//! event instead of requiring a compiled producer.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use nano_analysis::{EventWeight, HistSet1D};
use nano_core::{BranchType, Event, ObjectView};
use nano_corrections::Value as CorrectionValue;
use nano_inference::{mock_scores, InferRequest, Tensor, TensorData};

use crate::kir::{
    Block, ForEachAxis, KirObject, KirObjectCorrection, KirObjectCorrectionPayload, KirProgram,
    KirShapeCorrection, KirShapeCorrectionPayload, Rvalue, Stmt, ValueId,
};
use crate::{
    ArithOp, CmpOp, Cut, DerivedObjectDef, DerivedSource, Expr, ModelDef, ModelProviderKind,
    ObjectCandidateDef, ObjectPairDef, PairConstituentRank, PairConstraint, PairSelection,
    ResolvedPlan, SystematicDef,
};

/// One typed output cell produced by the interpreter.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    F64(f64),
    I64(i64),
    U32(u32),
    U64(u64),
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

    pub fn iter(&self) -> impl Iterator<Item = (&String, &HistSet1D<String>)> {
        self.histograms.iter()
    }

    pub fn scale(&mut self, factor: f64) {
        for histograms in self.histograms.values_mut() {
            histograms.scale(factor);
        }
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
    Correction(String),
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
            Self::Correction(detail) => f.write_str(detail),
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
    p4: SelectedKinematics,
    leading_values: HashMap<String, NumericValue>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SelectedKinematics {
    pt: f64,
    eta: f64,
    phi: f64,
    mass: f64,
}

impl Default for SelectedKinematics {
    fn default() -> Self {
        Self {
            pt: 0.0,
            eta: 0.0,
            phi: 0.0,
            mass: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DerivedObject {
    mass: f64,
    pt: f64,
    eta: f64,
    phi: f64,
    min_delta_r: f64,
    delta_eta: f64,
    delta_phi: f64,
    leading_pt: f64,
    subleading_pt: f64,
    leading_eta: f64,
    subleading_eta: f64,
    leading_phi: f64,
    subleading_phi: f64,
    leading_mass: f64,
    subleading_mass: f64,
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
    mass: NumericValue,
    values: HashMap<String, NumericValue>,
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
        let factor = shape_factor_for_source(program, event, batch, attr, item, systematic)?;
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

fn correction_set_from_path(file: &str) -> Result<Arc<nano_corrections::CorrectionSet>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<nano_corrections::CorrectionSet>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = cache.lock().map_err(|error| {
            InterpretError::Correction(format!("correction cache lock failed: {error}"))
        })?;
        if let Some(set) = guard.get(file) {
            return Ok(Arc::clone(set));
        }
    }

    let loaded = Arc::new(
        nano_corrections::CorrectionSet::from_path(file).map_err(|error| {
            InterpretError::Correction(format!("failed to load `{file}`: {error}"))
        })?,
    );
    let mut guard = cache.lock().map_err(|error| {
        InterpretError::Correction(format!("correction cache lock failed: {error}"))
    })?;
    let entry = guard
        .entry(file.to_string())
        .or_insert_with(|| Arc::clone(&loaded));
    Ok(Arc::clone(entry))
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
    interpreted_systematic_variants_from_parts(
        &plan.spec.systematics,
        &plan.spec.shape_corrections,
        &plan.spec.scale_factor_corrections,
    )
}

fn interpreted_systematic_variants_from_parts(
    systematics: &[SystematicDef],
    shape_corrections: &[impl NamedSystematicCorrection],
    scale_factor_corrections: &[impl NamedSystematicCorrection],
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
    for correction in scale_factor_corrections
        .iter()
        .filter(|correction| correction.has_systematic())
    {
        variants.push(interpreted_variant_name(correction.name(), "Up"));
        variants.push(interpreted_variant_name(correction.name(), "Down"));
    }
    variants
}

trait NamedSystematicCorrection {
    fn name(&self) -> &str;
    fn has_systematic(&self) -> bool {
        true
    }
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

impl NamedSystematicCorrection for crate::ScaleFactorCorrectionDef {
    fn name(&self) -> &str {
        &self.name
    }

    fn has_systematic(&self) -> bool {
        self.systematic.is_some()
    }
}

impl NamedSystematicCorrection for crate::kir::KirScaleFactorCorrection {
    fn name(&self) -> &str {
        &self.name
    }

    fn has_systematic(&self) -> bool {
        self.systematic.is_some()
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
        let weight = self.weight_for(&systematic)?;
        for histogram in &self.program.histograms {
            if expr_has_missing_value(&histogram.def.expr, &self.selected, &self.derived)? {
                return Ok(false);
            }
            let value = eval_numeric_expr(
                &histogram.def.expr,
                &self.selected,
                &self.derived,
                None,
                Some(self.event),
            )?
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
                    &self.selected,
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
                if let Expr::EventScalar(branch) = &requirement.lhs {
                    let branch_type = self
                        .event
                        .schema()
                        .find(branch)
                        .map(|info| info.branch_type)
                        .ok_or_else(|| InterpretError::MissingBranch(branch.clone()))?;
                    if branch_type == BranchType::Bool {
                        let lhs = self.event.scalar::<bool>(branch)?;
                        return Ok(RuntimeValue::Bool(compare_bool(
                            lhs,
                            requirement.op,
                            requirement.rhs.value,
                        )?));
                    }
                }
                if expr_has_missing_value(&requirement.lhs, &self.selected, &self.derived)? {
                    return Ok(RuntimeValue::Bool(false));
                }
                let lhs = eval_numeric_expr(
                    &requirement.lhs,
                    &self.selected,
                    &self.derived,
                    None,
                    Some(self.event),
                )?;
                Ok(RuntimeValue::Bool(compare(
                    lhs.as_f64(),
                    requirement.op,
                    requirement.rhs.value,
                )))
            }
            Rvalue::LumiMask { mask } => {
                if self.event.is_mc() {
                    return Ok(RuntimeValue::Bool(true));
                }
                let run = self.event.scalar::<u32>("run")?;
                let luminosity_block = self.event.scalar::<u32>("luminosityBlock")?;
                Ok(RuntimeValue::Bool(mask.contains(run, luminosity_block)))
            }
            Rvalue::Output { expr, .. } => Ok(RuntimeValue::Output(eval_output_expr(
                expr,
                &self.selected,
                &self.derived,
                self.event,
            )?)),
            Rvalue::Histogram { histogram } => Ok(RuntimeValue::Histogram(histogram.name.clone())),
            Rvalue::HistogramValue { expr, .. } => {
                if expr_has_missing_value(expr, &self.selected, &self.derived)? {
                    return Ok(RuntimeValue::Output(None));
                }
                Ok(RuntimeValue::Numeric(
                    eval_numeric_expr(expr, &self.selected, &self.derived, None, Some(self.event))?
                        .as_f64(),
                ))
            }
            Rvalue::ScaleFactor { systematic } => {
                let RuntimeValue::Systematic(systematic) = self.value(*systematic)? else {
                    return Err(InterpretError::InvalidExpression(format!(
                        "KIR scale factor expected systematic value {systematic:?}"
                    )));
                };
                Ok(RuntimeValue::Numeric(
                    self.scale_factor_weight_for(&systematic)?,
                ))
            }
            Rvalue::Weight {
                systematic,
                scale_factor: _,
            } => {
                let RuntimeValue::Systematic(systematic) = self.value(*systematic)? else {
                    return Err(InterpretError::InvalidExpression(format!(
                        "KIR weight expected systematic value {systematic:?}"
                    )));
                };
                Ok(RuntimeValue::Weight(self.weight_for(&systematic)?))
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
            &self.program.scale_factor_corrections,
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

    fn weight_for(&self, systematic: &str) -> Result<EventWeight> {
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
        Ok(weight.times(self.scale_factor_weight_for(systematic)?))
    }

    fn scale_factor_weight_for(&self, systematic: &str) -> Result<f64> {
        let mut weight = 1.0_f64;
        for correction in &self.program.scale_factor_corrections {
            weight *= evaluate_scale_factor_correction(
                self.program,
                self.event,
                correction,
                &self.selected,
                systematic,
            )?;
        }
        Ok(weight)
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
    event: &Event,
    object: &KirObject,
    item: &ObjectView<'_>,
    selected: &SelectedObjects,
    systematic: &str,
    model_outputs: &ModelOutputs,
) -> Result<bool> {
    for cut in &object.cuts {
        let lhs = eval_object_numeric_expr(
            program,
            event,
            &object.name,
            &object.source,
            &cut.lhs,
            item,
            selected,
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
    selected: &SelectedObjects,
    systematic: &str,
    model_outputs: &ModelOutputs,
) -> Result<Vec<SelectedObject>> {
    let collection = event.collection(&object.source)?;
    let mut objects = Vec::new();

    for item in collection.iter() {
        let mut leading_values = HashMap::new();
        if passes_object_cuts(
            program,
            event,
            object,
            item,
            selected,
            systematic,
            model_outputs,
        )? {
            for attr in leading_attrs_for_object(program, &object.name) {
                let value = read_object_attr(
                    program,
                    event,
                    &object.name,
                    &object.source,
                    item,
                    &attr,
                    systematic,
                    model_outputs,
                )?;
                leading_values.insert(attr, value);
            }
            let kinematic_components = object_required_kinematic_components(program, &object.name);
            let p4 = if !kinematic_components.is_empty() {
                selected_kinematics(
                    program,
                    event,
                    object,
                    item,
                    systematic,
                    model_outputs,
                    &kinematic_components,
                )?
            } else {
                SelectedKinematics::default()
            };
            objects.push(SelectedObject {
                source_index: item.index(),
                p4,
                leading_values,
            });
        }
    }

    Ok(objects)
}

fn object_required_kinematic_components(
    program: &KirProgram,
    object_name: &str,
) -> BTreeSet<&'static str> {
    let mut components = BTreeSet::new();
    for derived in &program.derived_objects {
        match &derived.def.source {
            DerivedSource::Pair(pair) if pair.object == object_name => {
                components.extend(["pt", "eta", "phi", "mass"]);
            }
            DerivedSource::Candidate(candidate)
                if candidate.items.iter().any(|item| item == object_name) =>
            {
                components.extend(["pt", "eta", "phi", "mass"]);
            }
            _ => {}
        }
    }
    for requirement in program
        .regions
        .iter()
        .flat_map(|region| region.requirements.iter())
    {
        collect_expr_required_kinematic_components(&requirement.lhs, object_name, &mut components);
    }
    for output in &program.outputs {
        collect_expr_required_kinematic_components(&output.expr, object_name, &mut components);
    }
    components
}

fn collect_expr_required_kinematic_components(
    expr: &Expr,
    object_name: &str,
    components: &mut BTreeSet<&'static str>,
) {
    match expr {
        Expr::EitherPairPt { left, right, .. } => {
            if left == object_name || right == object_name {
                components.insert("pt");
            }
        }
        Expr::LegacyLeptonRpt {
            muons, electrons, ..
        } => {
            if muons == object_name || electrons == object_name {
                components.insert("pt");
            }
        }
        Expr::LeadingType1Mt { object, .. } => {
            if object == object_name {
                components.insert("pt");
                components.insert("phi");
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr_required_kinematic_components(lhs, object_name, components);
            collect_expr_required_kinematic_components(rhs, object_name, components);
        }
        Expr::Abs(inner) | Expr::Sqrt(inner) => {
            collect_expr_required_kinematic_components(inner, object_name, components);
        }
        Expr::CountWhere { predicate, .. }
        | Expr::All { predicate, .. }
        | Expr::Any { predicate, .. } => {
            collect_expr_required_kinematic_components(&predicate.lhs, object_name, components);
        }
        _ => {}
    }
}

fn selected_kinematics(
    program: &KirProgram,
    event: &Event,
    object: &KirObject,
    item: &ObjectView<'_>,
    systematic: &str,
    model_outputs: &ModelOutputs,
    components: &BTreeSet<&'static str>,
) -> Result<SelectedKinematics> {
    let read = |attr: &str| {
        read_object_attr(
            program,
            event,
            &object.name,
            &object.source,
            item,
            attr,
            systematic,
            model_outputs,
        )
        .map(NumericValue::as_f64)
    };
    Ok(SelectedKinematics {
        pt: if components.contains("pt") {
            read(&object.kinematics.pt)?
        } else {
            0.0
        },
        eta: if components.contains("eta") {
            read(&object.kinematics.eta)?
        } else {
            0.0
        },
        phi: if components.contains("phi") {
            read(&object.kinematics.phi)?
        } else {
            0.0
        },
        mass: if components.contains("mass") {
            read(&object.kinematics.mass)?
        } else {
            0.0
        },
    })
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
        order.sort_by(|&left, &right| objects[right].p4.pt.total_cmp(&objects[left].p4.pt));
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
            constituents.push(constituent_from_selected(item, object));
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
        let geometry = constituent_geometry(&constituents);
        let candidate = DerivedObject {
            mass,
            pt,
            eta: vector_eta(px, py, pz),
            phi: vector_phi(px, py),
            min_delta_r: geometry.min_delta_r,
            delta_eta: geometry.delta_eta,
            delta_phi: geometry.delta_phi,
            leading_pt: geometry.leading_pt,
            subleading_pt: geometry.subleading_pt,
            leading_eta: geometry.leading_eta,
            subleading_eta: geometry.subleading_eta,
            leading_phi: geometry.leading_phi,
            subleading_phi: geometry.subleading_phi,
            leading_mass: geometry.leading_mass,
            subleading_mass: geometry.subleading_mass,
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
            first.p4.eta,
            first.p4.phi,
            second.p4.eta,
            second.p4.phi,
        )),
        Expr::PairLeadingPt => Ok(first.p4.pt.max(second.p4.pt)),
        Expr::PairSubleadingPt => Ok(first.p4.pt.min(second.p4.pt)),
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
        constituents.push(constituent_from_selected(object, item));
    }
    let (mass, pt) = mass_pt(energy, px, py, pz);
    let geometry = constituent_geometry(&constituents);
    Ok(DerivedObject {
        mass,
        pt,
        eta: vector_eta(px, py, pz),
        phi: vector_phi(px, py),
        min_delta_r: geometry.min_delta_r,
        delta_eta: geometry.delta_eta,
        delta_phi: geometry.delta_phi,
        leading_pt: geometry.leading_pt,
        subleading_pt: geometry.subleading_pt,
        leading_eta: geometry.leading_eta,
        subleading_eta: geometry.subleading_eta,
        leading_phi: geometry.leading_phi,
        subleading_phi: geometry.subleading_phi,
        leading_mass: geometry.leading_mass,
        subleading_mass: geometry.subleading_mass,
        energy,
        px,
        py,
        pz,
        constituents,
    })
}

fn selected_four_vector(item: &SelectedObject) -> (f64, f64, f64, f64) {
    let pt = item.p4.pt;
    let eta = item.p4.eta;
    let phi = item.p4.phi;
    let mass = item.p4.mass;
    let px = pt * phi.cos();
    let py = pt * phi.sin();
    let pz = pt * eta.sinh();
    let energy = (px * px + py * py + pz * pz + mass * mass).sqrt();
    (energy, px, py, pz)
}

fn constituent_from_selected(object_name: &str, item: &SelectedObject) -> Constituent {
    Constituent {
        object: object_name.to_string(),
        index: item.source_index,
        pt: NumericValue::F64(item.p4.pt),
        eta: NumericValue::F64(item.p4.eta),
        phi: NumericValue::F64(item.p4.phi),
        mass: NumericValue::F64(item.p4.mass),
        values: item.leading_values.clone(),
    }
}

fn attr_f64(item: &SelectedObject, attr: &str) -> f64 {
    item.leading_values
        .get(attr)
        .copied()
        .map(NumericValue::as_f64)
        .unwrap_or(0.0)
}

fn required_attr_f64(item: &SelectedObject, object: &str, attr: &str) -> Result<f64> {
    item.leading_values
        .get(attr)
        .copied()
        .map(NumericValue::as_f64)
        .ok_or_else(|| {
            InterpretError::InvalidExpression(format!(
                "attribute `{attr}` was not materialized for `{object}`"
            ))
        })
}

fn mass_pt(energy: f64, px: f64, py: f64, pz: f64) -> (f64, f64) {
    (
        (energy * energy - px * px - py * py - pz * pz)
            .max(0.0)
            .sqrt(),
        (px * px + py * py).sqrt(),
    )
}

fn vector_eta(px: f64, py: f64, pz: f64) -> f64 {
    let pt = px.hypot(py);
    let momentum = pt.hypot(pz);
    let denominator = momentum - pz;
    if denominator <= 0.0 {
        0.0
    } else {
        0.5 * ((momentum + pz) / denominator).ln()
    }
}

fn vector_phi(px: f64, py: f64) -> f64 {
    py.atan2(px)
}

fn delta_r(left_eta: f64, left_phi: f64, right_eta: f64, right_phi: f64) -> f64 {
    let deta = left_eta - right_eta;
    let dphi = delta_phi(left_phi, right_phi);
    (deta * deta + dphi * dphi).sqrt()
}

fn delta_phi(left_phi: f64, right_phi: f64) -> f64 {
    let mut dphi = left_phi - right_phi;
    while dphi > std::f64::consts::PI {
        dphi -= 2.0 * std::f64::consts::PI;
    }
    while dphi <= -std::f64::consts::PI {
        dphi += 2.0 * std::f64::consts::PI;
    }
    dphi.abs()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ConstituentGeometry {
    min_delta_r: f64,
    delta_eta: f64,
    delta_phi: f64,
    leading_pt: f64,
    subleading_pt: f64,
    leading_eta: f64,
    subleading_eta: f64,
    leading_phi: f64,
    subleading_phi: f64,
    leading_mass: f64,
    subleading_mass: f64,
}

fn constituent_geometry(constituents: &[Constituent]) -> ConstituentGeometry {
    let mut ordered = constituents.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| right.pt.as_f64().total_cmp(&left.pt.as_f64()));
    let leading = ordered.first().copied();
    let subleading = ordered.get(1).copied();
    let leading_pt = leading.map(|item| item.pt.as_f64()).unwrap_or(0.0);
    let subleading_pt = subleading.map(|item| item.pt.as_f64()).unwrap_or(0.0);
    let leading_eta = leading.map(|item| item.eta.as_f64()).unwrap_or(0.0);
    let subleading_eta = subleading.map(|item| item.eta.as_f64()).unwrap_or(0.0);
    let leading_phi = leading.map(|item| item.phi.as_f64()).unwrap_or(0.0);
    let subleading_phi = subleading.map(|item| item.phi.as_f64()).unwrap_or(0.0);
    let leading_mass = leading.map(|item| item.mass.as_f64()).unwrap_or(0.0);
    let subleading_mass = subleading.map(|item| item.mass.as_f64()).unwrap_or(0.0);

    ConstituentGeometry {
        min_delta_r: candidate_min_delta_r(constituents),
        delta_eta: leading
            .zip(subleading)
            .map(|(left, right)| (left.eta.as_f64() - right.eta.as_f64()).abs())
            .unwrap_or(0.0),
        delta_phi: leading
            .zip(subleading)
            .map(|(left, right)| delta_phi(left.phi.as_f64(), right.phi.as_f64()))
            .unwrap_or(0.0),
        leading_pt,
        subleading_pt,
        leading_eta,
        subleading_eta,
        leading_phi,
        subleading_phi,
        leading_mass,
        subleading_mass,
    }
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
    for object in &program.objects {
        for cut in &object.cuts {
            collect_selected_attrs(&cut.lhs, object_name, &mut attrs);
        }
    }
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
            collect_pair_constituent_attrs(program, &requirement.lhs, object_name, &mut attrs);
        }
    }
    for output in &program.outputs {
        collect_selected_attrs(&output.expr, object_name, &mut attrs);
        collect_pair_constituent_attrs(program, &output.expr, object_name, &mut attrs);
    }
    for correction in &program.scale_factor_corrections {
        if correction.collection == object_name {
            for input in &correction.inputs {
                if let crate::ScaleFactorInputSource::From(source) = &input.source {
                    let object = program
                        .objects
                        .iter()
                        .find(|object| object.name == object_name)
                        .expect("object exists");
                    let branch = format!("{}_{}", object.source, source);
                    if program.read_branches.iter().any(|spec| spec.name == branch) {
                        push_attr(&mut attrs, source);
                    }
                }
            }
        }
    }
    for derived in &program.derived_objects {
        match &derived.def.source {
            DerivedSource::Pair(pair) if pair.object == object_name => {
                for attr in kinematic_attrs_for_object(program, object_name) {
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
                for attr in kinematic_attrs_for_object(program, object_name) {
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

fn kinematic_attrs_for_object<'a>(program: &'a KirProgram, object_name: &str) -> Vec<&'a str> {
    program
        .objects
        .iter()
        .find(|object| object.name == object_name)
        .map(|object| {
            object
                .kinematics
                .component_attrs()
                .into_iter()
                .map(|(_, attr)| attr)
                .collect()
        })
        .unwrap_or_default()
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
        Expr::IndexNotIn { object, attr } if object == object_name => push_attr(attrs, attr),
        Expr::JetVetoMapRun2024 { object } if object == object_name => {
            for attr in crate::JET_VETO_MAP_RUN2024_ATTRS {
                push_attr(attrs, attr);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_selected_attrs(lhs, object_name, attrs);
            collect_selected_attrs(rhs, object_name, attrs);
        }
        Expr::Abs(inner) | Expr::Sqrt(inner) => collect_selected_attrs(inner, object_name, attrs),
        Expr::LegacyLeptonRpt { .. } => {}
        Expr::MetType1Pt {
            jets,
            nominal_pt,
            shifted_pt,
            ..
        }
        | Expr::MetType1Phi {
            jets,
            nominal_pt,
            shifted_pt,
            ..
        }
        | Expr::LeadingType1Mt {
            jets,
            nominal_pt,
            shifted_pt,
            ..
        } if jets == object_name => {
            for attr in crate::MET_TYPE1_JET_ATTRS {
                push_attr(attrs, attr);
            }
            push_attr(attrs, nominal_pt);
            push_attr(attrs, shifted_pt);
        }
        Expr::LeadingType1Mt { object, .. } if object == object_name => {
            push_attr(attrs, "pt");
            push_attr(attrs, "phi");
        }
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

fn collect_pair_constituent_attrs(
    program: &KirProgram,
    expr: &Expr,
    object_name: &str,
    attrs: &mut Vec<String>,
) {
    match expr {
        Expr::PairConstituentAttr { pair, attr, .. } => {
            let Some(derived) = program
                .derived_objects
                .iter()
                .find(|derived| derived.name == *pair)
            else {
                return;
            };
            if let DerivedSource::Pair(pair_def) = &derived.def.source {
                if pair_def.object == object_name {
                    push_attr(attrs, attr);
                }
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_pair_constituent_attrs(program, lhs, object_name, attrs);
            collect_pair_constituent_attrs(program, rhs, object_name, attrs);
        }
        Expr::Abs(inner) | Expr::Sqrt(inner) => {
            collect_pair_constituent_attrs(program, inner, object_name, attrs);
        }
        _ => {}
    }
}

fn collect_pair_filter_attrs(expr: &Expr, _attrs: &mut Vec<String>) {
    match expr {
        Expr::PairDeltaR | Expr::PairLeadingPt | Expr::PairSubleadingPt => {}
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

#[allow(clippy::too_many_arguments)]
fn eval_object_numeric_expr(
    program: &KirProgram,
    event: &Event,
    current_object: &str,
    source: &str,
    expr: &Expr,
    item: &ObjectView<'_>,
    selected: &SelectedObjects,
    systematic: &str,
    model_outputs: &ModelOutputs,
) -> Result<NumericValue> {
    match expr {
        Expr::Attr { object, attr } if object == current_object => {
            read_object_attr(
                program,
                event,
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
        Expr::IndexNotIn { object, attr } => Ok(NumericValue::U64(u64::from(index_not_in(
            selected,
            object,
            attr,
            item.index(),
        )?))),
        Expr::JetIdTightRun2024 { object } if object == current_object => {
            Ok(NumericValue::U64(u64::from(jet_id_tight_run2024(
                program,
                event,
                current_object,
                source,
                item,
                systematic,
                model_outputs,
            )?)))
        }
        Expr::JetIdTightRun2024 { object } => Err(InterpretError::Unsupported(format!(
            "object `{current_object}` cut references jet ID for `{object}`"
        ))),
        Expr::JetVetoMapRun2024 { object } if object == current_object => {
            Ok(NumericValue::F64(jet_veto_map_run2024(
                program,
                event,
                current_object,
                source,
                item,
                systematic,
                model_outputs,
            )?))
        }
        Expr::JetVetoMapRun2024 { object } => Err(InterpretError::Unsupported(format!(
            "object `{current_object}` cut references jet veto map for `{object}`"
        ))),
        Expr::Binary { op, lhs, rhs } => {
            let lhs =
                eval_object_numeric_expr(
                    program,
                    event,
                    current_object,
                    source,
                    lhs,
                    item,
                    selected,
                    systematic,
                    model_outputs,
                )?
                    .as_f64();
            let rhs =
                eval_object_numeric_expr(
                    program,
                    event,
                    current_object,
                    source,
                    rhs,
                    item,
                    selected,
                    systematic,
                    model_outputs,
                )?
                    .as_f64();
            Ok(NumericValue::F64(eval_arithmetic(*op, lhs, rhs)))
        }
        Expr::Abs(inner) => Ok(eval_object_numeric_expr(
            program,
            event,
            current_object,
            source,
            inner,
            item,
            selected,
            systematic,
            model_outputs,
        )?
        .abs()),
        Expr::Sqrt(inner) => Ok(NumericValue::F64(
            eval_object_numeric_expr(
                program,
                event,
                current_object,
                source,
                inner,
                item,
                selected,
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

fn jet_id_tight_run2024(
    program: &KirProgram,
    event: &Event,
    current_object: &str,
    source: &str,
    item: &ObjectView<'_>,
    systematic: &str,
    model_outputs: &ModelOutputs,
) -> Result<bool> {
    let eta = read_object_attr(
        program,
        event,
        current_object,
        source,
        item,
        "eta",
        systematic,
        model_outputs,
    )?
    .as_f64()
    .abs();
    let ch_hef = read_object_attr(
        program,
        event,
        current_object,
        source,
        item,
        "chHEF",
        systematic,
        model_outputs,
    )?
    .as_f64();
    let ne_hef = read_object_attr(
        program,
        event,
        current_object,
        source,
        item,
        "neHEF",
        systematic,
        model_outputs,
    )?
    .as_f64();
    let ne_em_ef = read_object_attr(
        program,
        event,
        current_object,
        source,
        item,
        "neEmEF",
        systematic,
        model_outputs,
    )?
    .as_f64();
    let ch_mult = read_object_attr(
        program,
        event,
        current_object,
        source,
        item,
        "chMultiplicity",
        systematic,
        model_outputs,
    )?
    .as_f64();
    let ne_mult = read_object_attr(
        program,
        event,
        current_object,
        source,
        item,
        "neMultiplicity",
        systematic,
        model_outputs,
    )?
    .as_f64();

    Ok(if eta <= 2.6 {
        ne_hef < 0.99 && ne_em_ef < 0.9 && ch_mult + ne_mult > 1.0 && ch_hef > 0.01 && ch_mult > 0.0
    } else if eta <= 2.7 {
        ne_hef < 0.90 && ne_em_ef < 0.99
    } else if eta < 3.0 {
        ne_hef < 0.99
    } else {
        ne_mult >= 2.0 && ne_em_ef < 0.4
    })
}

fn jet_veto_map_run2024(
    program: &KirProgram,
    event: &Event,
    current_object: &str,
    source: &str,
    item: &ObjectView<'_>,
    systematic: &str,
    model_outputs: &ModelOutputs,
) -> Result<f64> {
    let eta = read_object_attr(
        program,
        event,
        current_object,
        source,
        item,
        "eta",
        systematic,
        model_outputs,
    )?
    .as_f64()
    .clamp(-5.18, 5.18);
    let phi = read_object_attr(
        program,
        event,
        current_object,
        source,
        item,
        "phi",
        systematic,
        model_outputs,
    )?
    .as_f64()
    .clamp(-std::f64::consts::PI, std::f64::consts::PI);
    jet_veto_map_run2024_value(eta, phi)
}

fn jet_veto_map_run2024_value(eta: f64, phi: f64) -> Result<f64> {
    static CORRECTION: OnceLock<std::result::Result<nano_corrections::Correction, String>> =
        OnceLock::new();
    let correction = CORRECTION
        .get_or_init(|| {
            let set = nano_corrections::CorrectionSet::from_path(crate::JET_VETO_MAP_RUN2024_FILE)
                .map_err(|error| error.to_string())?;
            set.correction(crate::JET_VETO_MAP_RUN2024_CORRECTION)
                .cloned()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| {
            InterpretError::Unsupported(format!(
                "Run2024 jet veto map payload is unavailable: {error}"
            ))
        })?;
    correction
        .evaluate(&[
            CorrectionValue::from(crate::JET_VETO_MAP_TYPE),
            CorrectionValue::Real(eta),
            CorrectionValue::Real(phi),
        ])
        .map_err(|error| {
            InterpretError::Unsupported(format!("Run2024 jet veto map evaluation failed: {error}"))
        })
}

fn index_not_in(
    selected: &SelectedObjects,
    object: &str,
    attr: &str,
    source_index: usize,
) -> Result<bool> {
    let objects = selected
        .get(object)
        .ok_or_else(|| InterpretError::MissingObject(object.to_string()))?;
    let index = source_index as f64;
    Ok(objects.iter().all(|selected_object| {
        selected_object
            .leading_values
            .get(attr)
            .copied()
            .map(|value| (value.as_f64() - index).abs() > f64::EPSILON)
            .unwrap_or(true)
    }))
}

#[allow(clippy::too_many_arguments)]
fn read_object_attr(
    program: &KirProgram,
    event: &Event,
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

    if let Some(correction) = program
        .object_corrections
        .iter()
        .find(|correction| correction.collection == collection && correction.attr == attr)
    {
        return evaluate_object_correction(program, event, source, item, correction);
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
            let factor = shape_factor(program, event, collection, source, item, attr, systematic)?;
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

fn evaluate_object_correction(
    program: &KirProgram,
    event: &Event,
    source: &str,
    item: &ObjectView<'_>,
    correction: &KirObjectCorrection,
) -> Result<NumericValue> {
    let cache_attr = format!("__nano_object_correction:{}", correction.name);
    if let Ok(cached) = item.extra::<NumericValue>(&cache_attr) {
        return Ok(*cached);
    }

    match &correction.payload {
        KirObjectCorrectionPayload::JecNominal {
            file,
            correction: payload_name,
            raw_factor_attr,
            inputs,
        } => {
            let source_pt = f64::from(item.get::<f32>(&correction.source_attr)?);
            let raw_factor = f64::from(item.get::<f32>(raw_factor_attr)?);
            let raw_pt = source_pt * (1.0 - raw_factor);
            let set = correction_set_from_path(file)?;
            let payload = set.correction_ref(payload_name).map_err(|error| {
                InterpretError::Correction(format!(
                    "JEC nominal correction `{}` payload lookup failed: {error}",
                    correction.name
                ))
            })?;
            let mut values = Vec::with_capacity(payload.inputs().len());
            for payload_input in payload.inputs() {
                let input = inputs
                    .iter()
                    .find(|input| input.name == payload_input.name)
                    .ok_or_else(|| {
                        InterpretError::Correction(format!(
                            "JEC nominal correction `{}` is missing input `{}`",
                            correction.name, payload_input.name
                        ))
                    })?;
                values.push(correction_input_value(
                    program,
                    event,
                    &correction.collection,
                    source,
                    item,
                    input,
                    Some(("raw_pt", CorrectionValue::Real(raw_pt))),
                )?);
            }
            let factor = payload.evaluate(&set, &values).map_err(|error| {
                InterpretError::Correction(format!(
                    "JEC nominal correction `{}` evaluation failed: {error}",
                    correction.name
                ))
            })?;
            let value = NumericValue::F64(f64::from((raw_pt * factor) as f32));
            item.set(cache_attr, value);
            Ok(value)
        }
        KirObjectCorrectionPayload::JerNominal {
            scale_factor_file,
            scale_factor_correction,
            scale_factor_inputs,
            resolution_file,
            resolution_correction,
            resolution_inputs,
            gen_jet_index_attr,
            gen_jet_pt_branch,
        } => {
            let source_pt = correction_input_real(
                program,
                event,
                &correction.collection,
                source,
                item,
                &correction.source_attr,
            )?;
            let scale_factor = evaluate_object_correction_payload(
                program,
                event,
                &correction.collection,
                source,
                item,
                scale_factor_file,
                scale_factor_correction,
                scale_factor_inputs,
                &correction.name,
                "JER scale-factor",
            )?;
            let pt_resolution = evaluate_object_correction_payload(
                program,
                event,
                &correction.collection,
                source,
                item,
                resolution_file,
                resolution_correction,
                resolution_inputs,
                &correction.name,
                "JER resolution",
            )?;
            let gen_jet_index_branch = format!("{source}_{gen_jet_index_attr}");
            let gen_jet_index_type = branch_type(program, &gen_jet_index_branch)?;
            let gen_jet_index = match object_correction_value(
                item,
                gen_jet_index_attr,
                &gen_jet_index_branch,
                gen_jet_index_type,
            )? {
                CorrectionValue::Int(value) => value,
                other => {
                    return Err(InterpretError::Correction(format!(
                        "JER correction `{}` GenJet index input evaluated to {other:?}, expected int",
                        correction.name
                    )));
                }
            };
            let gen_jet_pts = event.vector_ref::<f32>(gen_jet_pt_branch)?;
            let matched = gen_jet_index >= 0
                && gen_jet_pts
                    .get(gen_jet_index as usize)
                    .is_some_and(|gen_pt| {
                        (source_pt - f64::from(*gen_pt)) / source_pt < 3.0 * pt_resolution
                    });
            let scale = if matched {
                let gen_pt = f64::from(gen_jet_pts[gen_jet_index as usize]);
                let pt_diff_rel = (source_pt - gen_pt) / source_pt;
                (1.0 + (scale_factor - 1.0) * pt_diff_rel).max(0.0)
            } else {
                1.0
            };
            let value = NumericValue::F64(f64::from((source_pt * scale) as f32));
            item.set(cache_attr, value);
            Ok(value)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_object_correction_payload(
    program: &KirProgram,
    event: &Event,
    collection: &str,
    source: &str,
    item: &ObjectView<'_>,
    file: &str,
    payload_name: &str,
    inputs: &[crate::ScaleFactorInputDef],
    correction_name: &str,
    label: &str,
) -> Result<f64> {
    let set = correction_set_from_path(file)?;
    let payload = set.correction_ref(payload_name).map_err(|error| {
        InterpretError::Correction(format!(
            "{label} correction `{correction_name}` payload lookup failed: {error}"
        ))
    })?;
    let mut values = Vec::with_capacity(payload.inputs().len());
    for payload_input in payload.inputs() {
        let input = inputs
            .iter()
            .find(|input| input.name == payload_input.name)
            .ok_or_else(|| {
                InterpretError::Correction(format!(
                    "{label} correction `{correction_name}` is missing input `{}`",
                    payload_input.name
                ))
            })?;
        values.push(correction_input_value(
            program, event, collection, source, item, input, None,
        )?);
    }
    payload.evaluate(&set, &values).map_err(|error| {
        InterpretError::Correction(format!(
            "{label} correction `{correction_name}` evaluation failed: {error}"
        ))
    })
}

fn correction_input_real(
    program: &KirProgram,
    event: &Event,
    collection: &str,
    source: &str,
    item: &ObjectView<'_>,
    attr: &str,
) -> Result<f64> {
    if let Some(correction) = program
        .object_corrections
        .iter()
        .find(|correction| correction.collection == collection && correction.attr == attr)
    {
        return Ok(evaluate_object_correction(program, event, source, item, correction)?.as_f64());
    }
    let branch = format!("{source}_{attr}");
    let branch_type = branch_type(program, &branch)?;
    object_correction_value(item, attr, &branch, branch_type).map(|value| match value {
        CorrectionValue::Real(value) => value,
        CorrectionValue::Int(value) => value as f64,
        CorrectionValue::Str(value) => value.parse::<f64>().unwrap_or(0.0),
    })
}

fn shape_factor(
    program: &KirProgram,
    event: &Event,
    collection: &str,
    source: &str,
    item: &ObjectView<'_>,
    attr: &str,
    systematic: &str,
) -> Result<f64> {
    program
        .shape_corrections
        .iter()
        .find(|correction| correction.collection == collection && correction.attr == attr)
        .map(|correction| {
            shape_correction_factor(program, event, source, item, correction, systematic)
        })
        .unwrap_or(Ok(1.0))
}

fn shape_correction_factor(
    program: &KirProgram,
    event: &Event,
    source: &str,
    item: &ObjectView<'_>,
    correction: &KirShapeCorrection,
    systematic: &str,
) -> Result<f64> {
    match &correction.payload {
        KirShapeCorrectionPayload::Scale { up, down } => Ok(match systematic {
            value if value == interpreted_variant_name(&correction.name, "Up") => *up,
            value if value == interpreted_variant_name(&correction.name, "Down") => *down,
            _ => 1.0,
        }),
        KirShapeCorrectionPayload::Jes { .. } => {
            if systematic == interpreted_variant_name(&correction.name, "Up") {
                Ok(1.0 + evaluate_jes_uncertainty(program, event, source, item, correction)?)
            } else if systematic == interpreted_variant_name(&correction.name, "Down") {
                Ok(1.0 - evaluate_jes_uncertainty(program, event, source, item, correction)?)
            } else {
                Ok(1.0)
            }
        }
    }
}

fn evaluate_jes_uncertainty(
    program: &KirProgram,
    event: &Event,
    source: &str,
    item: &ObjectView<'_>,
    correction: &KirShapeCorrection,
) -> Result<f64> {
    let KirShapeCorrectionPayload::Jes {
        file,
        correction: payload_name,
        inputs,
    } = &correction.payload
    else {
        return Ok(0.0);
    };
    let set = correction_set_from_path(file)?;
    let payload = set.correction_ref(payload_name).map_err(|error| {
        InterpretError::Correction(format!(
            "JES correction `{}` payload lookup failed: {error}",
            correction.name
        ))
    })?;
    let mut values = Vec::with_capacity(payload.inputs().len());
    for payload_input in payload.inputs() {
        let input = inputs
            .iter()
            .find(|input| input.name == payload_input.name)
            .ok_or_else(|| {
                InterpretError::Correction(format!(
                    "JES correction `{}` is missing input `{}`",
                    correction.name, payload_input.name
                ))
            })?;
        values.push(correction_input_value(
            program,
            event,
            &correction.collection,
            source,
            item,
            input,
            None,
        )?);
    }
    payload.evaluate(&set, &values).map_err(|error| {
        InterpretError::Correction(format!(
            "JES correction `{}` evaluation failed: {error}",
            correction.name
        ))
    })
}

fn correction_input_value(
    program: &KirProgram,
    event: &Event,
    collection: &str,
    source: &str,
    item: &ObjectView<'_>,
    input: &crate::ScaleFactorInputDef,
    pseudo: Option<(&str, CorrectionValue)>,
) -> Result<CorrectionValue> {
    match &input.source {
        crate::ScaleFactorInputSource::Literal(value) => Ok(scale_factor_literal_value(value)),
        crate::ScaleFactorInputSource::From(attr) => {
            if let Some((name, value)) = pseudo {
                if attr == name {
                    return Ok(value);
                }
            }
            if let Some(correction) = program
                .object_corrections
                .iter()
                .find(|correction| correction.collection == collection && correction.attr == *attr)
            {
                return Ok(CorrectionValue::Real(
                    evaluate_object_correction(program, event, source, item, correction)?.as_f64(),
                ));
            }
            let branch = format!("{source}_{attr}");
            if program.read_branches.iter().any(|spec| spec.name == branch) {
                let branch_type = branch_type(program, &branch)?;
                return object_correction_value(item, attr, &branch, branch_type);
            }
            let branch_type = branch_type(program, attr)?;
            scalar_correction_value(event, attr, branch_type)
        }
    }
}

fn object_correction_value(
    item: &ObjectView<'_>,
    attr: &str,
    branch: &str,
    branch_type: BranchType,
) -> Result<CorrectionValue> {
    match branch_type {
        BranchType::VecI8 => Ok(CorrectionValue::Int(i64::from(item.get::<i8>(attr)?))),
        BranchType::VecU8 => Ok(CorrectionValue::Int(i64::from(item.get::<u8>(attr)?))),
        BranchType::VecI16 => Ok(CorrectionValue::Int(i64::from(item.get::<i16>(attr)?))),
        BranchType::VecU16 => Ok(CorrectionValue::Int(i64::from(item.get::<u16>(attr)?))),
        BranchType::VecI32 => Ok(CorrectionValue::Int(i64::from(item.get::<i32>(attr)?))),
        BranchType::VecU32 => Ok(CorrectionValue::Int(i64::from(item.get::<u32>(attr)?))),
        BranchType::VecI64 => Ok(CorrectionValue::Int(item.get::<i64>(attr)?)),
        BranchType::VecU64 => i64::try_from(item.get::<u64>(attr)?)
            .map(CorrectionValue::Int)
            .map_err(|error| {
                InterpretError::NumericConversion(format!(
                    "unsigned correction input `{attr}` cannot fit into i64: {error}"
                ))
            }),
        BranchType::VecF32 => Ok(CorrectionValue::Real(f64::from(item.get::<f32>(attr)?))),
        other => Err(InterpretError::TypeMismatch {
            branch: branch.to_string(),
            branch_type: other,
            expected: "numeric vector branch",
        }),
    }
}

fn evaluate_scale_factor_correction(
    program: &KirProgram,
    event: &Event,
    correction: &crate::kir::KirScaleFactorCorrection,
    selected: &SelectedObjects,
    systematic: &str,
) -> Result<f64> {
    let set = nano_corrections::CorrectionSet::from_path(&correction.file).map_err(|error| {
        InterpretError::Correction(format!(
            "scale-factor correction `{}` failed to load `{}`: {error}",
            correction.name, correction.file
        ))
    })?;
    let payload = set.correction(&correction.correction).map_err(|error| {
        InterpretError::Correction(format!(
            "scale-factor correction `{}` payload lookup failed: {error}",
            correction.name
        ))
    })?;
    let objects = selected
        .get(&correction.collection)
        .ok_or_else(|| InterpretError::MissingObject(correction.collection.clone()))?;
    let mut weight = 1.0_f64;
    for object in objects {
        let mut values = Vec::with_capacity(payload.inputs.len());
        for payload_input in &payload.inputs {
            if correction
                .systematic
                .as_ref()
                .is_some_and(|declared| declared.name == payload_input.name)
            {
                values.push(CorrectionValue::Str(scale_factor_systematic_value(
                    correction, systematic,
                )));
                continue;
            }
            let input = correction
                .inputs
                .iter()
                .find(|input| input.name == payload_input.name)
                .ok_or_else(|| {
                    InterpretError::Correction(format!(
                        "scale-factor correction `{}` is missing input `{}`",
                        correction.name, payload_input.name
                    ))
                })?;
            values.push(scale_factor_input_value(
                program, event, correction, object, input,
            )?);
        }
        weight *= payload.evaluate(&values).map_err(|error| {
            InterpretError::Correction(format!(
                "scale-factor correction `{}` evaluation failed: {error}",
                correction.name
            ))
        })?;
    }
    Ok(weight)
}

fn scale_factor_systematic_value(
    correction: &crate::kir::KirScaleFactorCorrection,
    systematic: &str,
) -> String {
    let Some(declared) = &correction.systematic else {
        return String::new();
    };
    if systematic == interpreted_variant_name(&correction.name, "Up") {
        declared.up.clone()
    } else if systematic == interpreted_variant_name(&correction.name, "Down") {
        declared.down.clone()
    } else {
        declared.nominal.clone()
    }
}

fn scale_factor_input_value(
    program: &KirProgram,
    event: &Event,
    correction: &crate::kir::KirScaleFactorCorrection,
    object: &SelectedObject,
    input: &crate::ScaleFactorInputDef,
) -> Result<CorrectionValue> {
    match &input.source {
        crate::ScaleFactorInputSource::Literal(value) => Ok(scale_factor_literal_value(value)),
        crate::ScaleFactorInputSource::From(source) => {
            let collection_source = program
                .objects
                .iter()
                .find(|object| object.name == correction.collection)
                .map(|object| object.source.as_str())
                .ok_or_else(|| InterpretError::MissingObject(correction.collection.clone()))?;
            let object_branch = format!("{collection_source}_{source}");
            if program
                .read_branches
                .iter()
                .any(|branch| branch.name == object_branch)
            {
                let value = object.leading_values.get(source).copied().ok_or_else(|| {
                    InterpretError::InvalidExpression(format!(
                        "selected `{}` object is missing scale-factor input `{source}`",
                        correction.collection
                    ))
                })?;
                return numeric_value_to_correction_value(value);
            }
            let branch_type = branch_type(program, source)?;
            scalar_correction_value(event, source, branch_type)
        }
    }
}

fn scale_factor_literal_value(value: &crate::ScaleFactorLiteral) -> CorrectionValue {
    match value {
        crate::ScaleFactorLiteral::Real(value) => CorrectionValue::Real(*value),
        crate::ScaleFactorLiteral::Int(value) => CorrectionValue::Int(*value),
        crate::ScaleFactorLiteral::Str(value) => CorrectionValue::Str(value.clone()),
    }
}

fn numeric_value_to_correction_value(value: NumericValue) -> Result<CorrectionValue> {
    match value {
        NumericValue::F64(value) => Ok(CorrectionValue::Real(value)),
        NumericValue::I64(value) => Ok(CorrectionValue::Int(value)),
        NumericValue::U64(value) => {
            i64::try_from(value)
                .map(CorrectionValue::Int)
                .map_err(|error| {
                    InterpretError::NumericConversion(format!(
                        "unsigned scale-factor input cannot fit into i64: {error}"
                    ))
                })
        }
    }
}

fn scalar_correction_value(
    event: &Event,
    branch: &str,
    branch_type: BranchType,
) -> Result<CorrectionValue> {
    match branch_type {
        BranchType::I8 => Ok(CorrectionValue::Int(i64::from(event.scalar::<i8>(branch)?))),
        BranchType::U8 => Ok(CorrectionValue::Int(i64::from(event.scalar::<u8>(branch)?))),
        BranchType::I16 => Ok(CorrectionValue::Int(i64::from(
            event.scalar::<i16>(branch)?,
        ))),
        BranchType::U16 => Ok(CorrectionValue::Int(i64::from(
            event.scalar::<u16>(branch)?,
        ))),
        BranchType::I32 => Ok(CorrectionValue::Int(i64::from(
            event.scalar::<i32>(branch)?,
        ))),
        BranchType::U32 => Ok(CorrectionValue::Int(i64::from(
            event.scalar::<u32>(branch)?,
        ))),
        BranchType::I64 => Ok(CorrectionValue::Int(event.scalar::<i64>(branch)?)),
        BranchType::U64 => i64::try_from(event.scalar::<u64>(branch)?)
            .map(CorrectionValue::Int)
            .map_err(|error| {
                InterpretError::NumericConversion(format!(
                    "scalar branch `{branch}` cannot fit into i64: {error}"
                ))
            }),
        BranchType::F32 => Ok(CorrectionValue::Real(f64::from(
            event.scalar::<f32>(branch)?,
        ))),
        other => Err(InterpretError::TypeMismatch {
            branch: branch.to_string(),
            branch_type: other,
            expected: "numeric scalar branch",
        }),
    }
}

fn shape_factor_for_source(
    program: &KirProgram,
    event: &Event,
    source: &str,
    attr: &str,
    item: &ObjectView<'_>,
    systematic: &str,
) -> Result<f64> {
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
        .map(|correction| {
            shape_correction_factor(program, event, source, item, correction, systematic)
        })
        .try_fold(1.0, |product, factor| factor.map(|factor| product * factor))
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
        Expr::Attr { .. }
        | Expr::Literal(_)
        | Expr::Count(_)
        | Expr::SumAttr { .. }
        | Expr::IndexNotIn { .. }
        | Expr::JetIdTightRun2024 { .. }
        | Expr::JetVetoMapRun2024 { .. } => Ok(false),
        Expr::EventScalar(_) => Ok(false),
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
        Expr::ZepVv { system, dijet, .. } => {
            Ok(derived_object(derived, system)?.is_none()
                || derived_object(derived, dijet)?.is_none())
        }
        Expr::SystemMetEta { system, .. } | Expr::SystemMetPt { system, .. } => {
            Ok(derived_object(derived, system)?.is_none())
        }
        Expr::SystemPairMetPt { system, pair, .. } => Ok(derived_object(derived, system)?
            .is_none()
            || derived_object(derived, pair)?.is_none()),
        Expr::SystemPairPtBalance { system, pair, .. } => Ok(derived_object(derived, system)?
            .is_none()
            || derived_object(derived, pair)?.is_none()),
        Expr::MetType1Pt { .. } | Expr::MetType1Phi { .. } | Expr::LeadingType1Mt { .. } => {
            Ok(false)
        }
        Expr::SystemDeltaPhi { left, right } => {
            Ok(derived_object(derived, left)?.is_none()
                || derived_object(derived, right)?.is_none())
        }
        Expr::LegacyLeptonRpt { dijet, .. } => Ok(derived_object(derived, dijet)?.is_none()),
        Expr::PairConstituentAttr { pair, .. } => Ok(derived_object(derived, pair)?.is_none()),
        Expr::ZepMax { system, dijet } => {
            Ok(derived_object(derived, system)?.is_none()
                || derived_object(derived, dijet)?.is_none())
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
    event: &Event,
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
        Expr::EventScalar(branch) => Ok(Some(read_event_scalar_value(event, branch)?)),
        Expr::Literal(_) | Expr::Binary { .. } | Expr::Abs(_) | Expr::Sqrt(_) => Ok(Some(
            Value::F64(eval_numeric_expr(expr, selected, derived, None, Some(event))?.as_f64()),
        )),
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
                NumericValue::U64(value) => Value::U64(value),
            }))
        }
        Expr::Attr { object, attr } => {
            let Some(candidate) = derived_object(derived, object)? else {
                return Ok(None);
            };
            match attr.as_str() {
                "mass" => Ok(Some(Value::F64(candidate.mass))),
                "pt" => Ok(Some(Value::F64(candidate.pt))),
                "eta" => Ok(Some(Value::F64(candidate.eta))),
                "phi" => Ok(Some(Value::F64(candidate.phi))),
                "min_delta_r" | "dR" | "dr" => Ok(Some(Value::F64(candidate.min_delta_r))),
                "delta_eta" => Ok(Some(Value::F64(candidate.delta_eta))),
                "delta_phi" => Ok(Some(Value::F64(candidate.delta_phi))),
                "leading_pt" => Ok(Some(Value::F64(candidate.leading_pt))),
                "subleading_pt" => Ok(Some(Value::F64(candidate.subleading_pt))),
                "leading_eta" => Ok(Some(Value::F64(candidate.leading_eta))),
                "subleading_eta" => Ok(Some(Value::F64(candidate.subleading_eta))),
                "leading_phi" => Ok(Some(Value::F64(candidate.leading_phi))),
                "subleading_phi" => Ok(Some(Value::F64(candidate.subleading_phi))),
                "leading_mass" => Ok(Some(Value::F64(candidate.leading_mass))),
                "subleading_mass" => Ok(Some(Value::F64(candidate.subleading_mass))),
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
        Expr::ZepVv {
            system,
            met_pt,
            met_phi,
            dijet,
        } => Ok(Some(Value::F64(zep_vv(
            event, derived, system, met_pt, met_phi, dijet,
        )?))),
        Expr::SystemMetEta {
            system,
            met_pt,
            met_phi,
        } => Ok(Some(Value::F64(system_met_eta(
            event, derived, system, met_pt, met_phi,
        )?))),
        Expr::SystemMetPt {
            system,
            met_pt,
            met_phi,
        } => Ok(Some(Value::F64(system_met_pt(
            event, derived, system, met_pt, met_phi,
        )?))),
        Expr::SystemPairMetPt {
            system,
            met_pt,
            met_phi,
            pair,
        } => Ok(Some(Value::F64(system_pair_met_pt(
            event, derived, system, met_pt, met_phi, pair,
        )?))),
        Expr::SystemPairPtBalance {
            system,
            met_pt,
            met_phi,
            pair,
        } => Ok(Some(Value::F64(system_pair_pt_balance(
            event, derived, system, met_pt, met_phi, pair,
        )?))),
        Expr::MetType1Pt {
            jets,
            met_pt,
            met_phi,
            nominal_pt,
            shifted_pt,
        } => Ok(Some(Value::F64(met_type1_pt(
            event, selected, jets, met_pt, met_phi, nominal_pt, shifted_pt,
        )?))),
        Expr::MetType1Phi {
            jets,
            met_pt,
            met_phi,
            nominal_pt,
            shifted_pt,
        } => Ok(Some(Value::F64(met_type1_phi(
            event, selected, jets, met_pt, met_phi, nominal_pt, shifted_pt,
        )?))),
        Expr::LeadingType1Mt {
            object,
            jets,
            met_pt,
            met_phi,
            nominal_pt,
            shifted_pt,
        } => Ok(Some(Value::F64(leading_type1_mt(
            event, selected, object, jets, met_pt, met_phi, nominal_pt, shifted_pt,
        )?))),
        Expr::SystemDeltaPhi { left, right } => {
            Ok(Some(Value::F64(system_delta_phi(derived, left, right)?)))
        }
        Expr::LegacyLeptonRpt {
            muons,
            electrons,
            dijet,
        } => Ok(Some(Value::F64(legacy_lepton_rpt(
            selected, derived, muons, electrons, dijet,
        )?))),
        Expr::PairConstituentAttr { pair, attr, rank } => Ok(Some(Value::F64(
            pair_constituent_attr(derived, pair, attr, *rank)?,
        ))),
        Expr::ZepMax { system, dijet } => Ok(Some(Value::F64(zep_max(derived, system, dijet)?))),
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
    event: Option<&Event>,
) -> Result<NumericValue> {
    match expr {
        Expr::EventScalar(branch) => {
            let event = event.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires event scalar access"
                ))
            })?;
            read_event_scalar_numeric(event, branch)
        }
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
                "eta" => Ok(NumericValue::F64(candidate.eta)),
                "phi" => Ok(NumericValue::F64(candidate.phi)),
                "min_delta_r" | "dR" | "dr" => Ok(NumericValue::F64(candidate.min_delta_r)),
                "delta_eta" => Ok(NumericValue::F64(candidate.delta_eta)),
                "delta_phi" => Ok(NumericValue::F64(candidate.delta_phi)),
                "leading_pt" => Ok(NumericValue::F64(candidate.leading_pt)),
                "subleading_pt" => Ok(NumericValue::F64(candidate.subleading_pt)),
                "leading_eta" => Ok(NumericValue::F64(candidate.leading_eta)),
                "subleading_eta" => Ok(NumericValue::F64(candidate.subleading_eta)),
                "leading_phi" => Ok(NumericValue::F64(candidate.leading_phi)),
                "subleading_phi" => Ok(NumericValue::F64(candidate.subleading_phi)),
                "leading_mass" => Ok(NumericValue::F64(candidate.leading_mass)),
                "subleading_mass" => Ok(NumericValue::F64(candidate.subleading_mass)),
                other => Err(InterpretError::InvalidExpression(format!(
                    "derived object `{object}` has no interpreted attribute `{other}`"
                ))),
            }
        }
        Expr::Literal(value) => Ok(NumericValue::F64(*value)),
        Expr::Binary { op, lhs, rhs } => {
            let lhs = eval_numeric_expr(lhs, selected, derived, current, event)?.as_f64();
            let rhs = eval_numeric_expr(rhs, selected, derived, current, event)?.as_f64();
            Ok(NumericValue::F64(eval_arithmetic(*op, lhs, rhs)))
        }
        Expr::Abs(inner) => Ok(eval_numeric_expr(inner, selected, derived, current, event)?.abs()),
        Expr::Sqrt(inner) => Ok(NumericValue::F64(
            eval_numeric_expr(inner, selected, derived, current, event)?
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
        Expr::IndexNotIn { object, attr } => {
            let Some((_, current)) = current else {
                return Err(InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires a current selected object"
                )));
            };
            Ok(NumericValue::U64(u64::from(index_not_in(
                selected,
                object,
                attr,
                current.source_index,
            )?)))
        }
        Expr::JetIdTightRun2024 { .. } => Err(InterpretError::InvalidExpression(format!(
            "expression `{expr}` requires a current object item"
        ))),
        Expr::JetVetoMapRun2024 { .. } => Err(InterpretError::InvalidExpression(format!(
            "expression `{expr}` requires a current object item"
        ))),
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
        Expr::ZepVv {
            system,
            met_pt,
            met_phi,
            dijet,
        } => {
            let event = event.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires event scalar access"
                ))
            })?;
            Ok(NumericValue::F64(zep_vv(
                event, derived, system, met_pt, met_phi, dijet,
            )?))
        }
        Expr::SystemMetEta {
            system,
            met_pt,
            met_phi,
        } => {
            let event = event.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires event scalar access"
                ))
            })?;
            Ok(NumericValue::F64(system_met_eta(
                event, derived, system, met_pt, met_phi,
            )?))
        }
        Expr::SystemMetPt {
            system,
            met_pt,
            met_phi,
        } => {
            let event = event.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires event scalar access"
                ))
            })?;
            Ok(NumericValue::F64(system_met_pt(
                event, derived, system, met_pt, met_phi,
            )?))
        }
        Expr::SystemPairMetPt {
            system,
            met_pt,
            met_phi,
            pair,
        } => {
            let event = event.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires event scalar access"
                ))
            })?;
            Ok(NumericValue::F64(system_pair_met_pt(
                event, derived, system, met_pt, met_phi, pair,
            )?))
        }
        Expr::SystemPairPtBalance {
            system,
            met_pt,
            met_phi,
            pair,
        } => {
            let event = event.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires event scalar access"
                ))
            })?;
            Ok(NumericValue::F64(system_pair_pt_balance(
                event, derived, system, met_pt, met_phi, pair,
            )?))
        }
        Expr::MetType1Pt {
            jets,
            met_pt,
            met_phi,
            nominal_pt,
            shifted_pt,
        } => {
            let event = event.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires event scalar access"
                ))
            })?;
            Ok(NumericValue::F64(met_type1_pt(
                event, selected, jets, met_pt, met_phi, nominal_pt, shifted_pt,
            )?))
        }
        Expr::MetType1Phi {
            jets,
            met_pt,
            met_phi,
            nominal_pt,
            shifted_pt,
        } => {
            let event = event.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires event scalar access"
                ))
            })?;
            Ok(NumericValue::F64(met_type1_phi(
                event, selected, jets, met_pt, met_phi, nominal_pt, shifted_pt,
            )?))
        }
        Expr::LeadingType1Mt {
            object,
            jets,
            met_pt,
            met_phi,
            nominal_pt,
            shifted_pt,
        } => {
            let event = event.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "expression `{expr}` requires event scalar access"
                ))
            })?;
            Ok(NumericValue::F64(leading_type1_mt(
                event, selected, object, jets, met_pt, met_phi, nominal_pt, shifted_pt,
            )?))
        }
        Expr::SystemDeltaPhi { left, right } => {
            Ok(NumericValue::F64(system_delta_phi(derived, left, right)?))
        }
        Expr::LegacyLeptonRpt {
            muons,
            electrons,
            dijet,
        } => Ok(NumericValue::F64(legacy_lepton_rpt(
            selected, derived, muons, electrons, dijet,
        )?)),
        Expr::PairConstituentAttr { pair, attr, rank } => Ok(NumericValue::F64(
            pair_constituent_attr(derived, pair, attr, *rank)?,
        )),
        Expr::ZepMax { system, dijet } => Ok(NumericValue::F64(zep_max(derived, system, dijet)?)),
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
        .map(|selected_object| selected_object.p4.pt)
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

fn read_event_scalar_value(event: &Event, branch: &str) -> Result<Value> {
    let branch_type = event
        .schema()
        .find(branch)
        .map(|info| info.branch_type)
        .ok_or_else(|| InterpretError::MissingBranch(branch.to_string()))?;
    match branch_type {
        BranchType::Bool => Ok(Value::Bool(event.scalar::<bool>(branch)?)),
        BranchType::I8 => Ok(Value::I64(i64::from(event.scalar::<i8>(branch)?))),
        BranchType::U8 => Ok(Value::I64(i64::from(event.scalar::<u8>(branch)?))),
        BranchType::I16 => Ok(Value::I64(i64::from(event.scalar::<i16>(branch)?))),
        BranchType::U16 => Ok(Value::I64(i64::from(event.scalar::<u16>(branch)?))),
        BranchType::I32 => Ok(Value::I64(i64::from(event.scalar::<i32>(branch)?))),
        BranchType::U32 => Ok(Value::U32(event.scalar::<u32>(branch)?)),
        BranchType::I64 => Ok(Value::I64(event.scalar::<i64>(branch)?)),
        BranchType::U64 => Ok(Value::U64(event.scalar::<u64>(branch)?)),
        BranchType::F32 => Ok(Value::F64(f64::from(event.scalar::<f32>(branch)?))),
        other => Err(InterpretError::TypeMismatch {
            branch: branch.to_string(),
            branch_type: other,
            expected: "scalar bool or numeric",
        }),
    }
}

fn read_event_scalar_numeric(event: &Event, branch: &str) -> Result<NumericValue> {
    let branch_type = event
        .schema()
        .find(branch)
        .map(|info| info.branch_type)
        .ok_or_else(|| InterpretError::MissingBranch(branch.to_string()))?;
    match branch_type {
        BranchType::I8 => Ok(NumericValue::I64(i64::from(event.scalar::<i8>(branch)?))),
        BranchType::U8 => Ok(NumericValue::U64(u64::from(event.scalar::<u8>(branch)?))),
        BranchType::I16 => Ok(NumericValue::I64(i64::from(event.scalar::<i16>(branch)?))),
        BranchType::U16 => Ok(NumericValue::U64(u64::from(event.scalar::<u16>(branch)?))),
        BranchType::I32 => Ok(NumericValue::I64(i64::from(event.scalar::<i32>(branch)?))),
        BranchType::U32 => Ok(NumericValue::U64(u64::from(event.scalar::<u32>(branch)?))),
        BranchType::I64 => Ok(NumericValue::I64(event.scalar::<i64>(branch)?)),
        BranchType::U64 => Ok(NumericValue::U64(event.scalar::<u64>(branch)?)),
        BranchType::F32 => Ok(NumericValue::F64(f64::from(event.scalar::<f32>(branch)?))),
        other => Err(InterpretError::TypeMismatch {
            branch: branch.to_string(),
            branch_type: other,
            expected: "numeric scalar",
        }),
    }
}

fn system_met_components(
    event: &Event,
    derived: &DerivedObjects,
    system: &str,
    met_pt: &str,
    met_phi: &str,
) -> Result<(f64, f64, f64)> {
    let system = derived_object(derived, system)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{system}` has no candidate"))
    })?;
    let met_pt = f64::from(event.scalar::<f32>(met_pt)?);
    let met_phi = f64::from(event.scalar::<f32>(met_phi)?);
    Ok((
        system.px + met_pt * met_phi.cos(),
        system.py + met_pt * met_phi.sin(),
        system.pz,
    ))
}

fn system_met_eta(
    event: &Event,
    derived: &DerivedObjects,
    system: &str,
    met_pt: &str,
    met_phi: &str,
) -> Result<f64> {
    let (px, py, pz) = system_met_components(event, derived, system, met_pt, met_phi)?;
    Ok(vector_eta(px, py, pz))
}

fn system_met_pt(
    event: &Event,
    derived: &DerivedObjects,
    system: &str,
    met_pt: &str,
    met_phi: &str,
) -> Result<f64> {
    let (px, py, _) = system_met_components(event, derived, system, met_pt, met_phi)?;
    Ok((px * px + py * py).sqrt())
}

fn system_pair_met_pt(
    event: &Event,
    derived: &DerivedObjects,
    system: &str,
    met_pt: &str,
    met_phi: &str,
    pair: &str,
) -> Result<f64> {
    let (px, py, _) = system_met_components(event, derived, system, met_pt, met_phi)?;
    let pair = derived_object(derived, pair)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{pair}` has no candidate"))
    })?;
    Ok(((px + pair.px).powi(2) + (py + pair.py).powi(2)).sqrt())
}

fn system_pair_pt_balance(
    event: &Event,
    derived: &DerivedObjects,
    system: &str,
    met_pt: &str,
    met_phi: &str,
    pair: &str,
) -> Result<f64> {
    let vv_pt = system_met_pt(event, derived, system, met_pt, met_phi)?;
    let pair = derived_object(derived, pair)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{pair}` has no candidate"))
    })?;
    Ok((vv_pt - pair.pt) / pair.pt)
}

fn met_type1_components(
    event: &Event,
    selected: &SelectedObjects,
    jets: &str,
    met_pt: &str,
    met_phi: &str,
    nominal_pt: &str,
    shifted_pt: &str,
) -> Result<(f64, f64)> {
    let met_pt = f64::from(event.scalar::<f32>(met_pt)?);
    let met_phi = f64::from(event.scalar::<f32>(met_phi)?);
    let mut px = met_pt * met_phi.cos();
    let mut py = met_pt * met_phi.sin();
    let objects = selected
        .get(jets)
        .ok_or_else(|| InterpretError::MissingObject(jets.to_string()))?;

    for jet in objects {
        let ch_em_ef = required_attr_f64(jet, jets, "chEmEF")?;
        let ne_em_ef = required_attr_f64(jet, jets, "neEmEF")?;
        if ch_em_ef + ne_em_ef >= 0.9 {
            continue;
        }
        let muon_subtr = required_attr_f64(jet, jets, "muonSubtrFactor")?;
        let nominal = required_attr_f64(jet, jets, nominal_pt)? * (1.0 - muon_subtr);
        if nominal < 15.0 {
            continue;
        }
        let shifted = required_attr_f64(jet, jets, shifted_pt)? * (1.0 - muon_subtr);
        let phi = required_attr_f64(jet, jets, "phi")?;
        px += (nominal - shifted) * phi.cos();
        py += (nominal - shifted) * phi.sin();
    }
    Ok((px, py))
}

fn met_type1_pt(
    event: &Event,
    selected: &SelectedObjects,
    jets: &str,
    met_pt: &str,
    met_phi: &str,
    nominal_pt: &str,
    shifted_pt: &str,
) -> Result<f64> {
    let (px, py) = met_type1_components(
        event, selected, jets, met_pt, met_phi, nominal_pt, shifted_pt,
    )?;
    Ok(px.hypot(py))
}

fn met_type1_phi(
    event: &Event,
    selected: &SelectedObjects,
    jets: &str,
    met_pt: &str,
    met_phi: &str,
    nominal_pt: &str,
    shifted_pt: &str,
) -> Result<f64> {
    let (px, py) = met_type1_components(
        event, selected, jets, met_pt, met_phi, nominal_pt, shifted_pt,
    )?;
    Ok(vector_phi(px, py))
}

#[allow(clippy::too_many_arguments)]
fn leading_type1_mt(
    event: &Event,
    selected: &SelectedObjects,
    object: &str,
    jets: &str,
    met_pt: &str,
    met_phi: &str,
    nominal_pt: &str,
    shifted_pt: &str,
) -> Result<f64> {
    let lepton = selected
        .get(object)
        .and_then(|objects| objects.first())
        .ok_or_else(|| {
            InterpretError::InvalidExpression(format!(
                "`leading_type1_mt({object}, ...)` has no selected object"
            ))
        })?;
    let lepton_pt = required_attr_f64(lepton, object, "pt")?;
    let lepton_phi = required_attr_f64(lepton, object, "phi")?;
    let (met_px, met_py) = met_type1_components(
        event, selected, jets, met_pt, met_phi, nominal_pt, shifted_pt,
    )?;
    let corrected_met_pt = met_px.hypot(met_py);
    let corrected_met_phi = vector_phi(met_px, met_py);
    Ok((2.0
        * lepton_pt
        * corrected_met_pt
        * (1.0 - delta_phi(lepton_phi, corrected_met_phi).cos()))
    .sqrt())
}

fn system_delta_phi(derived: &DerivedObjects, left: &str, right: &str) -> Result<f64> {
    let left = derived_object(derived, left)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{left}` has no candidate"))
    })?;
    let right = derived_object(derived, right)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{right}` has no candidate"))
    })?;
    Ok(delta_phi(left.phi, right.phi))
}

fn legacy_lepton_rpt(
    selected: &SelectedObjects,
    derived: &DerivedObjects,
    muons: &str,
    electrons: &str,
    dijet: &str,
) -> Result<f64> {
    let dijet = derived_object(derived, dijet)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{dijet}` has no candidate"))
    })?;
    let mut lepton_pts = Vec::new();
    for object in [muons, electrons] {
        let objects = selected
            .get(object)
            .ok_or_else(|| InterpretError::MissingObject(object.to_string()))?;
        for item in objects {
            lepton_pts.push(item.p4.pt);
        }
    }
    if lepton_pts.len() < 2 {
        return Ok(1.0);
    }
    Ok((lepton_pts[0] * lepton_pts[1]) / (dijet.leading_pt * dijet.subleading_pt))
}

fn pair_constituent_attr(
    derived: &DerivedObjects,
    pair: &str,
    attr: &str,
    rank: PairConstituentRank,
) -> Result<f64> {
    let pair_object = derived_object(derived, pair)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{pair}` has no candidate"))
    })?;
    let mut constituents = pair_object.constituents.iter().collect::<Vec<_>>();
    constituents.sort_by(|left, right| right.pt.as_f64().total_cmp(&left.pt.as_f64()));
    let index = match rank {
        PairConstituentRank::Leading => 0,
        PairConstituentRank::Subleading => 1,
    };
    let rank_label = match rank {
        PairConstituentRank::Leading => "leading",
        PairConstituentRank::Subleading => "subleading",
    };
    let Some(constituent) = constituents.get(index) else {
        return Err(InterpretError::InvalidExpression(format!(
            "derived pair `{pair}` has no {rank_label} constituent"
        )));
    };
    constituent
        .values
        .get(attr)
        .copied()
        .map(NumericValue::as_f64)
        .ok_or_else(|| {
            InterpretError::InvalidExpression(format!(
                "attribute `{attr}` was not materialized for derived pair `{pair}`"
            ))
        })
}

fn zep_vv(
    event: &Event,
    derived: &DerivedObjects,
    system: &str,
    met_pt: &str,
    met_phi: &str,
    dijet: &str,
) -> Result<f64> {
    let dijet = derived_object(derived, dijet)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{dijet}` has no candidate"))
    })?;
    if dijet.delta_eta <= 0.0 {
        return Ok(f64::INFINITY);
    }

    let vv_eta = system_met_eta(event, derived, system, met_pt, met_phi)?;
    let jet_midpoint = (dijet.leading_eta + dijet.subleading_eta) / 2.0;

    Ok((vv_eta - jet_midpoint).abs() / dijet.delta_eta)
}

fn zep_max(derived: &DerivedObjects, system: &str, dijet: &str) -> Result<f64> {
    let system = derived_object(derived, system)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{system}` has no candidate"))
    })?;
    let dijet = derived_object(derived, dijet)?.ok_or_else(|| {
        InterpretError::InvalidExpression(format!("derived object `{dijet}` has no candidate"))
    })?;
    if dijet.delta_eta <= 0.0 {
        return Ok(f64::INFINITY);
    }
    let jet_midpoint = (dijet.leading_eta + dijet.subleading_eta) / 2.0;
    Ok(system
        .constituents
        .iter()
        .map(|constituent| (constituent.eta.as_f64() - jet_midpoint).abs() / dijet.delta_eta)
        .fold(0.0_f64, f64::max))
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
        None,
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

fn compare_bool(lhs: bool, op: CmpOp, rhs: f64) -> Result<bool> {
    let rhs = match rhs {
        0.0 => false,
        1.0 => true,
        _ => {
            return Err(InterpretError::InvalidExpression(format!(
                "boolean comparison rhs must be 0 or 1, got {rhs}"
            )));
        }
    };
    match op {
        CmpOp::Eq => Ok(lhs == rhs),
        CmpOp::Ne => Ok(lhs != rhs),
        _ => Err(InterpretError::InvalidExpression(
            "boolean comparison supports only == or !=".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
    use nano_producers::{MuonProducer, MuonSkimRow};

    use super::*;
    use crate::{validate, AnalysisSpec, Catalogue};

    const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
    const NANOV15_CATALOGUE: &str = include_str!("../../../configs/branches/nanov15.yaml");
    const MUON_SPEC_TOML: &str = include_str!("../examples/muon.toml");
    const JEC_NOMINAL_SPEC_TOML: &str = r#"
[analysis]
name = "jec_nominal_runtime"
year = "Run2024"

[[correction]]
name = "jet_pt_jec"
kind = "jec_nominal"
file = "../nano-spec/tests/data/jec_nominal.json"
correction = "synthetic_jec_nominal"
collection = "good_jet"
attr = "ptJec"
source_attr = "pt"
raw_factor_attr = "rawFactor"
inputs = [
  { name = "JetA", from = "area" },
  { name = "JetEta", from = "eta" },
  { name = "JetPt", from = "raw_pt" },
  { name = "Rho", from = "Rho_fixedGridRhoFastjetAll" },
  { name = "JetPhi", from = "phi" },
]

[objects.good_jet]
source = "Jet"
kinematics = { pt = "ptJec" }
cuts = ["ptJec > 90 GeV"]

[[outputs]]
name = "lead_jec_pt"
expr = "leading(good_jet).ptJec"
"#;
    const JER_NOMINAL_SPEC_TOML: &str = r#"
[analysis]
name = "jer_nominal_runtime"
year = "Run2024"

[[correction]]
name = "jet_pt_jec"
kind = "jec_nominal"
file = "../nano-spec/tests/data/jec_nominal.json"
correction = "synthetic_jec_nominal"
collection = "good_jet"
attr = "ptJec"
source_attr = "pt"
raw_factor_attr = "rawFactor"
inputs = [
  { name = "JetA", from = "area" },
  { name = "JetEta", from = "eta" },
  { name = "JetPt", from = "raw_pt" },
  { name = "Rho", from = "Rho_fixedGridRhoFastjetAll" },
  { name = "JetPhi", from = "phi" },
]

[[correction]]
name = "jet_pt_def"
kind = "jer_nominal"
collection = "good_jet"
attr = "ptDef"
source_attr = "ptJec"
scale_factor_file = "../nano-spec/tests/data/jer_nominal.json"
scale_factor_correction = "synthetic_jer_scale_factor"
scale_factor_inputs = [
  { name = "JetEta", from = "eta" },
  { name = "JetPt", from = "ptJec" },
  { name = "systematic", value = "nom" },
]
resolution_file = "../nano-spec/tests/data/jer_nominal.json"
resolution_correction = "synthetic_jer_resolution"
resolution_inputs = [
  { name = "JetEta", from = "eta" },
  { name = "JetPt", from = "ptJec" },
  { name = "Rho", from = "Rho_fixedGridRhoFastjetAll" },
]
gen_jet_index_attr = "genJetIdx"
gen_jet_pt_branch = "GenJet_pt"

[objects.good_jet]
source = "Jet"
kinematics = { pt = "ptDef" }
cuts = ["ptDef > 90 GeV"]

[[outputs]]
name = "lead_def_pt"
expr = "leading(good_jet).ptDef"
"#;

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
    fn interpret_jec_nominal_virtual_object_attribute() {
        let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV15_CATALOGUE, "v15")
            .expect("parse nanov15 catalogue");
        let spec =
            AnalysisSpec::from_toml_str(JEC_NOMINAL_SPEC_TOML).expect("parse JEC nominal spec");
        let plan = validate(&spec, &catalogue).expect("validate JEC nominal spec");

        let row = interpret(&plan, &jec_nominal_event())
            .expect("interpret")
            .expect("event selected");

        assert_eq!(
            row.values,
            vec![("lead_jec_pt".to_string(), Value::F64(99.0))]
        );
    }

    #[test]
    fn interpret_jer_nominal_virtual_object_attribute() {
        let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV15_CATALOGUE, "v15")
            .expect("parse nanov15 catalogue");
        let spec =
            AnalysisSpec::from_toml_str(JER_NOMINAL_SPEC_TOML).expect("parse JER nominal spec");
        let plan = validate(&spec, &catalogue).expect("validate JER nominal spec");

        let row = interpret(&plan, &jer_nominal_event())
            .expect("interpret")
            .expect("event selected");

        let Some(Value::F64(value)) = row.get("lead_def_pt") else {
            panic!("missing lead_def_pt output: {:?}", row.values);
        };
        assert!((value - 100.8).abs() < 1.0e-5, "value={value}");
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
    fn interpret_pair_geometry_outputs_vbs_observables() {
        let spec = AnalysisSpec::from_toml_str(
            r#"
[analysis]
name = "vbs_pair_geometry"
year = "Run2018"

[objects.vbs_jet]
source = "Jet"
cuts = ["pt > 50 GeV", "abs(eta) < 4.7"]

[derived.vbs_jj]
kind = "pair"
object = "vbs_jet"
selection = "leading_pt"

[regions.signal]
require = ["vbs_jj.mass > 500 GeV", "vbs_jj.delta_eta > 2.5"]

[[outputs]]
name = "vbs_mjj"
expr = "vbs_jj.mass"

[[outputs]]
name = "vbs_detajj"
expr = "vbs_jj.delta_eta"

[[outputs]]
name = "vbs_dphijj"
expr = "vbs_jj.delta_phi"

[[outputs]]
name = "vbs_phij1"
expr = "vbs_jj.leading_phi"

[[outputs]]
name = "vbs_phij2"
expr = "vbs_jj.subleading_phi"

[[outputs]]
name = "vbs_massj1"
expr = "vbs_jj.leading_mass"

[[outputs]]
name = "vbs_massj2"
expr = "vbs_jj.subleading_mass"
"#,
        )
        .expect("parse VBS pair spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        let plan = validate(&spec, &catalogue).expect("validate VBS pair spec");
        let event = vbs_pair_event();
        let row = interpret(&plan, &event)
            .expect("interpret VBS pair event")
            .expect("selected VBS pair event");

        let Value::F64(mjj) = row.get("vbs_mjj").expect("vbs_mjj") else {
            panic!("unexpected vbs_mjj value")
        };
        let Value::F64(detajj) = row.get("vbs_detajj").expect("vbs_detajj") else {
            panic!("unexpected vbs_detajj value")
        };
        let Value::F64(dphijj) = row.get("vbs_dphijj").expect("vbs_dphijj") else {
            panic!("unexpected vbs_dphijj value")
        };
        let Value::F64(phij1) = row.get("vbs_phij1").expect("vbs_phij1") else {
            panic!("unexpected vbs_phij1 value")
        };
        let Value::F64(phij2) = row.get("vbs_phij2").expect("vbs_phij2") else {
            panic!("unexpected vbs_phij2 value")
        };
        let Value::F64(massj1) = row.get("vbs_massj1").expect("vbs_massj1") else {
            panic!("unexpected vbs_massj1 value")
        };
        let Value::F64(massj2) = row.get("vbs_massj2").expect("vbs_massj2") else {
            panic!("unexpected vbs_massj2 value")
        };

        assert!(mjj > 500.0);
        assert!((detajj - 6.0).abs() < 1.0e-6);
        assert!((dphijj - 1.0).abs() < 1.0e-6);
        assert!((phij1 - 0.5).abs() < 1.0e-6);
        assert!((phij2 + 0.5).abs() < 1.0e-6);
        assert!((massj1 - 20.0).abs() < 1.0e-6);
        assert!((massj2 - 30.0).abs() < 1.0e-6);
    }

    #[test]
    fn interpret_zep_vv_uses_met_and_vbs_pair_geometry() {
        let spec = AnalysisSpec::from_toml_str(
            r#"
[analysis]
name = "vbs_zepvv"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = []

[objects.vbs_jet]
source = "Jet"
cuts = ["pt > 50 GeV", "abs(eta) < 4.7"]

[derived.dilepton]
kind = "pair"
object = "good_muon"
selection = "leading_pt"

[derived.vbs_jj]
kind = "pair"
object = "vbs_jet"
selection = "leading_pt"

[regions.signal]
require = [
  "PuppiMET_pt > 30 GeV",
  "zep_vv(dilepton, PuppiMET_pt, PuppiMET_phi, vbs_jj) < 1.0",
]

[[outputs]]
name = "met_pt"
expr = "PuppiMET_pt"

[[outputs]]
name = "vbs_zepvv"
expr = "zep_vv(dilepton, PuppiMET_pt, PuppiMET_phi, vbs_jj)"

[[outputs]]
name = "vbs_ptvv"
expr = "system_met_pt(dilepton, PuppiMET_pt, PuppiMET_phi)"

[[outputs]]
name = "vbs_ptjj"
expr = "vbs_jj.pt"

[[outputs]]
name = "vbs_pttot"
expr = "system_pair_met_pt(dilepton, PuppiMET_pt, PuppiMET_phi, vbs_jj)"

[[outputs]]
name = "vbs_ptbalance"
expr = "system_pair_pt_balance(dilepton, PuppiMET_pt, PuppiMET_phi, vbs_jj)"

[[outputs]]
name = "vbs_dphijjll"
expr = "system_delta_phi(vbs_jj, dilepton)"

[[outputs]]
name = "vv_eta"
expr = "system_met_eta(dilepton, PuppiMET_pt, PuppiMET_phi)"

[[outputs]]
name = "vbs_detavvj1"
expr = "abs(system_met_eta(dilepton, PuppiMET_pt, PuppiMET_phi) - vbs_jj.leading_eta)"

[[outputs]]
name = "vbs_detavvj2"
expr = "abs(system_met_eta(dilepton, PuppiMET_pt, PuppiMET_phi) - vbs_jj.subleading_eta)"

[[outputs]]
name = "vbs_zepmax"
expr = "zep_max(dilepton, vbs_jj)"
"#,
        )
        .expect("parse VBS zep spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        let plan = validate(&spec, &catalogue).expect("validate VBS zep spec");
        let event = vbs_zep_event();
        let row = interpret(&plan, &event)
            .expect("interpret VBS zep event")
            .expect("selected VBS zep event");

        let Value::F64(zepvv) = row.get("vbs_zepvv").expect("vbs_zepvv") else {
            panic!("unexpected vbs_zepvv value")
        };
        let Value::F64(met_pt) = row.get("met_pt").expect("met_pt") else {
            panic!("unexpected met_pt value")
        };
        let Value::F64(ptvv) = row.get("vbs_ptvv").expect("vbs_ptvv") else {
            panic!("unexpected vbs_ptvv value")
        };
        let Value::F64(ptjj) = row.get("vbs_ptjj").expect("vbs_ptjj") else {
            panic!("unexpected vbs_ptjj value")
        };
        let Value::F64(pttot) = row.get("vbs_pttot").expect("vbs_pttot") else {
            panic!("unexpected vbs_pttot value")
        };
        let Value::F64(ptbalance) = row.get("vbs_ptbalance").expect("vbs_ptbalance") else {
            panic!("unexpected vbs_ptbalance value")
        };
        let Value::F64(dphijjll) = row.get("vbs_dphijjll").expect("vbs_dphijjll") else {
            panic!("unexpected vbs_dphijjll value")
        };
        let Value::F64(vv_eta) = row.get("vv_eta").expect("vv_eta") else {
            panic!("unexpected vv_eta value")
        };
        let Value::F64(detavvj1) = row.get("vbs_detavvj1").expect("vbs_detavvj1") else {
            panic!("unexpected vbs_detavvj1 value")
        };
        let Value::F64(detavvj2) = row.get("vbs_detavvj2").expect("vbs_detavvj2") else {
            panic!("unexpected vbs_detavvj2 value")
        };
        let Value::F64(zepmax) = row.get("vbs_zepmax").expect("vbs_zepmax") else {
            panic!("unexpected vbs_zepmax value")
        };
        assert!(zepvv < 1.0);
        assert!((met_pt - 40.0).abs() < 1.0e-6);
        assert!(ptvv > 0.0);
        let jj_px = f64::from(120.0_f32) * f64::from(0.5_f32).cos()
            + f64::from(100.0_f32) * f64::from(-0.5_f32).cos();
        let jj_py = f64::from(120.0_f32) * f64::from(0.5_f32).sin()
            + f64::from(100.0_f32) * f64::from(-0.5_f32).sin();
        let ll_px = f64::from(45.0_f32) * f64::from(0.2_f32).cos()
            + f64::from(40.0_f32) * f64::from(-0.2_f32).cos();
        let ll_py = f64::from(45.0_f32) * f64::from(0.2_f32).sin()
            + f64::from(40.0_f32) * f64::from(-0.2_f32).sin();
        let met_px = f64::from(40.0_f32) * f64::from(1.2_f32).cos();
        let met_py = f64::from(40.0_f32) * f64::from(1.2_f32).sin();
        assert!((pttot - (jj_px + ll_px + met_px).hypot(jj_py + ll_py + met_py)).abs() < 1.0e-6);
        assert!((ptbalance - ((ptvv - ptjj) / ptjj)).abs() < 1.0e-6);
        assert!((dphijjll - delta_phi(jj_py.atan2(jj_px), ll_py.atan2(ll_px))).abs() < 1.0e-6);
        assert!((detavvj1 - (vv_eta - 3.0).abs()).abs() < 1.0e-6);
        assert!((detavvj2 - (vv_eta + 3.0).abs()).abs() < 1.0e-6);
        assert!((zepmax - (0.1_f64 / 6.0)).abs() < 1.0e-6);
    }

    #[test]
    fn interpret_legacy_lepton_rpt_uses_muon_then_electron_order() {
        let spec = AnalysisSpec::from_toml_str(
            r#"
[analysis]
name = "legacy_rpt"
year = "Run2018"

[objects.fake_muon]
source = "Muon"
cuts = []

[objects.fake_electron]
source = "Electron"
cuts = []

[objects.vbs_jet]
source = "Jet"
cuts = ["pt > 40 GeV"]

[derived.vbs_jj]
kind = "pair"
object = "vbs_jet"
selection = "leading_pt"

[regions.signal]
require = ["count(fake_muon) == 1", "count(fake_electron) == 2"]

[[outputs]]
name = "vbs_rpt"
expr = "legacy_lepton_rpt(fake_muon, fake_electron, vbs_jj)"
"#,
        )
        .expect("parse legacy rpt spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        let plan = validate(&spec, &catalogue).expect("validate legacy rpt spec");
        let row = interpret(&plan, &legacy_rpt_event())
            .expect("interpret legacy rpt event")
            .expect("selected legacy rpt event");

        let Value::F64(rpt) = row.get("vbs_rpt").expect("vbs_rpt") else {
            panic!("unexpected vbs_rpt value")
        };
        assert!((rpt - (45.0 * 30.0 / (100.0 * 50.0))).abs() < 1.0e-6);
    }

    #[test]
    fn interpret_pair_constituent_attr_reads_selected_source_attribute() {
        let spec = AnalysisSpec::from_toml_str(
            r#"
[analysis]
name = "pair_constituent_attr"
year = "Run2024"

[objects.vbs_jet]
source = "Jet"
cuts = ["pt > 40 GeV"]

[derived.vbs_jj]
kind = "pair"
object = "vbs_jet"
selection = "leading_pt"

[regions.signal]
require = ["count(vbs_jet) >= 2"]

[[outputs]]
name = "vbs_btagj1"
expr = "pair_leading_attr(vbs_jj, btagUParTAK4B)"

[[outputs]]
name = "vbs_btagj2"
expr = "pair_subleading_attr(vbs_jj, btagUParTAK4B)"
"#,
        )
        .expect("parse pair constituent attr spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV15_CATALOGUE, "v15").expect("parse catalogue");
        let plan = validate(&spec, &catalogue).expect("validate pair constituent attr spec");
        let row = interpret(&plan, &pair_constituent_attr_event())
            .expect("interpret pair constituent attr event")
            .expect("selected pair constituent attr event");

        let Value::F64(btagj1) = row.get("vbs_btagj1").expect("vbs_btagj1") else {
            panic!("unexpected vbs_btagj1 value")
        };
        let Value::F64(btagj2) = row.get("vbs_btagj2").expect("vbs_btagj2") else {
            panic!("unexpected vbs_btagj2 value")
        };
        assert!((btagj1 - 0.7).abs() < 1.0e-6);
        assert!((btagj2 - 0.2).abs() < 1.0e-6);
    }

    #[test]
    fn interpret_met_type1_applies_legacy_shift() {
        let spec = AnalysisSpec::from_toml_str(
            r#"
[analysis]
name = "met_type1"
year = "Run2024"

[objects.clean_jet]
source = "Jet"
cuts = ["pt > 10 GeV"]

[regions.signal]
require = ["count(clean_jet) >= 1"]

[[outputs]]
name = "met_pt_def"
expr = "met_type1_pt(clean_jet, PuppiMET_pt, PuppiMET_phi, pt, mass)"

[[outputs]]
name = "met_phi_def"
expr = "met_type1_phi(clean_jet, PuppiMET_pt, PuppiMET_phi, pt, mass)"
"#,
        )
        .expect("parse Type-1 MET spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV15_CATALOGUE, "v15").expect("parse catalogue");
        let plan = validate(&spec, &catalogue).expect("validate Type-1 MET spec");
        let row = interpret(&plan, &met_type1_event())
            .expect("interpret Type-1 MET event")
            .expect("selected Type-1 MET event");

        let Value::F64(met_pt) = row.get("met_pt_def").expect("met_pt_def") else {
            panic!("unexpected met_pt_def value")
        };
        let Value::F64(met_phi) = row.get("met_phi_def").expect("met_phi_def") else {
            panic!("unexpected met_phi_def value")
        };
        assert!((met_pt - 130.0).abs() < 1.0e-6);
        assert!(met_phi.abs() < 1.0e-6);
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

    fn jec_nominal_event() -> Event {
        let schema = BranchSchema::new([
            BranchSpec::new("nJet", BranchType::U32),
            BranchSpec::new("Jet_area", BranchType::VecF32),
            BranchSpec::new("Jet_eta", BranchType::VecF32),
            BranchSpec::new("Jet_phi", BranchType::VecF32),
            BranchSpec::new("Jet_pt", BranchType::VecF32),
            BranchSpec::new("Jet_rawFactor", BranchType::VecF32),
            BranchSpec::new("Rho_fixedGridRhoFastjetAll", BranchType::F32),
        ])
        .expect("schema");
        Event::from_columns(
            schema,
            [
                ("nJet", BranchColumn::U32(vec![1])),
                ("Jet_area", BranchColumn::VecF32(vec![vec![0.5]])),
                ("Jet_eta", BranchColumn::VecF32(vec![vec![0.2]])),
                ("Jet_phi", BranchColumn::VecF32(vec![vec![1.0]])),
                ("Jet_pt", BranchColumn::VecF32(vec![vec![100.0]])),
                ("Jet_rawFactor", BranchColumn::VecF32(vec![vec![0.1]])),
                ("Rho_fixedGridRhoFastjetAll", BranchColumn::F32(vec![20.0])),
            ],
            0,
        )
        .expect("event")
    }

    fn jer_nominal_event() -> Event {
        let schema = BranchSchema::new([
            BranchSpec::new("nJet", BranchType::U32),
            BranchSpec::new("Jet_area", BranchType::VecF32),
            BranchSpec::new("Jet_eta", BranchType::VecF32),
            BranchSpec::new("Jet_genJetIdx", BranchType::VecI16),
            BranchSpec::new("Jet_phi", BranchType::VecF32),
            BranchSpec::new("Jet_pt", BranchType::VecF32),
            BranchSpec::new("Jet_rawFactor", BranchType::VecF32),
            BranchSpec::new("GenJet_pt", BranchType::VecF32),
            BranchSpec::new("Rho_fixedGridRhoFastjetAll", BranchType::F32),
        ])
        .expect("schema");
        Event::from_columns(
            schema,
            [
                ("nJet", BranchColumn::U32(vec![1])),
                ("Jet_area", BranchColumn::VecF32(vec![vec![0.5]])),
                ("Jet_eta", BranchColumn::VecF32(vec![vec![0.2]])),
                ("Jet_genJetIdx", BranchColumn::VecI16(vec![vec![0]])),
                ("Jet_phi", BranchColumn::VecF32(vec![vec![1.0]])),
                ("Jet_pt", BranchColumn::VecF32(vec![vec![100.0]])),
                ("Jet_rawFactor", BranchColumn::VecF32(vec![vec![0.1]])),
                ("GenJet_pt", BranchColumn::VecF32(vec![vec![90.0]])),
                ("Rho_fixedGridRhoFastjetAll", BranchColumn::F32(vec![20.0])),
            ],
            0,
        )
        .expect("event")
    }

    fn vbs_pair_event() -> Event {
        let schema = BranchSchema::new([
            BranchSpec::new("nJet", BranchType::U32),
            BranchSpec::new("Jet_pt", BranchType::VecF32),
            BranchSpec::new("Jet_eta", BranchType::VecF32),
            BranchSpec::new("Jet_phi", BranchType::VecF32),
            BranchSpec::new("Jet_mass", BranchType::VecF32),
        ])
        .expect("schema");
        Event::from_columns(
            schema,
            [
                ("nJet", BranchColumn::U32(vec![2])),
                ("Jet_pt", BranchColumn::VecF32(vec![vec![120.0, 100.0]])),
                ("Jet_eta", BranchColumn::VecF32(vec![vec![3.0, -3.0]])),
                ("Jet_phi", BranchColumn::VecF32(vec![vec![0.5, -0.5]])),
                ("Jet_mass", BranchColumn::VecF32(vec![vec![20.0, 30.0]])),
            ],
            0,
        )
        .expect("event")
    }

    fn vbs_zep_event() -> Event {
        let schema = BranchSchema::new([
            BranchSpec::new("nMuon", BranchType::U32),
            BranchSpec::new("Muon_pt", BranchType::VecF32),
            BranchSpec::new("Muon_eta", BranchType::VecF32),
            BranchSpec::new("Muon_phi", BranchType::VecF32),
            BranchSpec::new("Muon_mass", BranchType::VecF32),
            BranchSpec::new("nJet", BranchType::U32),
            BranchSpec::new("Jet_pt", BranchType::VecF32),
            BranchSpec::new("Jet_eta", BranchType::VecF32),
            BranchSpec::new("Jet_phi", BranchType::VecF32),
            BranchSpec::new("Jet_mass", BranchType::VecF32),
            BranchSpec::new("PuppiMET_pt", BranchType::F32),
            BranchSpec::new("PuppiMET_phi", BranchType::F32),
        ])
        .expect("schema");
        Event::from_columns(
            schema,
            [
                ("nMuon", BranchColumn::U32(vec![2])),
                ("Muon_pt", BranchColumn::VecF32(vec![vec![45.0, 40.0]])),
                ("Muon_eta", BranchColumn::VecF32(vec![vec![0.1, -0.1]])),
                ("Muon_phi", BranchColumn::VecF32(vec![vec![0.2, -0.2]])),
                ("Muon_mass", BranchColumn::VecF32(vec![vec![0.105, 0.105]])),
                ("nJet", BranchColumn::U32(vec![2])),
                ("Jet_pt", BranchColumn::VecF32(vec![vec![120.0, 100.0]])),
                ("Jet_eta", BranchColumn::VecF32(vec![vec![3.0, -3.0]])),
                ("Jet_phi", BranchColumn::VecF32(vec![vec![0.5, -0.5]])),
                ("Jet_mass", BranchColumn::VecF32(vec![vec![20.0, 20.0]])),
                ("PuppiMET_pt", BranchColumn::F32(vec![40.0])),
                ("PuppiMET_phi", BranchColumn::F32(vec![1.2])),
            ],
            0,
        )
        .expect("event")
    }

    fn legacy_rpt_event() -> Event {
        let schema = BranchSchema::new([
            BranchSpec::new("nMuon", BranchType::U32),
            BranchSpec::new("Muon_pt", BranchType::VecF32),
            BranchSpec::new("nElectron", BranchType::U32),
            BranchSpec::new("Electron_pt", BranchType::VecF32),
            BranchSpec::new("nJet", BranchType::U32),
            BranchSpec::new("Jet_pt", BranchType::VecF32),
            BranchSpec::new("Jet_eta", BranchType::VecF32),
            BranchSpec::new("Jet_phi", BranchType::VecF32),
            BranchSpec::new("Jet_mass", BranchType::VecF32),
        ])
        .expect("schema");
        Event::from_columns(
            schema,
            [
                ("nMuon", BranchColumn::U32(vec![1])),
                ("Muon_pt", BranchColumn::VecF32(vec![vec![45.0]])),
                ("nElectron", BranchColumn::U32(vec![2])),
                ("Electron_pt", BranchColumn::VecF32(vec![vec![30.0, 20.0]])),
                ("nJet", BranchColumn::U32(vec![2])),
                ("Jet_pt", BranchColumn::VecF32(vec![vec![100.0, 50.0]])),
                ("Jet_eta", BranchColumn::VecF32(vec![vec![3.0, -3.0]])),
                ("Jet_phi", BranchColumn::VecF32(vec![vec![0.5, -0.5]])),
                ("Jet_mass", BranchColumn::VecF32(vec![vec![20.0, 20.0]])),
            ],
            0,
        )
        .expect("event")
    }

    fn pair_constituent_attr_event() -> Event {
        let schema = BranchSchema::new([
            BranchSpec::new("nJet", BranchType::U32),
            BranchSpec::new("Jet_pt", BranchType::VecF32),
            BranchSpec::new("Jet_eta", BranchType::VecF32),
            BranchSpec::new("Jet_phi", BranchType::VecF32),
            BranchSpec::new("Jet_mass", BranchType::VecF32),
            BranchSpec::new("Jet_btagUParTAK4B", BranchType::VecF32),
        ])
        .expect("schema");
        Event::from_columns(
            schema,
            [
                ("nJet", BranchColumn::U32(vec![2])),
                ("Jet_pt", BranchColumn::VecF32(vec![vec![100.0, 50.0]])),
                ("Jet_eta", BranchColumn::VecF32(vec![vec![3.0, -3.0]])),
                ("Jet_phi", BranchColumn::VecF32(vec![vec![0.5, -0.5]])),
                ("Jet_mass", BranchColumn::VecF32(vec![vec![20.0, 20.0]])),
                (
                    "Jet_btagUParTAK4B",
                    BranchColumn::VecF32(vec![vec![0.7, 0.2]]),
                ),
            ],
            0,
        )
        .expect("event")
    }

    fn met_type1_event() -> Event {
        let schema = BranchSchema::new([
            BranchSpec::new("nJet", BranchType::U32),
            BranchSpec::new("Jet_pt", BranchType::VecF32),
            BranchSpec::new("Jet_mass", BranchType::VecF32),
            BranchSpec::new("Jet_phi", BranchType::VecF32),
            BranchSpec::new("Jet_muonSubtrFactor", BranchType::VecF32),
            BranchSpec::new("Jet_chEmEF", BranchType::VecF32),
            BranchSpec::new("Jet_neEmEF", BranchType::VecF32),
            BranchSpec::new("PuppiMET_pt", BranchType::F32),
            BranchSpec::new("PuppiMET_phi", BranchType::F32),
        ])
        .expect("schema");
        Event::from_columns(
            schema,
            [
                ("nJet", BranchColumn::U32(vec![2])),
                ("Jet_pt", BranchColumn::VecF32(vec![vec![50.0, 60.0]])),
                ("Jet_mass", BranchColumn::VecF32(vec![vec![20.0, 10.0]])),
                ("Jet_phi", BranchColumn::VecF32(vec![vec![0.0, 1.0]])),
                (
                    "Jet_muonSubtrFactor",
                    BranchColumn::VecF32(vec![vec![0.0, 0.0]]),
                ),
                ("Jet_chEmEF", BranchColumn::VecF32(vec![vec![0.1, 0.5]])),
                ("Jet_neEmEF", BranchColumn::VecF32(vec![vec![0.1, 0.4]])),
                ("PuppiMET_pt", BranchColumn::F32(vec![100.0])),
                ("PuppiMET_phi", BranchColumn::F32(vec![0.0])),
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
