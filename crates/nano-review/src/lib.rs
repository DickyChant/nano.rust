//! Semantic review and repair helpers for agent-authored analysis specs.

use std::collections::{BTreeMap, BTreeSet};

use nano_core::BranchSpec;
use nano_spec::{
    validate, AnalysisSpec, Catalogue, CmpOp, Cut, ModelDef, Quantity, Requirement, SpecError,
    SpecFormat, Unit,
};

const MAX_REPAIR_ITERATIONS: usize = 5;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SemanticDiff {
    pub ok: bool,
    pub validation: DiffValidation,
    pub objects: ObjectDiff,
    pub regions: RegionDiff,
    pub outputs: OutputDiff,
    pub models: ModelDiff,
    pub read_branches: NameDelta,
    pub summary: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DiffValidation {
    pub a: ValidationState,
    pub b: ValidationState,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ValidationState {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct NameDelta {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ObjectDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub cuts_changed: Vec<ObjectCutChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ObjectCutChange {
    pub object: String,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<ValueChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ValueChange {
    pub name: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct RegionDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub requirements_changed: Vec<ValueChange>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct OutputDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub expr_changed: Vec<ValueChange>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ModelDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<ValueChange>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RepairSuggestion {
    pub error_message: String,
    pub kind: RepairKind,
    pub suggestion: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairKind {
    MissingBranch,
    MissingUnit,
    UndefinedObject,
    TypeMismatch,
    Parse,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RepairOutcome {
    pub applied: Vec<AppliedRepair>,
    pub final_spec_text: String,
    pub remaining_errors: Vec<String>,
    pub converged: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AppliedRepair {
    pub error_message: String,
    pub kind: RepairKind,
    pub suggestion: String,
    pub replacement: String,
    pub confidence: f64,
}

struct ValidatedSpec {
    spec: AnalysisSpec,
    plan: nano_spec::ResolvedPlan,
}

enum LoadError {
    Parse(String),
    Validation(Vec<String>),
}

/// Parse, validate, and compare two specs at the semantic IR level.
pub fn semantic_diff(spec_a: &str, spec_b: &str, catalogue: &Catalogue) -> SemanticDiff {
    let a = parse_and_validate(spec_a, catalogue);
    let b = parse_and_validate(spec_b, catalogue);
    let validation = DiffValidation {
        a: validation_state(&a),
        b: validation_state(&b),
    };

    let (Ok(a), Ok(b)) = (a, b) else {
        return SemanticDiff {
            ok: false,
            validation,
            objects: ObjectDiff::default(),
            regions: RegionDiff::default(),
            outputs: OutputDiff::default(),
            models: ModelDiff::default(),
            read_branches: NameDelta::default(),
            summary: Vec::new(),
        };
    };

    let objects = diff_objects(&a.spec, &b.spec);
    let regions = diff_regions(&a.spec, &b.spec);
    let outputs = diff_outputs(&a.spec, &b.spec);
    let models = diff_models(&a.spec, &b.spec);
    let read_branches = diff_names(
        branch_names(a.plan.read_branches.specs()),
        branch_names(b.plan.read_branches.specs()),
    );
    let summary = summarize(&objects, &regions, &outputs, &models, &read_branches);

    SemanticDiff {
        ok: true,
        validation,
        objects,
        regions,
        outputs,
        models,
        read_branches,
        summary,
    }
}

/// Return typed, ranked repair suggestions for validation errors in a spec.
pub fn suggest_repairs(spec_text: &str, catalogue: &Catalogue) -> Vec<RepairSuggestion> {
    let Ok(spec) = parse_spec_auto(spec_text) else {
        return vec![RepairSuggestion {
            error_message: "failed to parse spec text as TOML, YAML, or JSON".to_string(),
            kind: RepairKind::Parse,
            suggestion: "fix the spec syntax before semantic repair".to_string(),
            replacement: None,
            confidence: 0.0,
        }];
    };

    match validate(&spec, catalogue) {
        Ok(_) => Vec::new(),
        Err(errors) => errors
            .iter()
            .map(|error| suggestion_for_error(error, &spec, catalogue))
            .collect(),
    }
}

/// Optionally apply high-confidence textual repairs in a bounded validate-repair loop.
pub fn repair_spec(spec_text: &str, catalogue: &Catalogue, apply: bool) -> RepairOutcome {
    if !apply {
        return RepairOutcome {
            applied: Vec::new(),
            final_spec_text: spec_text.to_string(),
            remaining_errors: suggest_repairs(spec_text, catalogue)
                .into_iter()
                .map(|suggestion| suggestion.error_message)
                .collect(),
            converged: parse_and_validate(spec_text, catalogue).is_ok(),
        };
    }

    let mut current = spec_text.to_string();
    let mut applied = Vec::new();

    for _ in 0..MAX_REPAIR_ITERATIONS {
        if parse_and_validate(&current, catalogue).is_ok() {
            return RepairOutcome {
                applied,
                final_spec_text: current,
                remaining_errors: Vec::new(),
                converged: true,
            };
        }

        let suggestions = suggest_repairs(&current, catalogue);
        let mut changed = false;
        for suggestion in suggestions
            .into_iter()
            .filter(|suggestion| suggestion.confidence >= 0.60)
        {
            let Some(replacement) = suggestion.replacement.clone() else {
                continue;
            };
            let next = apply_replacement(&current, &suggestion, &replacement);
            if next == current {
                continue;
            }
            current = next;
            applied.push(AppliedRepair {
                error_message: suggestion.error_message,
                kind: suggestion.kind,
                suggestion: suggestion.suggestion,
                replacement,
                confidence: suggestion.confidence,
            });
            changed = true;
        }

        if !changed {
            break;
        }
    }

    let remaining_errors = match parse_spec_auto(&current) {
        Ok(spec) => validate(&spec, catalogue)
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|error| error.to_string())
            .collect(),
        Err(error) => vec![error.to_string()],
    };
    let converged = remaining_errors.is_empty();

    RepairOutcome {
        applied,
        final_spec_text: current,
        remaining_errors,
        converged,
    }
}

fn parse_and_validate(spec_text: &str, catalogue: &Catalogue) -> Result<ValidatedSpec, LoadError> {
    let spec = parse_spec_auto(spec_text).map_err(|error| LoadError::Parse(error.to_string()))?;
    let plan = validate(&spec, catalogue).map_err(|errors| {
        LoadError::Validation(
            errors
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>(),
        )
    })?;
    Ok(ValidatedSpec { spec, plan })
}

fn parse_spec_auto(spec_text: &str) -> Result<AnalysisSpec, nano_spec::ParseError> {
    let mut last_error = None;
    for format in [SpecFormat::Toml, SpecFormat::Yaml, SpecFormat::Json] {
        match nano_spec::parse_analysis_spec_with_format(spec_text, format) {
            Ok(spec) => return Ok(spec),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.expect("at least one parser attempted"))
}

fn validation_state(result: &Result<ValidatedSpec, LoadError>) -> ValidationState {
    match result {
        Ok(_) => ValidationState {
            valid: true,
            parse_error: None,
            validation_errors: Vec::new(),
        },
        Err(LoadError::Parse(error)) => ValidationState {
            valid: false,
            parse_error: Some(error.clone()),
            validation_errors: Vec::new(),
        },
        Err(LoadError::Validation(errors)) => ValidationState {
            valid: false,
            parse_error: None,
            validation_errors: errors.clone(),
        },
    }
}

fn diff_objects(a: &AnalysisSpec, b: &AnalysisSpec) -> ObjectDiff {
    let a_objects = a
        .objects
        .iter()
        .map(|object| (object.name.as_str(), object))
        .collect::<BTreeMap<_, _>>();
    let b_objects = b
        .objects
        .iter()
        .map(|object| (object.name.as_str(), object))
        .collect::<BTreeMap<_, _>>();
    let names = diff_names(
        a_objects.keys().map(|name| (*name).to_string()).collect(),
        b_objects.keys().map(|name| (*name).to_string()).collect(),
    );
    let mut cuts_changed = Vec::new();

    for name in a_objects
        .keys()
        .filter(|name| b_objects.contains_key(**name))
    {
        let before = cut_map(&a_objects[name].cuts);
        let after = cut_map(&b_objects[name].cuts);
        let added = after
            .iter()
            .filter(|(key, _)| !before.contains_key(*key))
            .map(|(_, cut)| describe_cut(cut))
            .collect::<Vec<_>>();
        let removed = before
            .iter()
            .filter(|(key, _)| !after.contains_key(*key))
            .map(|(_, cut)| describe_cut(cut))
            .collect::<Vec<_>>();
        let changed = before
            .iter()
            .filter_map(|(key, old)| {
                let new = after.get(key)?;
                (old != new).then(|| ValueChange {
                    name: key.0.clone(),
                    before: describe_quantity(&old.rhs),
                    after: describe_quantity(&new.rhs),
                })
            })
            .collect::<Vec<_>>();

        if !added.is_empty() || !removed.is_empty() || !changed.is_empty() {
            cuts_changed.push(ObjectCutChange {
                object: (*name).to_string(),
                added,
                removed,
                changed,
            });
        }
    }

    ObjectDiff {
        added: names.added,
        removed: names.removed,
        cuts_changed,
    }
}

fn diff_regions(a: &AnalysisSpec, b: &AnalysisSpec) -> RegionDiff {
    let a_regions = by_name(&a.regions, |region| region.name.as_str());
    let b_regions = by_name(&b.regions, |region| region.name.as_str());
    let names = diff_names(
        a_regions.keys().map(|name| (*name).to_string()).collect(),
        b_regions.keys().map(|name| (*name).to_string()).collect(),
    );
    let requirements_changed = a_regions
        .iter()
        .filter_map(|(name, old)| {
            let new = b_regions.get(name)?;
            (old.require != new.require).then(|| ValueChange {
                name: (*name).to_string(),
                before: old
                    .require
                    .iter()
                    .map(describe_requirement)
                    .collect::<Vec<_>>()
                    .join(", "),
                after: new
                    .require
                    .iter()
                    .map(describe_requirement)
                    .collect::<Vec<_>>()
                    .join(", "),
            })
        })
        .collect();

    RegionDiff {
        added: names.added,
        removed: names.removed,
        requirements_changed,
    }
}

fn diff_outputs(a: &AnalysisSpec, b: &AnalysisSpec) -> OutputDiff {
    let a_outputs = by_name(&a.outputs, |output| output.name.as_str());
    let b_outputs = by_name(&b.outputs, |output| output.name.as_str());
    let names = diff_names(
        a_outputs.keys().map(|name| (*name).to_string()).collect(),
        b_outputs.keys().map(|name| (*name).to_string()).collect(),
    );
    let expr_changed = a_outputs
        .iter()
        .filter_map(|(name, old)| {
            let new = b_outputs.get(name)?;
            (old.expr != new.expr).then(|| ValueChange {
                name: (*name).to_string(),
                before: old.expr.to_string(),
                after: new.expr.to_string(),
            })
        })
        .collect();

    OutputDiff {
        added: names.added,
        removed: names.removed,
        expr_changed,
    }
}

fn diff_models(a: &AnalysisSpec, b: &AnalysisSpec) -> ModelDiff {
    let a_models = by_name(&a.models, |model| model.name.as_str());
    let b_models = by_name(&b.models, |model| model.name.as_str());
    let names = diff_names(
        a_models.keys().map(|name| (*name).to_string()).collect(),
        b_models.keys().map(|name| (*name).to_string()).collect(),
    );
    let changed = a_models
        .iter()
        .filter_map(|(name, old)| {
            let new = b_models.get(name)?;
            (old != new).then(|| ValueChange {
                name: (*name).to_string(),
                before: describe_model(old),
                after: describe_model(new),
            })
        })
        .collect();

    ModelDiff {
        added: names.added,
        removed: names.removed,
        changed,
    }
}

fn suggestion_for_error(
    error: &SpecError,
    spec: &AnalysisSpec,
    catalogue: &Catalogue,
) -> RepairSuggestion {
    match error {
        SpecError::MissingBranch { branch, .. } => {
            if let Some((nearest, confidence)) = nearest_branch(branch, catalogue) {
                RepairSuggestion {
                    error_message: error.to_string(),
                    kind: RepairKind::MissingBranch,
                    suggestion: nearest.clone(),
                    replacement: branch_replacement(branch, &nearest),
                    confidence,
                }
            } else {
                unsupported_suggestion(error)
            }
        }
        SpecError::MissingUnit { expected, .. } => RepairSuggestion {
            error_message: error.to_string(),
            kind: RepairKind::MissingUnit,
            suggestion: format!("add unit {expected}"),
            replacement: Some(expected.to_string()),
            confidence: 0.70,
        },
        SpecError::UndefinedObject { object, .. } => {
            if let Some((nearest, confidence)) = nearest_name(
                object,
                spec.objects.iter().map(|object| object.name.as_str()),
            ) {
                RepairSuggestion {
                    error_message: error.to_string(),
                    kind: RepairKind::UndefinedObject,
                    suggestion: nearest.clone(),
                    replacement: Some(nearest),
                    confidence,
                }
            } else {
                unsupported_suggestion(error)
            }
        }
        SpecError::WrongBranchType { expected, .. } => RepairSuggestion {
            error_message: error.to_string(),
            kind: RepairKind::TypeMismatch,
            suggestion: format!("use an expression with expected type: {expected}"),
            replacement: None,
            confidence: 0.0,
        },
        _ => unsupported_suggestion(error),
    }
}

fn unsupported_suggestion(error: &SpecError) -> RepairSuggestion {
    RepairSuggestion {
        error_message: error.to_string(),
        kind: RepairKind::Unsupported,
        suggestion: "manual review required".to_string(),
        replacement: None,
        confidence: 0.0,
    }
}

fn nearest_branch(target: &str, catalogue: &Catalogue) -> Option<(String, f64)> {
    nearest_name(target, catalogue.branch_names())
}

fn nearest_name<'a>(
    target: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<(String, f64)> {
    candidates
        .map(|candidate| {
            let distance = levenshtein(target, candidate);
            let max_len = target.chars().count().max(candidate.chars().count()).max(1);
            let confidence = 1.0 - (distance as f64 / max_len as f64);
            (candidate.to_string(), confidence, distance)
        })
        .max_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| right.0.cmp(&left.0))
        })
        .map(|(candidate, confidence, _)| (candidate, confidence.max(0.0)))
}

fn branch_replacement(missing: &str, nearest: &str) -> Option<String> {
    match (missing.split_once('_'), nearest.split_once('_')) {
        (Some((missing_source, _)), Some((nearest_source, nearest_attr)))
            if missing_source == nearest_source =>
        {
            Some(nearest_attr.to_string())
        }
        _ => Some(nearest.to_string()),
    }
}

fn apply_replacement(spec_text: &str, suggestion: &RepairSuggestion, replacement: &str) -> String {
    match suggestion.kind {
        RepairKind::MissingBranch => {
            let Some(missing) = extract_missing_branch(&suggestion.error_message) else {
                return spec_text.to_string();
            };
            let old = if let (Some((source, old_attr)), Some((new_source, _))) = (
                missing.split_once('_'),
                suggestion.suggestion.split_once('_'),
            ) {
                if source == new_source {
                    old_attr
                } else {
                    missing.as_str()
                }
            } else {
                missing.as_str()
            };
            replace_identifier_token(spec_text, old, replacement)
        }
        RepairKind::UndefinedObject => {
            let Some(object) = extract_undefined_object(&suggestion.error_message) else {
                return spec_text.to_string();
            };
            replace_identifier_token(spec_text, &object, replacement)
        }
        RepairKind::MissingUnit => add_missing_unit(spec_text, replacement),
        _ => spec_text.to_string(),
    }
}

fn add_missing_unit(spec_text: &str, unit: &str) -> String {
    let mut output = Vec::new();
    let mut changed = false;

    for line in spec_text.lines() {
        if changed || !(line.contains("pt") || line.contains("mass") || line.contains("energy")) {
            output.push(line.to_string());
            continue;
        }
        let Some((before_hash, comment)) = line.split_once('#') else {
            let next = add_unit_to_line(line, unit);
            changed = next != line;
            output.push(next);
            continue;
        };
        let next = add_unit_to_line(before_hash, unit);
        changed = next != before_hash;
        output.push(format!("{next}#{comment}"));
    }

    if output.is_empty() {
        spec_text.to_string()
    } else {
        let mut joined = output.join("\n");
        if spec_text.ends_with('\n') {
            joined.push('\n');
        }
        joined
    }
}

fn add_unit_to_line(line: &str, unit: &str) -> String {
    for op in [">=", "<=", "==", "!=", ">", "<"] {
        let Some((lhs, rhs)) = line.split_once(op) else {
            continue;
        };
        if rhs.contains("GeV") {
            return line.to_string();
        }
        let mut chars = rhs.char_indices().peekable();
        while let Some((start, ch)) = chars.next() {
            if !ch.is_ascii_digit() {
                continue;
            }
            let mut end = start + ch.len_utf8();
            while let Some((index, next)) = chars.peek().copied() {
                if next.is_ascii_digit() || next == '.' {
                    end = index + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let mut next = String::new();
            next.push_str(lhs);
            next.push_str(op);
            next.push_str(&rhs[..end]);
            next.push(' ');
            next.push_str(unit);
            next.push_str(&rhs[end..]);
            return next;
        }
    }
    line.to_string()
}

fn replace_identifier_token(input: &str, old: &str, new: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while let Some(offset) = input[index..].find(old) {
        let start = index + offset;
        let end = start + old.len();
        output.push_str(&input[index..start]);
        if is_identifier_boundary(input[..start].chars().last())
            && is_identifier_boundary(input[end..].chars().next())
        {
            output.push_str(new);
        } else {
            output.push_str(old);
        }
        index = end;
    }
    output.push_str(&input[index..]);
    output
}

fn is_identifier_boundary(ch: Option<char>) -> bool {
    !matches!(ch, Some('_' | 'A'..='Z' | 'a'..='z' | '0'..='9'))
}

fn extract_missing_branch(message: &str) -> Option<String> {
    extract_backticked_after(message, "missing branch")
}

fn extract_undefined_object(message: &str) -> Option<String> {
    extract_backticked_after(message, "undefined object")
}

fn extract_backticked_after(message: &str, marker: &str) -> Option<String> {
    let start = message.find(marker)?;
    let rest = &message[start..];
    let first = rest.find('`')?;
    let second = rest[first + 1..].find('`')?;
    Some(rest[first + 1..first + 1 + second].to_string())
}

fn summarize(
    objects: &ObjectDiff,
    regions: &RegionDiff,
    outputs: &OutputDiff,
    models: &ModelDiff,
    read_branches: &NameDelta,
) -> Vec<String> {
    let mut summary = Vec::new();
    for object in &objects.cuts_changed {
        for change in &object.changed {
            summary.push(format!(
                "cut {} {}->{}",
                change.name, change.before, change.after
            ));
        }
        for added in &object.added {
            summary.push(format!("+cut {} {added}", object.object));
        }
        for removed in &object.removed {
            summary.push(format!("-cut {} {removed}", object.object));
        }
    }
    summary.extend(outputs.added.iter().map(|name| format!("+output {name}")));
    summary.extend(outputs.removed.iter().map(|name| format!("-output {name}")));
    summary.extend(
        outputs
            .expr_changed
            .iter()
            .map(|change| format!("output {} {}->{}", change.name, change.before, change.after)),
    );
    summary.extend(regions.added.iter().map(|name| format!("+region {name}")));
    summary.extend(regions.removed.iter().map(|name| format!("-region {name}")));
    summary.extend(models.added.iter().map(|name| format!("+model {name}")));
    summary.extend(models.removed.iter().map(|name| format!("-model {name}")));
    summary.extend(
        read_branches
            .added
            .iter()
            .map(|name| format!("+branch {name}")),
    );
    summary.extend(
        read_branches
            .removed
            .iter()
            .map(|name| format!("-branch {name}")),
    );
    summary
}

fn by_name<'a, T>(items: &'a [T], name: impl Fn(&'a T) -> &'a str) -> BTreeMap<&'a str, &'a T> {
    items.iter().map(|item| (name(item), item)).collect()
}

fn diff_names(a: BTreeSet<String>, b: BTreeSet<String>) -> NameDelta {
    NameDelta {
        added: b.difference(&a).cloned().collect(),
        removed: a.difference(&b).cloned().collect(),
    }
}

fn branch_names(branches: &[BranchSpec]) -> BTreeSet<String> {
    branches.iter().map(|branch| branch.name.clone()).collect()
}

fn cut_map(cuts: &[Cut]) -> BTreeMap<(String, String), &Cut> {
    cuts.iter()
        .map(|cut| ((cut.lhs.to_string(), describe_op(cut.op).to_string()), cut))
        .collect()
}

fn describe_cut(cut: &Cut) -> String {
    format!(
        "{} {} {}",
        cut.lhs,
        describe_op(cut.op),
        describe_quantity(&cut.rhs)
    )
}

fn describe_requirement(requirement: &Requirement) -> String {
    format!(
        "{} {} {}",
        requirement.lhs,
        describe_op(requirement.op),
        describe_quantity(&requirement.rhs)
    )
}

fn describe_quantity(quantity: &Quantity) -> String {
    match quantity.unit {
        Unit::Dimensionless => fmt_f64(quantity.value),
        unit => format!("{} {unit}", fmt_f64(quantity.value)),
    }
}

fn describe_model(model: &ModelDef) -> String {
    format!(
        "{}({}) -> {} inputs=[{}] provider={:?}",
        model.name,
        model.batch,
        model.output,
        model.inputs.join(","),
        model.provider.kind
    )
}

fn describe_op(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
    }
}

fn fmt_f64(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars = b.chars().collect::<Vec<_>>();
    let mut previous = (0..=b_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; b_chars.len() + 1];

    for (i, a_ch) in a.chars().enumerate() {
        current[0] = i + 1;
        for (j, b_ch) in b_chars.iter().enumerate() {
            let substitution = previous[j] + usize::from(a_ch != *b_ch);
            let insertion = current[j] + 1;
            let deletion = previous[j + 1] + 1;
            current[j + 1] = substitution.min(insertion).min(deletion);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
    const MUON: &str = include_str!("../../nano-spec/examples/muon.toml");
    const MUON_BROKEN: &str = include_str!("../../nano-spec/examples/muon_broken.toml");

    fn catalogue() -> Catalogue {
        Catalogue::from_nanoaod_yaml_str(CATALOGUE, "v9").expect("catalogue")
    }

    #[test]
    fn semantic_diff_reports_cut_output_and_read_branch_delta() {
        let variant = MUON.replace(
            "cuts = [\"pt > 30 GeV\", \"abs(eta) < 2.4\"]",
            "cuts = [\"pt > 35 GeV\", \"abs(eta) < 2.4\"]",
        )
            + "\n[[outputs]]\nname = \"lead_muon_phi\"\nexpr = \"leading(good_muon).phi\"\n";

        let diff = semantic_diff(MUON, &variant, &catalogue());

        assert!(diff.ok);
        assert!(diff.objects.added.is_empty());
        assert!(diff.objects.removed.is_empty());
        assert_eq!(diff.objects.cuts_changed.len(), 1);
        assert_eq!(diff.objects.cuts_changed[0].object, "good_muon");
        assert_eq!(
            diff.objects.cuts_changed[0].changed,
            vec![ValueChange {
                name: "good_muon.pt".to_string(),
                before: "30 GeV".to_string(),
                after: "35 GeV".to_string(),
            }]
        );
        assert_eq!(diff.outputs.added, vec!["lead_muon_phi"]);
        assert!(diff.outputs.removed.is_empty());
        assert_eq!(diff.read_branches.added, vec!["Muon_phi"]);
        assert_eq!(diff.read_branches.removed, Vec::<String>::new());
        assert!(diff.summary.contains(&"+output lead_muon_phi".to_string()));
        assert!(diff.summary.contains(&"+branch Muon_phi".to_string()));
    }

    #[test]
    fn suggest_repairs_broken_muon_prefers_muon_eta() {
        let suggestions = suggest_repairs(MUON_BROKEN, &catalogue());

        let branch = suggestions
            .iter()
            .find(|suggestion| suggestion.kind == RepairKind::MissingBranch)
            .expect("branch suggestion");
        assert_eq!(branch.suggestion, "Muon_eta");
        assert_eq!(branch.replacement.as_deref(), Some("eta"));
    }

    #[test]
    fn repair_spec_broken_muon_converges() {
        let outcome = repair_spec(MUON_BROKEN, &catalogue(), true);

        assert!(outcome.converged, "{:?}", outcome.remaining_errors);
        assert!(outcome.final_spec_text.contains("abs(eta) < 2.4"));
        assert!(parse_and_validate(&outcome.final_spec_text, &catalogue()).is_ok());
    }

    #[test]
    fn suggest_repairs_missing_unit_reports_expected_unit() {
        let spec = MUON.replace("pt > 30 GeV", "pt > 30");
        let suggestions = suggest_repairs(&spec, &catalogue());

        let unit = suggestions
            .iter()
            .find(|suggestion| suggestion.kind == RepairKind::MissingUnit)
            .expect("unit suggestion");
        assert_eq!(unit.suggestion, "add unit GeV");
        assert_eq!(unit.replacement.as_deref(), Some("GeV"));
    }

    #[test]
    fn suggest_repairs_undefined_object_reports_nearest_object() {
        let spec = MUON.replace("count(good_muon)", "count(good_moun)");
        let suggestions = suggest_repairs(&spec, &catalogue());

        let object = suggestions
            .iter()
            .find(|suggestion| suggestion.kind == RepairKind::UndefinedObject)
            .expect("object suggestion");
        assert_eq!(object.suggestion, "good_muon");
        assert_eq!(object.replacement.as_deref(), Some("good_muon"));
    }
}
