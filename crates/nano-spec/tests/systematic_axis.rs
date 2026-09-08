//! The systematic axis has exactly one derivation.
//!
//! A dropped or merged systematic is the silent-analysis-bug class this crate
//! exists to prevent, so "which variations exist" must not be recomputed
//! per back-end. These tests hold the three consumers to one answer:
//!
//! - the **spec** axis (`nano_spec::systematics::systematic_axis`),
//! - the axis the **interpreter** sees after lowering to KIR, and
//! - the `Systematic` enum **codegen** actually emits.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use nano_spec::codegen::generate_producer_source;
use nano_spec::kir::lower_plan_to_kir;
use nano_spec::systematics::{systematic_axis, systematic_axis_from_kir};
use nano_spec::{validate, AnalysisSpec, Catalogue};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const NANOV12_CATALOGUE: &str = include_str!("../../../configs/branches/nanov12.yaml");

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn catalogues() -> Vec<Catalogue> {
    vec![
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse nanov9 catalogue"),
        Catalogue::from_nanoaod_yaml_str(NANOV12_CATALOGUE, "v12")
            .expect("parse nanov12 catalogue"),
    ]
}

/// Every example spec that validates against some shipped catalogue.
fn validated_examples() -> Vec<(String, AnalysisSpec, nano_spec::ResolvedPlan)> {
    let mut cases = Vec::new();
    let mut paths = fs::read_dir(examples_dir())
        .expect("read examples dir")
        .map(|entry| entry.expect("read example entry").path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("toml") | Some("adl")
            )
        })
        .collect::<Vec<_>>();
    paths.sort();

    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("example file name")
            .to_string();
        let Ok(spec) = AnalysisSpec::from_path(&path) else {
            continue;
        };
        for catalogue in catalogues() {
            if let Ok(plan) = validate(&spec, &catalogue) {
                cases.push((name.clone(), spec.clone(), plan));
                break;
            }
        }
    }

    assert!(
        cases.len() > 20,
        "expected the example corpus to be discovered, found {}",
        cases.len()
    );
    cases
}

/// The variant names of the `pub enum Systematic` the generator emitted.
fn emitted_systematic_variants(source: &str) -> Vec<String> {
    let start = source
        .find("pub enum Systematic {")
        .expect("generated source declares a Systematic enum");
    let body = &source[start + "pub enum Systematic {".len()..];
    let end = body.find('}').expect("Systematic enum is closed");
    body[..end]
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_end_matches(',').to_string())
        .collect()
}

#[test]
fn lowering_to_kir_preserves_the_systematic_axis() {
    for (name, spec, plan) in validated_examples() {
        let Ok(program) = lower_plan_to_kir(&plan) else {
            continue;
        };
        let from_spec = systematic_axis(&spec).unwrap_or_else(|error| {
            panic!("{name}: validated spec has a malformed axis: {error}")
        });
        // What the interpreter derives at runtime: from the lowered program, not
        // the surface spec.
        let from_kir = systematic_axis_from_kir(&program).unwrap_or_else(|error| {
            panic!("{name}: lowered program has a malformed axis: {error}")
        });

        assert_eq!(
            from_spec, from_kir,
            "{name}: lowering to KIR changed the systematic axis"
        );
    }
}

#[test]
fn codegen_emits_exactly_the_derived_systematic_axis() {
    let mut checked = 0;
    for (name, spec, plan) in validated_examples() {
        let Ok(source) = generate_producer_source(&plan) else {
            continue;
        };
        let derived = systematic_axis(&spec)
            .unwrap_or_else(|error| panic!("{name}: validated spec has a malformed axis: {error}"))
            .into_iter()
            .map(|entry| entry.variant)
            .collect::<Vec<_>>();

        assert_eq!(
            emitted_systematic_variants(&source),
            derived,
            "{name}: generated Systematic enum disagrees with the derived axis"
        );
        checked += 1;
    }
    assert!(
        checked > 10,
        "expected to check many specs, checked {checked}"
    );
}

#[test]
fn every_declared_variation_gets_its_own_axis_entry() {
    for (name, spec, _) in validated_examples() {
        let axis = systematic_axis(&spec).expect("validated spec has a well-formed axis");
        let keys = axis
            .iter()
            .map(|entry| entry.variant.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            keys.len(),
            axis.len(),
            "{name}: a declared variation shares an axis key with another"
        );

        // Every non-nominal declaration contributes both directions; a spec that
        // silently kept only one side would be an asymmetric systematic.
        let declared = spec.systematics.len()
            + spec
                .scale_factor_corrections
                .iter()
                .filter(|correction| correction.systematic.is_some())
                .count()
            + spec.shape_corrections.len();
        assert!(
            axis.len() >= 1 + 2 * declared.saturating_sub(spec.systematics.len()),
            "{name}: axis is smaller than the declarations that produce it"
        );
    }
}
