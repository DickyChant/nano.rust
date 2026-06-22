use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_multijet_demo::reference::{ReferenceHistograms, ReferenceProducer, ReferenceRow};
use nano_gen_multijet_demo::{GenHistograms, GenRow, GeneratedProducer, Systematic};
use nano_spec::interpret::{interpret_and_fill, InterpretedHistograms, Value};
use nano_spec::{validate, AnalysisSpec, Catalogue};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const MULTIJET_HT_TOML: &str = include_str!("../../nano-spec/examples/multijet_ht.toml");
const MULTIJET_HT_ADL: &str = include_str!("../../nano-spec/examples/multijet_ht.adl");

#[test]
fn multijet_ht_adl_desugars_to_same_plan_as_toml() {
    let toml_spec = AnalysisSpec::from_toml_str(MULTIJET_HT_TOML).expect("parse TOML spec");
    let adl_spec = AnalysisSpec::from_adl_str(MULTIJET_HT_ADL).expect("parse ADL spec");
    assert_eq!(adl_spec, toml_spec, "AnalysisSpec differs");

    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let toml_plan = validate(&toml_spec, &catalogue).expect("validate TOML spec");
    let adl_plan = validate(&adl_spec, &catalogue).expect("validate ADL spec");
    assert_eq!(adl_plan.spec, toml_plan.spec, "plan spec differs");
    assert_eq!(
        adl_plan.read_branches.specs(),
        toml_plan.read_branches.specs(),
        "read branches differ"
    );
}

#[test]
fn generated_multijet_ht_matches_reference_and_interpreter_on_synthetic_events() {
    let spec = AnalysisSpec::from_toml_str(MULTIJET_HT_TOML).expect("parse spec");
    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let plan = validate(&spec, &catalogue).expect("validate spec");
    let mut generated_histograms = GenHistograms::new();
    let mut reference_histograms = ReferenceHistograms::new();
    let mut interpreted_histograms = InterpretedHistograms::new(&plan);

    let expected_selected = [true, false, false, false, true, false, true];
    for (entry, expected_selected) in expected_selected.into_iter().enumerate() {
        let event = synthetic_event(entry);
        let generated = GeneratedProducer::analyze_and_fill(
            &event,
            &mut generated_histograms,
            Systematic::Nominal,
        )
        .unwrap()
        .map(row_bits);
        let reference = ReferenceProducer::analyze_and_fill(&event, &mut reference_histograms)
            .unwrap()
            .map(reference_row_bits);
        let interpreted = interpret_and_fill(&plan, &event, &mut interpreted_histograms)
            .unwrap_or_else(|error| panic!("entry {entry}: {error:?}"))
            .map(|row| {
                (
                    value_f64_bits(row.get("ht").expect("ht")),
                    value_u32(row.get("n_selected_jets").expect("n_selected_jets")),
                    value_f64_bits(row.get("leading_jet_pt").expect("leading_jet_pt")),
                )
            });

        assert_eq!(
            generated, reference,
            "entry {entry}: generated != reference"
        );
        assert_eq!(
            interpreted, reference,
            "entry {entry}: interpreter != reference"
        );
        assert_eq!(
            reference.is_some(),
            expected_selected,
            "entry {entry}: unexpected selection decision"
        );
    }

    assert_eq!(
        generated_histograms.ht, reference_histograms.ht,
        "generated histogram differs from reference"
    );
    assert_eq!(
        interpreted_histograms
            .get("ht")
            .expect("interpreted ht histogram")
            .get("Nominal".to_string()),
        &reference_histograms.ht,
        "interpreted histogram differs from reference"
    );
    assert_eq!(reference_histograms.ht.sumw(), 3.0);
}

fn synthetic_event(entry: usize) -> Event {
    Event::from_columns(schema(), columns(), entry).unwrap()
}

fn schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nJet", BranchType::U32),
        BranchSpec::new("Jet_pt", BranchType::VecF32),
        BranchSpec::new("Jet_eta", BranchType::VecF32),
        BranchSpec::new("Jet_phi", BranchType::VecF32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    vec![
        (
            "nJet".to_string(),
            BranchColumn::U32(vec![4, 4, 6, 5, 5, 4, 6]),
        ),
        (
            "Jet_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![180.0, 140.0, 120.0, 90.0],
                vec![99.0, 98.0, 97.0, 96.0],
                vec![99.0, 98.0, 97.0, 96.0, 95.0, 94.0],
                vec![220.0, 100.0, 90.0, 80.0, 25.0],
                vec![250.0, 120.0, 80.0, 70.0, 60.0],
                vec![150.0, 140.0, 130.0, 120.0],
                vec![400.0, 90.0, 70.0, 40.0, 35.0, 20.0],
            ]),
        ),
        (
            "Jet_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.1, -1.0, 2.0, -2.4],
                vec![0.2, -0.5, 1.3, -2.0],
                vec![0.0, 0.4, -0.8, 1.1, -1.5, 2.2],
                vec![0.0, -0.8, 2.49, -2.4, 0.1],
                vec![0.3, -1.2, 1.8, -2.1, 0.0],
                vec![2.6, -0.2, 1.7, -2.3],
                vec![0.1, -0.3, 2.2, -1.0, 2.49, 0.0],
            ]),
        ),
        (
            "Jet_phi".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.0, 0.4, -1.1, 2.2],
                vec![0.3, -0.2, 1.4, -2.4],
                vec![0.1, -0.5, 2.0, -2.1, 0.7, 1.2],
                vec![0.8, -1.0, 2.2, -0.4, 1.6],
                vec![0.5, -1.8, 2.6, -0.9, 1.1],
                vec![1.0, -1.5, 0.2, 2.8],
                vec![0.6, -0.7, 1.9, -2.8, 0.4, 2.0],
            ]),
        ),
    ]
}

fn row_bits(row: GenRow) -> (u64, u32, u64) {
    (
        row.ht.to_bits(),
        row.n_selected_jets,
        f64::from(row.leading_jet_pt).to_bits(),
    )
}

fn reference_row_bits(row: ReferenceRow) -> (u64, u32, u64) {
    (
        row.ht.to_bits(),
        row.n_selected_jets,
        row.leading_jet_pt.to_bits(),
    )
}

fn value_f64_bits(value: Value) -> u64 {
    match value {
        Value::F64(value) => value.to_bits(),
        other => panic!("expected f64 value, got {other:?}"),
    }
}

fn value_u32(value: Value) -> u32 {
    match value {
        Value::U32(value) => value,
        other => panic!("expected u32 value, got {other:?}"),
    }
}
