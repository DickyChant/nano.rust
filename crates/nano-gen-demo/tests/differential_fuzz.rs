//! Differential/property evidence for valid specs on the compiled demo path.
//!
//! Seed: `0x4e414e4f5f444946`. Generated cases: 400.
//! The shared generator emits deterministic single-channel specs over real
//! NanoAOD v9 Muon/Electron/Jet branches, with randomized f32 object cuts,
//! two or three count/predicate/derived-object regions, count/count-where/
//! leading-pt/sum-pt/bool/derived outputs, pair-derived objects, nested
//! pair-plus-candidate objects, cross-collection candidate objects, and optional
//! histograms over object or derived attributes with optional weight or pt-shape
//! systematics.
//!
//! This test validates every generated spec, lowers it to KIR, verifies KIR,
//! requires string codegen to succeed, then compares the KIR interpreter against
//! the build-time compiled generated producer for every generated spec over a
//! deterministic synthetic event batch. Rows are normalized to one stable shape;
//! histogram contents are compared for Nominal on nominal-only histograms and
//! Nominal/JesUp/JesDown when a weight or shape systematic is present.
//! Multi-channel union specs, model inference, and combined weight-plus-shape
//! histogram systematics remain outside this dependency-free fuzz slice because
//! both dynamic and compiled backends do not yet share one fully supported path
//! for those combinations.

use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_demo::fuzz::{FuzzCaseResult, FuzzHist1D, FuzzHistVariation, FuzzRow, FuzzValue};
use nano_spec::codegen::generate_producer_source;
use nano_spec::interpret::{
    interpret_and_fill, interpret_and_fill_systematic, interpret_systematic, InterpretedHistograms,
    OutputRow, Value,
};
use nano_spec::{validate, Catalogue};

#[path = "../fuzz_specs.rs"]
mod fuzz_specs;

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const EVENT_COUNT: usize = 32;

#[test]
fn generated_valid_specs_interpret_like_compiled_codegen() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let cases = fuzz_specs::generated_specs();
    let events = synthetic_events();

    let mut validated = 0_usize;
    let mut kir_verified = 0_usize;
    let mut codegen_emitted = 0_usize;
    let mut compiled_compared = 0_usize;
    let mut histogram_cases = 0_usize;
    let mut weight_systematic_cases = 0_usize;
    let mut shape_systematic_cases = 0_usize;
    let mut derived_cases = 0_usize;
    let mut candidate_cases = 0_usize;
    let mut sum_region_cases = 0_usize;
    let mut leading_region_cases = 0_usize;

    for case in &cases {
        let plan = validate(&case.spec, &catalogue).unwrap_or_else(|errors| {
            panic!(
                "generated fuzz spec {} did not validate: {errors:?}\n{:#?}",
                case.index, case.spec
            )
        });
        validated += 1;

        let kir = nano_spec::kir::lower_plan_to_kir(&plan).unwrap_or_else(|error| {
            panic!(
                "generated fuzz spec {} did not lower to KIR: {error}",
                case.index
            )
        });
        nano_spec::kir::verify(&kir).unwrap_or_else(|error| {
            panic!(
                "generated fuzz spec {} produced invalid KIR: {error}",
                case.index
            )
        });
        kir_verified += 1;

        generate_producer_source(&plan).unwrap_or_else(|error| {
            panic!(
                "generated fuzz spec {} was not supported by codegen: {error}",
                case.index
            )
        });
        codegen_emitted += 1;

        let interpreted = interpret_case(&plan, &events, case);
        let compiled = nano_gen_demo::fuzz::run_case(case.index, &events)
            .unwrap_or_else(|error| panic!("compiled fuzz case {} failed: {error}", case.index));

        assert_eq!(compiled, interpreted, "fuzz case {}", case.index);
        compiled_compared += 1;
        histogram_cases += usize::from(case.has_histogram);
        weight_systematic_cases += usize::from(case.has_weight_systematic);
        shape_systematic_cases += usize::from(case.has_shape_correction);
        derived_cases += usize::from(case.has_derived_object);
        candidate_cases += usize::from(case.has_candidate_object);
        sum_region_cases += usize::from(has_sum_region_requirement(case));
        leading_region_cases += usize::from(has_leading_region_requirement(case));
    }

    assert_eq!(cases.len(), fuzz_specs::FUZZ_SPEC_COUNT);
    assert_eq!(validated, cases.len());
    assert_eq!(kir_verified, cases.len());
    assert_eq!(codegen_emitted, cases.len());
    assert_eq!(compiled_compared, cases.len());
    assert!(
        histogram_cases > 0,
        "deterministic generator should include histogram cases"
    );
    assert!(
        weight_systematic_cases > 0,
        "deterministic generator should include weight-systematic histogram cases"
    );
    assert!(
        shape_systematic_cases > 0,
        "deterministic generator should include shape-systematic histogram cases"
    );
    assert!(
        derived_cases > 0,
        "deterministic generator should include derived-object cases"
    );
    assert!(
        candidate_cases > 0,
        "deterministic generator should include candidate-object cases"
    );
    assert!(
        sum_region_cases > 0,
        "deterministic generator should include sum region requirements"
    );
    assert!(
        leading_region_cases > 0,
        "deterministic generator should include leading region requirements"
    );

    eprintln!(
        "differential fuzz seed=0x{seed:016x} generated={generated} validated={validated} kir_verified={kir_verified} codegen_emitted={codegen_emitted} compiled_compared={compiled_compared} histogram_cases={histogram_cases} weight_systematic_cases={weight_systematic_cases} shape_systematic_cases={shape_systematic_cases} derived_cases={derived_cases} candidate_cases={candidate_cases} sum_region_cases={sum_region_cases} leading_region_cases={leading_region_cases}",
        seed = fuzz_specs::FUZZ_SEED,
        generated = cases.len(),
    );
}

