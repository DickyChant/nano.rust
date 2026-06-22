//! Differential/property evidence for valid specs on the compiled demo path.
//!
//! Seed: `0x4e414e4f5f444946`. Baseline generated cases: 400.
//! The shared generator emits deterministic single-channel specs over real
//! NanoAOD v9 Muon/Electron/Jet branches, with randomized f32 object cuts,
//! two or three count/predicate/derived-object regions, count/count-where/
//! leading-pt/sum-pt/bool/derived outputs, pair-derived objects, nested
//! pair-plus-candidate objects, cross-collection candidate objects, and optional
//! histograms over object or derived attributes with optional weight, pt-shape,
//! or combined weight-plus-shape systematics. A deterministic subset of
//! otherwise no-derived specs is replaced with mock-model taggers over real Muon
//! or Jet batches; those model specs use the generated score in object cuts,
//! region requirements, leading-score outputs, and targeted score histograms.
//! A targeted union corpus adds multi-channel Muon/Electron/Jet specs with
//! matching output schemas and shared histograms.
//!
//! This test validates every generated spec, lowers it to KIR, verifies KIR,
//! requires string codegen to succeed, then compares the KIR interpreter against
//! the build-time compiled generated producer for every generated spec over a
//! deterministic synthetic event batch. Rows are normalized to one stable shape;
//! histogram contents are compared for Nominal on nominal-only histograms and
//! the per-spec generated Nominal/<declared>Up/<declared>Down variants when a
//! weight or shape systematic is present.
//! Random mock-model specs are included in the same build-time compiled corpus
//! and are compared interpreter == compiled producer using the shared mock score
//! routine through the interpreter and `MockPredictor` through generated code.
//! Multi-channel union specs are compared as rows per event plus their shared
//! histogram contents. Non-mock model providers remain excluded because the
//! dependency-free interpreter intentionally supports only the mock provider.
//! Derived objects under model-aware codegen remain excluded: model-aware
//! string codegen currently rejects them with `derived objects are not yet
//! supported by model-aware codegen`; an ignored minimal repro below pins that
//! capability gap.

