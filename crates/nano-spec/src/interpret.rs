//! Runtime interpreter for the validated semantic IR.
//!
//! This is the dynamic counterpart to [`crate::codegen`]: it executes the same
//! object cuts, region requirements, and output expressions directly over an
//! event instead of requiring a compiled producer.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use nano_core::{BranchType, Event, ObjectView};

use crate::{CmpOp, Expr, ObjectDef, ResolvedPlan};

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

type Result<T> = std::result::Result<T, InterpretError>;
type SelectedObjects = HashMap<String, Vec<SelectedObject>>;

#[derive(Debug, Clone, PartialEq)]
struct SelectedObject {
    source_index: usize,
    leading_values: HashMap<String, NumericValue>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum NumericValue {
    F64(f64),
    I64(i64),
    U64(u64),
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
/// `leading(...)` output had no selected object. Model specs are deliberately
/// unsupported in this runtime until inference is implemented for this path.
pub fn interpret(plan: &ResolvedPlan, event: &Event) -> Result<Option<OutputRow>> {
    if !plan.spec.models.is_empty() {
        return Err(InterpretError::Unsupported(
            "models not yet interpreted; use the compiled path".to_string(),
        ));
    }

    let selected = select_objects(plan, event)?;

    for region in &plan.spec.regions {
        for requirement in &region.require {
            let lhs = eval_numeric_expr(&requirement.lhs, &selected, None)?;
            if !compare(lhs.as_f64(), requirement.op, requirement.rhs.value) {
                return Ok(None);
            }
        }
    }

    let mut values = Vec::with_capacity(plan.spec.outputs.len());
    for output in &plan.spec.outputs {
        let Some(value) = eval_output_expr(&output.expr, &selected)? else {
            return Ok(None);
        };
        values.push((output.name.clone(), value));
    }

    Ok(Some(OutputRow::new(values)))
}

fn select_objects(plan: &ResolvedPlan, event: &Event) -> Result<SelectedObjects> {
    let mut selected = HashMap::with_capacity(plan.spec.objects.len());

    for object in &plan.spec.objects {
        let collection = event.collection(&object.source)?;
        let mut objects = Vec::new();

        for item in collection.iter() {
            let mut leading_values = HashMap::new();
            if passes_object_cuts(plan, object, item)? {
                for attr in leading_attrs_for_object(plan, &object.name) {
                    let value = read_object_attr(plan, &object.source, item, &attr)?;
                    leading_values.insert(attr, value);
                }
                objects.push(SelectedObject {
                    source_index: item.index(),
                    leading_values,
                });
            }
        }

        selected.insert(object.name.clone(), objects);
    }

    Ok(selected)
}

fn passes_object_cuts(
    plan: &ResolvedPlan,
    object: &ObjectDef,
    item: &ObjectView<'_>,
) -> Result<bool> {
    for cut in &object.cuts {
        let lhs = eval_object_numeric_expr(plan, &object.name, &object.source, &cut.lhs, item)?;
        if !compare(lhs.as_f64(), cut.op, cut.rhs.value) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn leading_attrs_for_object(plan: &ResolvedPlan, object_name: &str) -> Vec<String> {
    let mut attrs = Vec::new();
    for output in &plan.spec.outputs {
        if let Expr::LeadingAttr { object, attr } = &output.expr {
            if object == object_name && !attrs.contains(attr) {
                attrs.push(attr.clone());
            }
        }
    }
    for region in &plan.spec.regions {
        for requirement in &region.require {
            collect_leading_attrs(&requirement.lhs, object_name, &mut attrs);
        }
    }
    attrs
}

fn collect_leading_attrs(expr: &Expr, object_name: &str, attrs: &mut Vec<String>) {
    match expr {
        Expr::LeadingAttr { object, attr } if object == object_name && !attrs.contains(attr) => {
            attrs.push(attr.clone());
        }
        Expr::Abs(inner) => collect_leading_attrs(inner, object_name, attrs),
        _ => {}
    }
}

fn eval_object_numeric_expr(
    plan: &ResolvedPlan,
    current_object: &str,
    source: &str,
    expr: &Expr,
    item: &ObjectView<'_>,
) -> Result<NumericValue> {
    match expr {
        Expr::Attr { object, attr } if object == current_object => {
            read_object_attr(plan, source, item, attr)
        }
        Expr::Attr { object, .. } => Err(InterpretError::Unsupported(format!(
            "object `{current_object}` cut references `{object}`; this slice only supports cuts on the object being selected"
        ))),
        Expr::Abs(inner) => Ok(eval_object_numeric_expr(
            plan,
            current_object,
            source,
            inner,
            item,
        )?
        .abs()),
        other => Err(InterpretError::Unsupported(format!(
            "object cut expression `{other}` is not supported by the interpreter"
        ))),
    }
}

fn read_object_attr(
    plan: &ResolvedPlan,
    source: &str,
    item: &ObjectView<'_>,
    attr: &str,
) -> Result<NumericValue> {
    let branch = format!("{source}_{attr}");
    let branch_type = plan
        .read_branches
        .find(&branch)
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
        BranchType::VecF32 => Ok(NumericValue::F64(f64::from(item.get::<f32>(attr)?))),
        other => Err(InterpretError::TypeMismatch {
            branch,
            branch_type: other,
            expected: "numeric vector branch",
        }),
    }
}

fn eval_output_expr(expr: &Expr, selected: &SelectedObjects) -> Result<Option<Value>> {
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
        other => Err(InterpretError::Unsupported(format!(
            "output expression `{other}` is not supported by the interpreter"
        ))),
    }
}

fn eval_numeric_expr(
    expr: &Expr,
    selected: &SelectedObjects,
    current: Option<(&str, &SelectedObject)>,
) -> Result<NumericValue> {
    match expr {
        Expr::Attr { object, attr } => {
            let Some((current_object, selected_object)) = current else {
                return Err(InterpretError::Unsupported(format!(
                    "expression `{expr}` needs an object context"
                )));
            };
            if object != current_object {
                return Err(InterpretError::Unsupported(format!(
                    "expression `{expr}` references `{object}` outside the current object `{current_object}`"
                )));
            }
            selected_object
                .leading_values
                .get(attr)
                .copied()
                .ok_or_else(|| {
                    InterpretError::InvalidExpression(format!(
                        "attribute `{attr}` was not materialized for `{object}`"
                    ))
                })
        }
        Expr::Abs(inner) => Ok(eval_numeric_expr(inner, selected, current)?.abs()),
        Expr::Count(object) => {
            let count = selected
                .get(object)
                .ok_or_else(|| InterpretError::MissingObject(object.clone()))?
                .len();
            Ok(NumericValue::U64(count as u64))
        }
        Expr::LeadingAttr { object, attr } => {
            leading_value(selected, object, attr)?.ok_or_else(|| {
                InterpretError::InvalidExpression(format!(
                    "`leading({object}).{attr}` has no selected object"
                ))
            })
        }
    }
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

    fn muon_plan() -> ResolvedPlan {
        let spec = AnalysisSpec::from_toml_str(MUON_SPEC_TOML).expect("parse muon spec");
        let catalogue =
            Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
        validate(&spec, &catalogue).expect("validate muon spec")
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