#[test]
fn generated_candidate_min_delta_r_regression() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let cases = fuzz_specs::generated_specs();
    let events = synthetic_events();

    let (case, compiled) = cases
        .iter()
        .filter(|case| {
            case.has_candidate_object
                && case
                    .spec
                    .outputs
                    .iter()
                    .any(|output| output.name == "emu_min_delta_r")
        })
        .find_map(|case| {
            let compiled = nano_gen_demo::fuzz::run_case(case.index, &events)
                .expect("compiled regression case failed");
            compiled
                .rows
                .iter()
                .any(|row| row.is_some())
                .then_some((case, compiled))
        })
        .expect("deterministic generator should include a populated emu_min_delta_r case");

    let plan = validate(&case.spec, &catalogue).expect("validate regression spec");
    let interpreted = interpret_case(&plan, &events, case);

    assert_eq!(compiled, interpreted, "fuzz case {}", case.index);
}

fn interpret_case(
    plan: &nano_spec::ResolvedPlan,
    events: &[Event],
    case: &fuzz_specs::GeneratedSpec,
) -> FuzzCaseResult {
    let mut histograms = InterpretedHistograms::new(plan);
    let rows = events
        .iter()
        .enumerate()
        .map(|(entry, event)| {
            if case.has_shape_correction && !case.has_weight_systematic && case.has_histogram {
                let row = interpret_systematic(plan, event, nano_analysis::Systematic::Nominal)
                    .unwrap_or_else(|error| panic!("entry {entry} interpret failed: {error}"))
                    .map(normalize_interpreted_row);
                for systematic in [
                    nano_analysis::Systematic::Nominal,
                    nano_analysis::Systematic::JesUp,
                    nano_analysis::Systematic::JesDown,
                ] {
                    let _ = interpret_and_fill_systematic(plan, event, &mut histograms, systematic)
                        .unwrap_or_else(|error| {
                            panic!("entry {entry} interpret fill {systematic:?} failed: {error}")
                        });
                }
                row
            } else {
                interpret_and_fill(plan, event, &mut histograms)
                    .unwrap_or_else(|error| panic!("entry {entry} interpret failed: {error}"))
                    .map(normalize_interpreted_row)
            }
        })
        .collect::<Vec<_>>();
    let histogram = plan.spec.histograms.first().map(|histogram| {
        let hist = histograms
            .get(&histogram.name)
            .unwrap_or_else(|| panic!("missing interpreted histogram `{}`", histogram.name));
        if case.has_weight_systematic || case.has_shape_correction {
            vec![
                hist_variation("Nominal", hist.get(nano_analysis::Systematic::Nominal)),
                hist_variation("JesUp", hist.get(nano_analysis::Systematic::JesUp)),
                hist_variation("JesDown", hist.get(nano_analysis::Systematic::JesDown)),
            ]
        } else {
            vec![hist_variation(
                "Nominal",
                hist.get(nano_analysis::Systematic::Nominal),
            )]
        }
    });

    FuzzCaseResult { rows, histogram }
}

fn has_sum_region_requirement(case: &fuzz_specs::GeneratedSpec) -> bool {
    case.spec.regions.iter().any(|region| {
        region
            .require
            .iter()
            .any(|requirement| matches!(requirement.lhs, nano_spec::Expr::SumAttr { .. }))
    })
}