use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_demo::fuzz::{
    FuzzCaseResult, FuzzHist1D, FuzzHistVariation, FuzzRow, FuzzUnionCaseResult, FuzzUnionRow,
    FuzzValue,
};
use nano_spec::codegen::generate_producer_source;
use nano_spec::interpret::{
    interpret_and_fill, interpret_and_fill_systematic, interpret_systematic, interpret_union,
    ChannelOutputRow, InterpretedHistograms, OutputRow, Value,
};
use nano_spec::{validate, AnalysisSpec, Catalogue, ChannelDef, ResolvedPlan};
use nano_spec::{ShapeCorrectionDef, SystematicDef};

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
    let mut model_cases = 0_usize;
    let mut sum_region_cases = 0_usize;
    let mut leading_region_cases = 0_usize;
    let skipped = 0_usize;

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
        model_cases += usize::from(case.has_model);
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
        model_cases > 0,
        "deterministic generator should include mock-model cases"
    );
    assert_eq!(model_cases, nano_gen_demo::fuzz::FUZZ_MODEL_CASES);
    assert!(
        sum_region_cases > 0,
        "deterministic generator should include sum region requirements"
    );
    assert!(
        leading_region_cases > 0,
        "deterministic generator should include leading region requirements"
    );

    eprintln!(
        "differential fuzz seed=0x{seed:016x} generated={generated} validated={validated} kir_verified={kir_verified} codegen_emitted={codegen_emitted} compiled_compared={compiled_compared} skipped={skipped} skipped_reasons=[] model_cases={model_cases} histogram_cases={histogram_cases} weight_systematic_cases={weight_systematic_cases} shape_systematic_cases={shape_systematic_cases} derived_cases={derived_cases} candidate_cases={candidate_cases} sum_region_cases={sum_region_cases} leading_region_cases={leading_region_cases}",
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

#[test]
fn generated_union_specs_interpret_like_compiled_codegen() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let cases = fuzz_specs::generated_union_specs();
    let events = synthetic_events();

    let mut compiled_compared = 0_usize;
    let mut histogram_cases = 0_usize;
    for case in &cases {
        let plan = validate(&case.spec, &catalogue).unwrap_or_else(|errors| {
            panic!(
                "generated union fuzz spec {} did not validate: {errors:?}\n{:#?}",
                case.index, case.spec
            )
        });
        let kir = nano_spec::kir::lower_plan_to_kir(&plan).unwrap_or_else(|error| {
            panic!(
                "generated union fuzz spec {} did not lower to KIR: {error}",
                case.index
            )
        });
        nano_spec::kir::verify(&kir).unwrap_or_else(|error| {
            panic!(
                "generated union fuzz spec {} produced invalid KIR: {error}",
                case.index
            )
        });
        generate_producer_source(&plan).unwrap_or_else(|error| {
            panic!(
                "generated union fuzz spec {} was not supported by codegen: {error}",
                case.index
            )
        });

        let interpreted = interpret_union_case(&plan, &events, case);
        let compiled =
            nano_gen_demo::fuzz::run_union_case(case.index, &events).unwrap_or_else(|error| {
                panic!("compiled union fuzz case {} failed: {error}", case.index)
            });
        assert_eq!(compiled, interpreted, "union fuzz case {}", case.index);
        compiled_compared += 1;
        histogram_cases += usize::from(case.has_histogram);
    }

    assert_eq!(cases.len(), fuzz_specs::FUZZ_UNION_SPEC_COUNT);
    assert_eq!(compiled_compared, cases.len());
    assert_eq!(histogram_cases, cases.len());
    assert_eq!(compiled_compared, nano_gen_demo::fuzz::FUZZ_UNION_CASES);
    eprintln!(
        "union differential fuzz seed=0x{seed:016x} generated={generated} compiled_compared={compiled_compared} histogram_cases={histogram_cases}",
        seed = fuzz_specs::FUZZ_SEED,
        generated = cases.len(),
    );
}

#[test]
fn generated_model_histogram_specs_interpret_like_compiled_codegen() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let cases = fuzz_specs::generated_model_histogram_specs();
    let events = synthetic_events();

    let mut compiled_compared = 0_usize;
    for case in &cases {
        let plan = validate(&case.spec, &catalogue).unwrap_or_else(|errors| {
            panic!(
                "generated model-histogram fuzz spec {} did not validate: {errors:?}\n{:#?}",
                case.index, case.spec
            )
        });
        let kir = nano_spec::kir::lower_plan_to_kir(&plan).unwrap_or_else(|error| {
            panic!(
                "generated model-histogram fuzz spec {} did not lower to KIR: {error}",
                case.index
            )
        });
        nano_spec::kir::verify(&kir).unwrap_or_else(|error| {
            panic!(
                "generated model-histogram fuzz spec {} produced invalid KIR: {error}",
                case.index
            )
        });
        generate_producer_source(&plan).unwrap_or_else(|error| {
            panic!(
                "generated model-histogram fuzz spec {} was not supported by codegen: {error}",
                case.index
            )
        });

        let interpreted = interpret_case(&plan, &events, case);
        let compiled = nano_gen_demo::fuzz::run_model_histogram_case(case.index, &events)
            .unwrap_or_else(|error| {
                panic!(
                    "compiled model-histogram fuzz case {} failed: {error}",
                    case.index
                )
            });
        assert_eq!(
            compiled, interpreted,
            "model-histogram fuzz case {}",
            case.index
        );
        compiled_compared += 1;
    }

    assert_eq!(cases.len(), fuzz_specs::FUZZ_MODEL_HISTOGRAM_SPEC_COUNT);
    assert_eq!(compiled_compared, cases.len());
    assert_eq!(
        compiled_compared,
        nano_gen_demo::fuzz::FUZZ_MODEL_HISTOGRAM_CASES
    );
    eprintln!(
        "model-histogram differential fuzz seed=0x{seed:016x} generated={generated} compiled_compared={compiled_compared}",
        seed = fuzz_specs::FUZZ_SEED,
        generated = cases.len(),
    );
}

