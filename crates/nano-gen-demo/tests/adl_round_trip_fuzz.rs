//! ADL emitter/parser round-trip fuzzing over the deterministic generated corpus.
//!
//! Seed: `0x4e414e4f5f444946`. Generated cases: 400.
//! Current corpus result: generated=400, round_tripped=371, skipped=29.
//! Counts are asserted in the test body and printed when run with
//! `-- --nocapture`.
//!
//! Skips are limited to real gaps in the hand-written ADL grammar: model
//! bindings, multi-channel unions, and built-in JES/JER systematic enum
//! declarations.

use std::collections::BTreeMap;

use nano_spec::{
    to_adl_string, validate, AnalysisSpec, Catalogue, DerivedSource, Expr, ShapeCorrectionDef,
    SystematicDef,
};

#[allow(dead_code)]
#[path = "../fuzz_specs.rs"]
mod fuzz_specs;

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");

#[test]
fn generated_representable_specs_round_trip_through_adl() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let cases = fuzz_specs::generated_specs();
    let model_cases = cases.iter().filter(|case| case.has_model).count();
    let mut round_tripped = 0_usize;
    let mut skipped = BTreeMap::<&'static str, usize>::new();

    for case in &cases {
        if let Some(reason) = adl_skip_reason(&case.spec) {
            *skipped.entry(reason).or_default() += 1;
            continue;
        }

        let adl = to_adl_string(&case.spec);
        let reparsed = AnalysisSpec::from_adl_str(&adl).unwrap_or_else(|error| {
            panic!(
                "generated fuzz spec {} emitted ADL that did not parse: {error}\nADL:\n{adl}\nSPEC:\n{:#?}",
                case.index, case.spec
            )
        });
        assert_eq!(
            reparsed, case.spec,
            "generated fuzz spec {} ADL round-trip changed the spec\nADL:\n{adl}",
            case.index
        );

        let original_plan = validate(&case.spec, &catalogue).unwrap_or_else(|errors| {
            panic!(
                "generated fuzz spec {} did not validate before ADL round-trip: {errors:?}",
                case.index
            )
        });
        let reparsed_plan = validate(&reparsed, &catalogue).unwrap_or_else(|errors| {
            panic!(
                "generated fuzz spec {} did not validate after ADL round-trip: {errors:?}\nADL:\n{adl}",
                case.index
            )
        });
        assert_eq!(
            reparsed_plan.spec, original_plan.spec,
            "generated fuzz spec {} validated plan spec changed",
            case.index
        );
        assert_eq!(
            reparsed_plan.read_branches.specs(),
            original_plan.read_branches.specs(),
            "generated fuzz spec {} validated read branches changed",
            case.index
        );

        round_tripped += 1;
    }

    eprintln!(
        "adl round-trip fuzz seed=0x{seed:016x} generated={generated} round_tripped={round_tripped} skipped={skipped_total} model_cases={model_cases} skipped_by_reason={skipped:?}",
        seed = fuzz_specs::FUZZ_SEED,
        generated = cases.len(),
        skipped_total = skipped.values().sum::<usize>(),
    );

    assert_eq!(cases.len(), fuzz_specs::FUZZ_SPEC_COUNT);
    assert_eq!(round_tripped, cases.len() - model_cases);
    assert_eq!(skipped.values().sum::<usize>(), model_cases);
    assert_eq!(
        skipped.get("model binding").copied().unwrap_or(0),
        model_cases
    );
}

fn adl_skip_reason(spec: &AnalysisSpec) -> Option<&'static str> {
    if !spec.models.is_empty() {
        return Some("model binding");
    }
    if !spec.channels.is_empty() {
        return Some("multi-channel union");
    }
    if let Some(reason) = systematics_skip_reason(&spec.systematics) {
        return Some(reason);
    }
    if spec
        .shape_corrections
        .iter()
        .any(shape_correction_needs_unsupported_surface)
    {
        return Some("unsupported shape correction");
    }
    if spec_uses_candidate_filter_expr(spec) {
        return Some("candidate filter expression outside candidate object");
    }
    None
}

fn systematics_skip_reason(systematics: &[SystematicDef]) -> Option<&'static str> {
    let weight_count = systematics
        .iter()
        .filter(|systematic| matches!(systematic, SystematicDef::Weight(_)))
        .count();
    let has_unsupported = systematics.iter().any(|systematic| {
        matches!(
            systematic,
            SystematicDef::JesUp
                | SystematicDef::JesDown
                | SystematicDef::JerUp
                | SystematicDef::JerDown
        )
    });
    if has_unsupported {
        return Some("built-in JES/JER systematic");
    }
    if weight_count > 1 {
        return Some("multiple weight systematics");
    }
    if systematics.iter().any(|systematic| {
        !matches!(
            systematic,
            SystematicDef::Nominal | SystematicDef::Weight(_)
        )
    }) {
        return Some("unsupported systematic");
    }
    if systematics
        .iter()
        .any(|systematic| matches!(systematic, SystematicDef::Weight(_)))
        && !systematics
            .iter()
            .any(|systematic| matches!(systematic, SystematicDef::Nominal))
    {
        return Some("weight systematic without nominal");
    }
    None
}

fn shape_correction_needs_unsupported_surface(correction: &ShapeCorrectionDef) -> bool {
    correction.attr != "pt"
}

fn spec_uses_candidate_filter_expr(spec: &AnalysisSpec) -> bool {
    spec.derived_objects
        .iter()
        .any(|object| match &object.source {
            DerivedSource::Pair(pair) => pair
                .filters
                .iter()
                .any(|filter| expr_uses_candidate_filter_expr(&filter.lhs)),
            DerivedSource::Candidate(_) => false,
        })
}

fn expr_uses_candidate_filter_expr(expr: &Expr) -> bool {
    match expr {
        Expr::CandidateMinDeltaR | Expr::CandidateLeadingPt | Expr::CandidateSubleadingPt => true,
        Expr::Binary { lhs, rhs, .. } => {
            expr_uses_candidate_filter_expr(lhs) || expr_uses_candidate_filter_expr(rhs)
        }
        Expr::Abs(inner) | Expr::Sqrt(inner) => expr_uses_candidate_filter_expr(inner),
        Expr::CountWhere { predicate, .. }
        | Expr::All { predicate, .. }
        | Expr::Any { predicate, .. } => expr_uses_candidate_filter_expr(&predicate.lhs),
        Expr::Attr { .. }
        | Expr::Literal(_)
        | Expr::Count(_)
        | Expr::SumAttr { .. }
        | Expr::EitherPairPt { .. }
        | Expr::ClosestMass { .. }
        | Expr::OtherMass { .. }
        | Expr::LeadingAttr { .. }
        | Expr::PairDeltaR
        | Expr::PairLeadingPt
        | Expr::PairSubleadingPt => false,
    }
}