fn has_leading_region_requirement(case: &fuzz_specs::GeneratedSpec) -> bool {
    case.spec.regions.iter().any(|region| {
        region
            .require
            .iter()
            .any(|requirement| matches!(requirement.lhs, nano_spec::Expr::LeadingAttr { .. }))
    })
}

fn hist_variation(systematic: &'static str, hist: &nano_analysis::Hist1D) -> FuzzHistVariation {
    FuzzHistVariation {
        systematic,
        hist: FuzzHist1D {
            bins: hist.bins().to_vec(),
            underflow: hist.underflow(),
            overflow: hist.overflow(),
        },
    }
}

fn normalize_interpreted_row(row: OutputRow) -> FuzzRow {
    FuzzRow {
        values: row
            .values
            .into_iter()
            .map(|(name, value)| (name, normalize_value(value)))
            .collect(),
    }
}

fn normalize_value(value: Value) -> FuzzValue {
    match value {
        Value::F64(value) => FuzzValue::F64(value),
        Value::I64(value) => FuzzValue::I64(value),
        Value::U32(value) => FuzzValue::U32(value),
        Value::Bool(value) => FuzzValue::Bool(value),
    }
}

fn synthetic_events() -> Vec<Event> {
    (0..EVENT_COUNT)
        .map(|entry| Event::from_columns(schema(), columns(), entry).unwrap())
        .collect()
}

fn schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
        BranchSpec::new("Muon_mass", BranchType::VecF32),
        BranchSpec::new("Muon_charge", BranchType::VecI32),
        BranchSpec::new("Muon_dxy", BranchType::VecF32),
        BranchSpec::new("Muon_dz", BranchType::VecF32),
        BranchSpec::new("Muon_pfRelIso03_all", BranchType::VecF32),
        BranchSpec::new("nElectron", BranchType::U32),
        BranchSpec::new("Electron_pt", BranchType::VecF32),
        BranchSpec::new("Electron_eta", BranchType::VecF32),
        BranchSpec::new("Electron_phi", BranchType::VecF32),
        BranchSpec::new("Electron_mass", BranchType::VecF32),
        BranchSpec::new("Electron_charge", BranchType::VecI32),
        BranchSpec::new("Electron_dxy", BranchType::VecF32),
        BranchSpec::new("Electron_dz", BranchType::VecF32),
        BranchSpec::new("Electron_pfRelIso03_all", BranchType::VecF32),
        BranchSpec::new("nJet", BranchType::U32),
        BranchSpec::new("Jet_pt", BranchType::VecF32),
        BranchSpec::new("Jet_eta", BranchType::VecF32),
        BranchSpec::new("Jet_phi", BranchType::VecF32),
        BranchSpec::new("Jet_mass", BranchType::VecF32),
        BranchSpec::new("Jet_btagDeepFlavB", BranchType::VecF32),
        BranchSpec::new("Jet_area", BranchType::VecF32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    let n_muon = counts(0);
    let n_electron = counts(1);
    let n_jet = counts(2);
    vec![
        ("nMuon".to_string(), BranchColumn::U32(n_muon.clone())),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec_f32(&n_muon, "Muon", "pt")),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec_f32(&n_muon, "Muon", "eta")),
        ),
        (
            "Muon_phi".to_string(),
            BranchColumn::VecF32(vec_f32(&n_muon, "Muon", "phi")),
        ),
        (
            "Muon_mass".to_string(),
            BranchColumn::VecF32(vec_f32(&n_muon, "Muon", "mass")),
        ),
        (
            "Muon_charge".to_string(),
            BranchColumn::VecI32(vec_i32(&n_muon, "Muon", "charge")),
        ),
        (
            "Muon_dxy".to_string(),
            BranchColumn::VecF32(vec_f32(&n_muon, "Muon", "dxy")),
        ),
        (
            "Muon_dz".to_string(),
            BranchColumn::VecF32(vec_f32(&n_muon, "Muon", "dz")),
        ),
        (
            "Muon_pfRelIso03_all".to_string(),
            BranchColumn::VecF32(vec_f32(&n_muon, "Muon", "pfRelIso03_all")),
        ),
        (
            "nElectron".to_string(),
            BranchColumn::U32(n_electron.clone()),
        ),
        (
            "Electron_pt".to_string(),
            BranchColumn::VecF32(vec_f32(&n_electron, "Electron", "pt")),
        ),
        (
            "Electron_eta".to_string(),
            BranchColumn::VecF32(vec_f32(&n_electron, "Electron", "eta")),
        ),
        (
            "Electron_phi".to_string(),
            BranchColumn::VecF32(vec_f32(&n_electron, "Electron", "phi")),
        ),
        (
            "Electron_mass".to_string(),
            BranchColumn::VecF32(vec_f32(&n_electron, "Electron", "mass")),
        ),
        (
            "Electron_charge".to_string(),
            BranchColumn::VecI32(vec_i32(&n_electron, "Electron", "charge")),
        ),
        (
            "Electron_dxy".to_string(),
            BranchColumn::VecF32(vec_f32(&n_electron, "Electron", "dxy")),
        ),
        (
            "Electron_dz".to_string(),
            BranchColumn::VecF32(vec_f32(&n_electron, "Electron", "dz")),
        ),
        (
            "Electron_pfRelIso03_all".to_string(),
            BranchColumn::VecF32(vec_f32(&n_electron, "Electron", "pfRelIso03_all")),
        ),
        ("nJet".to_string(), BranchColumn::U32(n_jet.clone())),
        (
            "Jet_pt".to_string(),
            BranchColumn::VecF32(vec_f32(&n_jet, "Jet", "pt")),
        ),
        (
            "Jet_eta".to_string(),
            BranchColumn::VecF32(vec_f32(&n_jet, "Jet", "eta")),
        ),
        (
            "Jet_phi".to_string(),
            BranchColumn::VecF32(vec_f32(&n_jet, "Jet", "phi")),
        ),
        (
            "Jet_mass".to_string(),
            BranchColumn::VecF32(vec_f32(&n_jet, "Jet", "mass")),
        ),
        (
            "Jet_btagDeepFlavB".to_string(),
            BranchColumn::VecF32(vec_f32(&n_jet, "Jet", "btagDeepFlavB")),
        ),
        (
            "Jet_area".to_string(),
            BranchColumn::VecF32(vec_f32(&n_jet, "Jet", "area")),
        ),
    ]
}