#[test]
fn generated_weight_shape_specs_interpret_like_compiled_codegen() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let cases = fuzz_specs::generated_weight_shape_specs();
    let events = synthetic_events();

    let mut compiled_compared = 0_usize;
    for case in &cases {
        let plan = validate(&case.spec, &catalogue).unwrap_or_else(|errors| {
            panic!(
                "generated weight+shape fuzz spec {} did not validate: {errors:?}\n{:#?}",
                case.index, case.spec
            )
        });
        let kir = nano_spec::kir::lower_plan_to_kir(&plan).unwrap_or_else(|error| {
            panic!(
                "generated weight+shape fuzz spec {} did not lower to KIR: {error}",
                case.index
            )
        });
        nano_spec::kir::verify(&kir).unwrap_or_else(|error| {
            panic!(
                "generated weight+shape fuzz spec {} produced invalid KIR: {error}",
                case.index
            )
        });
        generate_producer_source(&plan).unwrap_or_else(|error| {
            panic!(
                "generated weight+shape fuzz spec {} was not supported by codegen: {error}",
                case.index
            )
        });

        let interpreted = interpret_case(&plan, &events, case);
        let compiled = nano_gen_demo::fuzz::run_weight_shape_case(case.index, &events)
            .unwrap_or_else(|error| {
                panic!(
                    "compiled weight+shape fuzz case {} failed: {error}",
                    case.index
                )
            });
        assert_eq!(
            compiled, interpreted,
            "weight+shape fuzz case {}",
            case.index
        );
        compiled_compared += 1;
    }

    assert_eq!(cases.len(), fuzz_specs::FUZZ_WEIGHT_SHAPE_SPEC_COUNT);
    assert_eq!(compiled_compared, cases.len());
    assert_eq!(
        compiled_compared,
        nano_gen_demo::fuzz::FUZZ_WEIGHT_SHAPE_CASES
    );
    eprintln!(
        "weight+shape differential fuzz seed=0x{seed:016x} generated={generated} compiled_compared={compiled_compared}",
        seed = fuzz_specs::FUZZ_SEED,
        generated = cases.len(),
    );
}

#[test]
#[ignore = "capability gap: model-aware string codegen rejects derived objects"]
fn derived_under_model_minimal_repro_documents_codegen_gap() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let mut case = fuzz_specs::generated_model_histogram_specs()
        .into_iter()
        .next()
        .expect("deterministic model histogram corpus is non-empty");
    case.spec.name = "derived_under_model_repro".to_string();
    case.spec.derived_objects = vec![nano_spec::DerivedObjectDef {
        name: "tagged_pair".to_string(),
        source: nano_spec::DerivedSource::Pair(nano_spec::ObjectPairDef {
            object: case.spec.objects[1].name.clone(),
            constraints: Vec::new(),
            filters: Vec::new(),
            selection: nano_spec::PairSelection::LeadingPt,
            exclude: Vec::new(),
        }),
    }];
    case.spec.outputs.push(nano_spec::OutputDef {
        name: "tagged_pair_mass".to_string(),
        expr: nano_spec::Expr::Attr {
            object: "tagged_pair".to_string(),
            attr: "mass".to_string(),
        },
    });
    let plan = validate(&case.spec, &catalogue).expect("minimal repro should validate");
    let error = generate_producer_source(&plan).expect_err("model-aware derived codegen gap");
    assert_eq!(
        error.to_string(),
        "derived objects are not yet supported by model-aware codegen"
    );
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
            if case.has_shape_correction && case.has_histogram {
                let row = interpret_systematic(plan, event, "Nominal")
                    .unwrap_or_else(|error| panic!("entry {entry} interpret failed: {error}"))
                    .map(normalize_interpreted_row);
                for systematic in systematic_variants(case) {
                    let _ =
                        interpret_and_fill_systematic(plan, event, &mut histograms, &systematic)
                            .unwrap_or_else(|error| {
                                panic!(
                                    "entry {entry} interpret fill {systematic:?} failed: {error}"
                                )
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
            systematic_variants(case)
                .into_iter()
                .map(|systematic| hist_variation(systematic.clone(), hist.get(systematic)))
                .collect()
        } else {
            vec![hist_variation(
                "Nominal".to_string(),
                hist.get("Nominal".to_string()),
            )]
        }
    });

    FuzzCaseResult { rows, histogram }
}

