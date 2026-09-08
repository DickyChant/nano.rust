//! The systematic axis: one derivation shared by the validator, the interpreter,
//! and codegen.
//!
//! Which variations an analysis has is a *domain fact* about the spec, not a
//! property of whichever back-end happens to run it. Deriving it in more than one
//! place is the drift class this crate exists to eliminate — two derivations that
//! disagree silently drop or merge a declared systematic. So the axis is computed
//! here, once, and `validate` rejects a spec whose axis is not well formed rather
//! than leaving that to codegen (which the interpreter never runs).

use std::collections::BTreeMap;
use std::fmt;

use crate::kir::KirProgram;
use crate::{AnalysisSpec, SystematicDef};

/// Where one entry of the systematic axis comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystematicSource {
    /// The always-present unvaried entry.
    Nominal,
    /// A declared weight variation, upward.
    WeightUp,
    /// A declared weight variation, downward.
    WeightDown,
    /// A correction that shifts kinematics, upward.
    ShapeUp,
    /// A correction that shifts kinematics, downward.
    ShapeDown,
}

/// One entry of the analysis systematic axis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystematicVariant {
    /// The name as declared in the spec (`jes`), or `nominal`.
    pub declared: String,
    /// The axis key: an `UpperCamel` Rust variant name (`JesUp`).
    pub variant: String,
    /// The visitor method name (`jes_up`), also the interpreter's weight key.
    pub method: String,
    /// What produced this entry.
    pub source: SystematicSource,
}

impl SystematicVariant {
    /// How to name this entry's declaration in a diagnostic.
    fn describe(&self) -> String {
        let kind = match self.source {
            SystematicSource::Nominal => "the nominal variation",
            SystematicSource::WeightUp | SystematicSource::WeightDown => "weight variation",
            SystematicSource::ShapeUp | SystematicSource::ShapeDown => "shape correction",
        };
        if matches!(self.source, SystematicSource::Nominal) {
            kind.to_string()
        } else {
            format!("{kind} `{}`", self.declared)
        }
    }
}

/// Why a spec's systematic axis is not well formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystematicAxisError {
    /// A declared name cannot become an axis key.
    InvalidName { name: String },
    /// Two declarations claim the same axis key, which would silently merge them.
    DuplicateVariant {
        variant: String,
        first: String,
        second: String,
    },
}

impl fmt::Display for SystematicAxisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName { name } => write!(
                f,
                "systematic name `{name}` cannot be used as a variation identifier"
            ),
            Self::DuplicateVariant {
                variant,
                first,
                second,
            } => write!(f, "{first} and {second} both map to variation `{variant}`"),
        }
    }
}

impl std::error::Error for SystematicAxisError {}

/// A correction that may contribute an up/down pair to the axis.
pub trait NamedSystematicCorrection {
    /// The declared correction name.
    fn name(&self) -> &str;
    /// Whether this correction declares a variation at all.
    fn has_systematic(&self) -> bool {
        true
    }
}

impl NamedSystematicCorrection for crate::ShapeCorrectionDef {
    fn name(&self) -> &str {
        &self.name
    }
}