fn counts(offset: usize) -> Vec<u32> {
    (0..EVENT_COUNT)
        .map(|entry| ((entry + offset) % 5) as u32)
        .collect()
}

fn vec_f32(counts: &[u32], object: &str, attr: &str) -> Vec<Vec<f32>> {
    counts
        .iter()
        .enumerate()
        .map(|(entry, count)| {
            (0..*count as usize)
                .map(|index| value_for(object, attr, entry, index))
                .collect()
        })
        .collect()
}

fn vec_i32(counts: &[u32], object: &str, attr: &str) -> Vec<Vec<i32>> {
    counts
        .iter()
        .enumerate()
        .map(|(entry, count)| {
            (0..*count as usize)
                .map(|index| i32_value_for(object, attr, entry, index))
                .collect()
        })
        .collect()
}

fn value_for(object: &str, attr: &str, entry: usize, index: usize) -> f32 {
    let seed = (entry * 37 + index * 29 + object_offset(object) + attr_offset(attr)) as u32;
    match attr {
        "pt" => 4.0 + (seed % 220) as f32,
        "eta" => -5.0 + (seed % 101) as f32 * 0.1,
        "phi" => -3.2 + (seed % 129) as f32 * 0.05,
        "dxy" => -0.45 + (seed % 91) as f32 * 0.01,
        "dz" => -0.9 + (seed % 181) as f32 * 0.01,
        "pfRelIso03_all" => (seed % 90) as f32 * 0.01,
        "mass" if object == "Muon" => 0.105,
        "mass" if object == "Electron" => 0.0005,
        "mass" => 1.0 + (seed % 90) as f32,
        "btagDeepFlavB" => (seed % 101) as f32 * 0.01,
        "area" => 0.2 + (seed % 130) as f32 * 0.01,
        other => panic!("unsupported synthetic attr `{other}`"),
    }
}

fn i32_value_for(object: &str, attr: &str, entry: usize, index: usize) -> i32 {
    match attr {
        "charge" => {
            if (entry + index + object_offset(object)).is_multiple_of(2) {
                1
            } else {
                -1
            }
        }
        other => panic!("unsupported synthetic i32 attr `{other}`"),
    }
}

fn object_offset(object: &str) -> usize {
    match object {
        "Muon" => 11,
        "Electron" => 23,
        "Jet" => 41,
        other => panic!("unsupported synthetic object `{other}`"),
    }
}

fn attr_offset(attr: &str) -> usize {
    match attr {
        "pt" => 3,
        "eta" => 5,
        "phi" => 6,
        "dxy" => 7,
        "dz" => 13,
        "pfRelIso03_all" => 17,
        "mass" => 19,
        "btagDeepFlavB" => 29,
        "area" => 31,
        other => panic!("unsupported synthetic attr `{other}`"),
    }
}