fn interpret_union_case(
    plan: &ResolvedPlan,
    events: &[Event],
    case: &fuzz_specs::GeneratedSpec,
) -> FuzzUnionCaseResult {
    let mut histograms = InterpretedHistograms::new(plan);
    let rows = events
        .iter()
        .enumerate()
        .map(|(entry, event)| {
            let rows = interpret_union(plan, event)
                .unwrap_or_else(|error| panic!("entry {entry} union interpret failed: {error}"))
                .into_iter()
                .map(normalize_interpreted_union_row)
                .collect::<Vec<_>>();
            for channel in &plan.spec.channels {
                let channel_plan = ResolvedPlan {
                    spec: channel_as_spec(channel, &plan.spec),
                    read_branches: plan.read_branches.clone(),
                };
                let _ = interpret_and_fill(&channel_plan, event, &mut histograms).unwrap_or_else(
                    |error| {
                        panic!(
                            "entry {entry} union channel `{}` histogram fill failed: {error}",
                            channel.name
                        )
                    },
                );
            }
            rows
        })
        .collect::<Vec<_>>();
    let histogram = plan.spec.histograms.first().map(|histogram| {
        let hist = histograms
            .get(&histogram.name)
            .unwrap_or_else(|| panic!("missing interpreted histogram `{}`", histogram.name));
        if case.has_weight_systematic || case.has_shape_correction {
            systematic_variants(case)
                .into_iter()
                .map(|systematic| hist_variation(systematic.clone(), hist.get(systematic)))
                .collect()
        } else {
            vec![hist_variation(
                "Nominal".to_string(),
                hist.get("Nominal".to_string()),
            )]
        }
    });

    FuzzUnionCaseResult { rows, histogram }
}

fn channel_as_spec(channel: &ChannelDef, parent: &AnalysisSpec) -> AnalysisSpec {
    AnalysisSpec {
        name: format!("{}_{}", parent.name, channel.name),
        year: parent.year.clone(),
        objects: channel.objects.clone(),
        derived_objects: channel.derived_objects.clone(),
        models: Vec::new(),
        regions: channel.regions.clone(),
        outputs: channel.outputs.clone(),
        histograms: parent.histograms.clone(),
        weight: parent.weight.clone(),
        systematics: parent.systematics.clone(),
        shape_corrections: parent.shape_corrections.clone(),
        channels: Vec::new(),
    }
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

fn systematic_variants(case: &fuzz_specs::GeneratedSpec) -> Vec<String> {
    let mut variants = vec!["Nominal".to_string()];
    for systematic in &case.spec.systematics {
        if let SystematicDef::Weight(systematic) = systematic {
            variants.push(systematic_variant_name(&systematic.name, "Up"));
            variants.push(systematic_variant_name(&systematic.name, "Down"));
        }
    }
    for ShapeCorrectionDef { name, .. } in &case.spec.shape_corrections {
        variants.push(systematic_variant_name(name, "Up"));
        variants.push(systematic_variant_name(name, "Down"));
    }
    variants
}

fn systematic_variant_name(name: &str, direction: &str) -> String {
    format!("{}{direction}", upper_camel(name))
}

fn upper_camel(value: &str) -> String {
    let mut ident = String::new();
    for part in value.split('_') {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            ident.push(first.to_ascii_uppercase());
            ident.extend(chars);
        }
    }
    ident
}

fn hist_variation(systematic: String, hist: &nano_analysis::Hist1D) -> FuzzHistVariation {
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

fn normalize_interpreted_union_row(row: ChannelOutputRow) -> FuzzUnionRow {
    FuzzUnionRow {
        channel: row.channel,
        values: row
            .row
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
