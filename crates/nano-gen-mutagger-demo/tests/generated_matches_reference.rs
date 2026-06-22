use nano_analysis::Hist1D;
use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_mutagger_demo::reference::{
    ReferenceHistograms, ReferenceProducer, ReferenceRow, MODEL_NAME,
};
use nano_gen_mutagger_demo::{GenRow, GeneratedProducer};
use nano_inference::MockPredictor;
use nano_spec::interpret::{interpret_and_fill, InterpretError, InterpretedHistograms};
use nano_spec::{validate, AnalysisSpec, Catalogue};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const MUTAGGER_TOML: &str = include_str!("../../nano-spec/examples/mutagger_cr.toml");
const MUTAGGER_ADL: &str = include_str!("../../nano-spec/examples/mutagger_cr.adl");

#[test]
fn mutagger_cr_adl_documents_current_model_surface_gap() {
    let toml_spec = AnalysisSpec::from_toml_str(MUTAGGER_TOML).expect("parse TOML spec");
    let adl_spec = AnalysisSpec::from_adl_str(MUTAGGER_ADL).expect("parse scoped ADL spec");

    assert_eq!(toml_spec.models.len(), 1);
    assert!(
        adl_spec.models.is_empty(),
        "current ADL grammar should not silently invent model bindings"
    );
    assert_ne!(
        adl_spec, toml_spec,
        "ADL and TOML cannot round-trip while ADL has no model surface"
    );

    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    validate(&toml_spec, &catalogue).expect("validate TOML spec");
    validate(&adl_spec, &catalogue).expect("validate scoped ADL spec");
}

#[test]
fn generated_mutagger_cr_matches_reference_on_synthetic_events() {
    let spec = AnalysisSpec::from_toml_str(MUTAGGER_TOML).expect("parse spec");
    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let plan = validate(&spec, &catalogue).expect("validate spec");
    let predictor = MockPredictor::new(MODEL_NAME);
    let mut generated_histogram = Hist1D::new(30, 30.0, 330.0);
    let mut reference_histograms = ReferenceHistograms::new();
    let mut interpreted_histograms = InterpretedHistograms::new(&plan);

    let expected_selected = [true, false, true, false, false, true, false, false];
    for (entry, expected_selected) in expected_selected.into_iter().enumerate() {
        let event = synthetic_event(entry);
        let generated = GeneratedProducer::analyze(&event, &predictor)
            .unwrap_or_else(|error| panic!("entry {entry}: generated failed: {error}"))
            .inspect(|row| generated_histogram.fill_weighted(f64::from(row.leading_muon_pt), 1.0))
            .map(row_bits);
        let reference =
            ReferenceProducer::analyze_and_fill(&event, &predictor, &mut reference_histograms)
                .unwrap_or_else(|error| panic!("entry {entry}: reference failed: {error}"))
                .map(reference_row_bits);
        let interpreted = interpret_and_fill(&plan, &event, &mut interpreted_histograms)
            .expect_err("interpreter should report the current model interpretation gap");

        assert_eq!(
            interpreted,
            InterpretError::Unsupported(
                "models not yet interpreted; use the compiled path".to_string()
            )
        );
        assert_eq!(
            generated, reference,
            "entry {entry}: generated != reference"
        );
        assert_eq!(
            reference.is_some(),
            expected_selected,
            "entry {entry}: unexpected selection decision"
        );
    }

    assert_eq!(
        generated_histogram, reference_histograms.leading_muon_pt,
        "generated leading-pt histogram differs from reference"
    );
    assert_eq!(reference_histograms.leading_muon_pt.sumw(), 3.0);
    assert_eq!(
        reference_histograms.leading_muon_pt.bins(),
        &[
            0.0, 2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ]
    );
}

fn synthetic_event(entry: usize) -> Event {
    Event::from_columns(schema(), columns(), entry).unwrap()
}

fn schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    vec![
        (
            "nMuon".to_string(),
            BranchColumn::U32(vec![3, 2, 2, 1, 0, 3, 2, 2]),
        ),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![45.0, 20.0, 36.0],
                vec![31.0, 29.0],
                vec![60.0, 42.0],
                vec![30.0],
                vec![],
                vec![80.0, 35.0, 32.0],
                vec![320.0, 35.0],
                vec![55.0, 34.0],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.1, 0.2, -2.5],
                vec![2.39, 0.0],
                vec![2.41, -1.1],
                vec![0.0],
                vec![],
                vec![0.3, -0.4, 1.2],
                vec![0.2, -2.39],
                vec![0.1, 2.5],
            ]),
        ),
        (
            "Muon_phi".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.01, 1.5, -2.0],
                vec![0.7, -0.2],
                vec![1.2, -2.4],
                vec![0.4],
                vec![],
                vec![2.2, -1.7, 0.5],
                vec![-0.8, 2.1],
                vec![-2.8, 0.6],
            ]),
        ),
    ]
}

fn row_bits(row: GenRow) -> (u32, u32, u64) {
    (
        row.n_selected_muons,
        row.n_tagged_muons,
        f64::from(row.leading_muon_pt).to_bits(),
    )
}

fn reference_row_bits(row: ReferenceRow) -> (u32, u32, u64) {
    (
        row.n_selected_muons,
        row.n_tagged_muons,
        row.leading_muon_pt.to_bits(),
    )
}