impl NamedSystematicCorrection for crate::kir::KirShapeCorrection {
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

/// Derive the systematic axis of a spec.
///
/// The order is stable and is the order generated `Systematic::ALL` uses:
/// nominal, declared weight systematics, scale-factor corrections, shape
/// corrections; each non-nominal declaration contributing `Up` then `Down`.
pub fn systematic_axis(spec: &AnalysisSpec) -> Result<Vec<SystematicVariant>, SystematicAxisError> {
    axis_from_parts(
        &spec.systematics,
        &spec.scale_factor_corrections,
        &spec.shape_corrections,
    )
}

/// Derive the systematic axis of a lowered program.
///
/// Lowering must not change the axis; `tests/systematic_axis.rs` holds it to that.
pub fn systematic_axis_from_kir(
    program: &KirProgram,
) -> Result<Vec<SystematicVariant>, SystematicAxisError> {
    axis_from_parts(
        &program.systematics,
        &program.scale_factor_corrections,
        &program.shape_corrections,
    )
}

/// The shared derivation.
///
/// Private on purpose: the two correction lists are separate generic parameters,
/// so a caller that swapped them would still typecheck. `systematic_axis` and
/// `systematic_axis_from_kir` are the only ways in.
fn axis_from_parts(
    systematics: &[SystematicDef],
    scale_factor_corrections: &[impl NamedSystematicCorrection],
    shape_corrections: &[impl NamedSystematicCorrection],
) -> Result<Vec<SystematicVariant>, SystematicAxisError> {
    let mut variants = vec![SystematicVariant {
        declared: "nominal".to_string(),
        variant: "Nominal".to_string(),
        method: "nominal".to_string(),
        source: SystematicSource::Nominal,
    }];

    for systematic in systematics {
        if let SystematicDef::Weight(systematic) = systematic {
            variants.push(variation(&systematic.name, "Up", SystematicSource::WeightUp)?);
            variants.push(variation(
                &systematic.name,
                "Down",
                SystematicSource::WeightDown,
            )?);
        }
    }

    for correction in scale_factor_corrections
        .iter()
        .filter(|correction| correction.has_systematic())
    {
        variants.push(variation(correction.name(), "Up", SystematicSource::WeightUp)?);
        variants.push(variation(
            correction.name(),
            "Down",
            SystematicSource::WeightDown,
        )?);
    }

    for correction in shape_corrections
        .iter()
        .filter(|correction| correction.has_systematic())
    {
        variants.push(variation(correction.name(), "Up", SystematicSource::ShapeUp)?);
        variants.push(variation(
            correction.name(),
            "Down",
            SystematicSource::ShapeDown,
        )?);
    }

    let mut seen: BTreeMap<&str, &SystematicVariant> = BTreeMap::new();
    for entry in &variants {
        if let Some(first) = seen.insert(entry.variant.as_str(), entry) {
            return Err(SystematicAxisError::DuplicateVariant {
                variant: entry.variant.clone(),
                first: first.describe(),
                second: entry.describe(),
            });
        }
    }

    Ok(variants)
}

/// The axis key one declaration and direction produce, if well formed.
///
/// `None` for a name `validate` would have rejected.
pub fn variant_key(name: &str, direction: &str) -> Option<String> {
    Some(format!("{}{direction}", upper_camel(name)?))
}

fn variation(
    name: &str,
    direction: &str,
    source: SystematicSource,
) -> Result<SystematicVariant, SystematicAxisError> {
    let invalid = || SystematicAxisError::InvalidName {
        name: name.to_string(),
    };
    let base = upper_camel(name).ok_or_else(invalid)?;
    let variant = format!("{base}{direction}");
    if !is_ident(&variant) {
        return Err(invalid());
    }
    if !is_ident(name) {
        return Err(invalid());
    }
    let method = format!("{name}_{}", direction.to_ascii_lowercase());
    if !is_ident(&method) {
        return Err(invalid());
    }
    Ok(SystematicVariant {
        declared: name.to_string(),
        variant,
        method,
        source,
    })
}

/// `muon_weight` -> `MuonWeight`; `None` when the name has no usable parts.
fn upper_camel(value: &str) -> Option<String> {
    if !is_ident(value) {
        return None;
    }
    let mut ident = String::new();
    for part in value.split('_') {
        let mut chars = part.chars();
        // An empty part means a doubled or edge underscore; the camel form would
        // silently swallow it and could collide with a different declaration.
        let first = chars.next()?;
        ident.push(first.to_ascii_uppercase());
        ident.extend(chars);
    }
    is_ident(&ident).then_some(ident)
}

fn is_ident(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && !RUST_KEYWORDS.contains(&value)
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ShapeCorrectionDef, WeightSystematicDef};

    fn weight(name: &str) -> SystematicDef {
        SystematicDef::Weight(WeightSystematicDef {
            name: name.to_string(),
            up: 2.0,
            down: 0.5,
        })
    }

    fn shape(name: &str) -> ShapeCorrectionDef {
        ShapeCorrectionDef {
            name: name.to_string(),
            collection: "good_muon".to_string(),
            attr: "pt".to_string(),
            payload: crate::ShapeCorrectionPayload::Scale {
                up: 1.05,
                down: 0.95,
            },
        }
    }

    #[test]
    fn axis_starts_at_nominal_and_pairs_each_declaration() {
        let axis = axis_from_parts(
            &[weight("muon_weight")],
            &[] as &[ShapeCorrectionDef],
            &[shape("jes")],
        )
        .unwrap();
        let names = axis
            .iter()
            .map(|entry| entry.variant.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["Nominal", "MuonWeightUp", "MuonWeightDown", "JesUp", "JesDown"]
        );
        assert_eq!(axis[1].method, "muon_weight_up");
        assert_eq!(axis[3].source, SystematicSource::ShapeUp);
    }

    #[test]
    fn colliding_declarations_are_rejected_not_merged() {
        let error = axis_from_parts(
            &[weight("jes")],
            &[] as &[ShapeCorrectionDef],
            &[shape("jes")],
        )
        .unwrap_err();
        assert_eq!(
            error,
            SystematicAxisError::DuplicateVariant {
                variant: "JesUp".to_string(),
                first: "weight variation `jes`".to_string(),
                second: "shape correction `jes`".to_string(),
            }
        );
    }

    #[test]
    fn names_that_cannot_be_identifiers_are_rejected() {
        for name in ["", "jes-total", "1jes", "jes__total", "_", "type"] {
            assert!(
                axis_from_parts(
                    &[weight(name)],
                    &[] as &[ShapeCorrectionDef],
                    &[] as &[ShapeCorrectionDef],
                )
                .is_err(),
                "expected `{name}` to be rejected"
            );
        }
    }

    #[test]
    fn distinct_names_that_camel_case_alike_still_collide_loudly() {
        // `jes_total` and `jesTotal` both become `JesTotalUp`.
        let error = axis_from_parts(
            &[weight("jes_total"), weight("jesTotal")],
            &[] as &[ShapeCorrectionDef],
            &[] as &[ShapeCorrectionDef],
        )
        .unwrap_err();
        assert!(matches!(error, SystematicAxisError::DuplicateVariant { .. }));
    }
}
