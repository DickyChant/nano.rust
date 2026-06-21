//! Runtime interpreter for the validated semantic IR.
//!
//! This is the dynamic counterpart to [`crate::codegen`]: it executes the same
//! object cuts, region requirements, and output expressions directly over an
//! event instead of requiring a compiled producer.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use nano_core::{BranchType, Event, ObjectView};

use crate::{
    ArithOp, CmpOp, Cut, DerivedObjectDef, DerivedSource, Expr, ObjectCandidateDef, ObjectDef,
    ObjectPairDef, PairConstraint, PairSelection, ResolvedPlan,
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
type DerivedObjects = HashMap<String, Option<DerivedObject>>;

#[derive(Debug, Clone, PartialEq)]
struct SelectedObject {
    source_index: usize,
    leading_values: HashMap<String, NumericValue>,
}

#[derive(Debug, Clone, PartialEq)]
struct DerivedObject {
    mass: f64,
    pt: f64,
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
    let derived = derive_objects(plan, &selected)?;

    for region in &plan.spec.regions {
        for requirement in &region.require {
            let lhs = eval_numeric_expr(&requirement.lhs, &selected, &derived, None)?;
            if !compare(lhs.as_f64(), requirement.op, requirement.rhs.value) {
                return Ok(None);
            }
        }
    }

    let mut values = Vec::with_capacity(plan.spec.outputs.len());
    for output in &plan.spec.outputs {
        let Some(value) = eval_output_expr(&output.expr, &selected, &derived)? else {
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

fn derive_objects(plan: &ResolvedPlan, selected: &SelectedObjects) -> Result<DerivedObjects> {
    let mut derived = HashMap::with_capacity(plan.spec.derived_objects.len());
    for object in ordered_derived_objects(plan)? {
        let value = match &object.source {
            DerivedSource::Pair(pair) => derive_pair(pair, selected, &derived)?,
            DerivedSource::Candidate(candidate) => derive_candidate(candidate, selected, &derived)?,
        };
        derived.insert(object.name.clone(), value);
    }
    Ok(derived)
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
    order.sort_by(|&left, &right| {
        attr_f64(&objects[right], "pt").total_cmp(&attr_f64(&objects[left], "pt"))
    });

    let target = match &pair.selection {
        PairSelection::LeadingPt => None,
        PairSelection::NearestMass { target } => Some(target.value),
    };
    let mut best = None;
    let mut best_diff = None::<f64>;
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
        } else if let Some(object) = derived_object(derived, item)? {
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

fn derived_object<'a>(
    derived: &'a DerivedObjects,
    name: &str,
) -> Result<Option<&'a DerivedObject>> {
    derived
        .get(name)
        .map(Option::as_ref)
        .ok_or_else(|| InterpretError::MissingObject(name.to_string()))
}

fn ordered_derived_objects(plan: &ResolvedPlan) -> Result<Vec<&DerivedObjectDef>> {
    fn visit<'a>(
        plan: &'a ResolvedPlan,
        object: &'a DerivedObjectDef,
        visiting: &mut Vec<String>,
        visited: &mut Vec<String>,
        ordered: &mut Vec<&'a DerivedObjectDef>,
    ) -> Result<()> {
        if visited.contains(&object.name) {
            return Ok(());
        }
        if visiting.contains(&object.name) {
            return Err(InterpretError::InvalidExpression(format!(
                "derived object dependency cycle includes `{}`",
                object.name
            )));
        }
        visiting.push(object.name.clone());
        for dependency in derived_dependencies(object) {
            if let Some(dep) = plan
                .spec
                .derived_objects
                .iter()
                .find(|derived| derived.name == dependency)
            {
                visit(plan, dep, visiting, visited, ordered)?;
            }
        }
        visiting.pop();
        visited.push(object.name.clone());
        ordered.push(object);
        Ok(())
    }

    let mut ordered = Vec::new();
    let mut visiting = Vec::new();
    let mut visited = Vec::new();
    for object in &plan.spec.derived_objects {
        visit(plan, object, &mut visiting, &mut visited, &mut ordered)?;
    }
    Ok(ordered)
}

fn derived_dependencies(object: &DerivedObjectDef) -> Vec<String> {
    match &object.source {
        DerivedSource::Pair(pair) => pair.exclude.clone(),
        DerivedSource::Candidate(candidate) => candidate.items.clone(),
    }
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
            collect_selected_attrs(&requirement.lhs, object_name, &mut attrs);
        }
    }
    for output in &plan.spec.outputs {
        collect_selected_attrs(&output.expr, object_name, &mut attrs);
    }
    for derived in &plan.spec.derived_objects {
        match &derived.source {
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
        Expr::Literal(value) => Ok(NumericValue::F64(*value)),
        Expr::Binary { op, lhs, rhs } => {
            let lhs = eval_object_numeric_expr(plan, current_object, source, lhs, item)?.as_f64();
            let rhs = eval_object_numeric_expr(plan, current_object, source, rhs, item)?.as_f64();
            Ok(NumericValue::F64(eval_arithmetic(*op, lhs, rhs)))
        }
        Expr::Abs(inner) => Ok(eval_object_numeric_expr(
            plan,
            current_object,
            source,
            inner,
            item,
        )?
        .abs()),
        Expr::Sqrt(inner) => Ok(NumericValue::F64(
            eval_object_numeric_expr(plan, current_object, source, inner, item)?
                .as_f64()
                .sqrt(),
        )),
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
