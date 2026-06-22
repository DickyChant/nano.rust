use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_zmumu_demo::reference::{ReferenceHistograms, ReferenceProducer, ReferenceRow};
use nano_gen_zmumu_demo::{GenHistograms, GenRow, GeneratedProducer, Systematic};
use nano_spec::interpret::{interpret_and_fill, InterpretedHistograms, Value};
use nano_spec::{validate, AnalysisSpec, Catalogue};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const ZMUMU_TOML: &str = include_str!("../../nano-spec/examples/zmumu_cr.toml");
const ZMUMU_ADL: &str = include_str!("../../nano-spec/examples/zmumu_cr.adl");

#[test]
fn zmumu_cr_adl_desugars_to_same_plan_as_toml() {
    let toml_spec = AnalysisSpec::from_toml_str(ZMUMU_TOML).expect("parse TOML spec");
    let adl_spec = AnalysisSpec::from_adl_str(ZMUMU_ADL).expect("parse ADL spec");
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
fn generated_zmumu_cr_matches_reference_and_interpreter_on_synthetic_events() {
    let spec = AnalysisSpec::from_toml_str(ZMUMU_TOML).expect("parse spec");
    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let plan = validate(&spec, &catalogue).expect("validate spec");
    let mut generated_histograms = GenHistograms::new();
    let mut reference_histograms = ReferenceHistograms::new();
    let mut interpreted_histograms = InterpretedHistograms::new(&plan);

    for entry in 0..7 {
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
                    value_bits(row.get("dimuon_mass").expect("dimuon_mass")),
                    value_bits(row.get("dimuon_pt").expect("dimuon_pt")),
                    value_bits(row.get("leading_muon_pt").expect("leading_muon_pt")),
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
    }

    assert_eq!(
        generated_histograms.dimuon_mass, reference_histograms.dimuon_mass,
        "generated histogram differs from reference"
    );
    assert_eq!(
        interpreted_histograms
            .get("dimuon_mass")
            .expect("interpreted dimuon_mass histogram")
            .get("Nominal".to_string()),
        &reference_histograms.dimuon_mass,
        "interpreted histogram differs from reference"
    );
    assert_eq!(reference_histograms.dimuon_mass.sumw(), 2.0);
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
        BranchSpec::new("Muon_mass", BranchType::VecF32),
        BranchSpec::new("Muon_charge", BranchType::VecI32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    vec![
        (
            "nMuon".to_string(),
            BranchColumn::U32(vec![2, 2, 3, 3, 2, 2, 4]),
        ),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![46.0, 45.0],
                vec![60.0, 50.0],
                vec![80.0, 35.0, 30.0],
                vec![30.0, 28.0, 27.0],
                vec![24.0, 80.0],
                vec![50.0, 40.0],
                vec![70.0, 68.0, 45.0, 42.0],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.0, 0.0],
                vec![0.1, -0.2],
                vec![0.0, 0.1, -0.1],
                vec![0.0, 0.0, 0.1],
                vec![0.0, 0.0],
                vec![0.0, 2.5],
                vec![0.2, -0.3, 0.0, 0.0],
            ]),
        ),
        (
            "Muon_phi".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.0, 2.7],
                vec![0.4, -2.4],
                vec![0.0, 2.0, std::f32::consts::PI],
                vec![0.0, 0.4, 0.8],
                vec![0.0, std::f32::consts::PI],
                vec![0.0, std::f32::consts::PI],
                vec![0.2, -2.2, 0.0, std::f32::consts::PI],
            ]),
        ),
        (
            "Muon_mass".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.105, 0.105],
                vec![0.105, 0.105],
                vec![0.105, 0.105, 0.105],
                vec![0.105, 0.105, 0.105],
                vec![0.105, 0.105],
                vec![0.105, 0.105],
                vec![0.105, 0.105, 0.105, 0.105],
            ]),
        ),
        (
            "Muon_charge".to_string(),
            BranchColumn::VecI32(vec![
                vec![1, -1],
                vec![1, 1],
                vec![1, -1, -1],
                vec![1, -1, 1],
                vec![1, -1],
                vec![1, -1],
                vec![1, -1, 1, -1],
            ]),
        ),
    ]
}

fn row_bits(row: GenRow) -> (u64, u64, u64) {
    (
        row.dimuon_mass.to_bits(),
        row.dimuon_pt.to_bits(),
        f64::from(row.leading_muon_pt).to_bits(),
    )
}

fn reference_row_bits(row: ReferenceRow) -> (u64, u64, u64) {
    (
        row.dimuon_mass.to_bits(),
        row.dimuon_pt.to_bits(),
        row.leading_muon_pt.to_bits(),
    )
}

fn value_bits(value: Value) -> u64 {
    match value {
        Value::F64(value) => value.to_bits(),
        other => panic!("expected f64 value, got {other:?}"),
    }
}
