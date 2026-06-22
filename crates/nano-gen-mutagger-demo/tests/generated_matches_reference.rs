use nano_analysis::Hist1D;
use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_mutagger_demo::reference::{
    ReferenceHistograms, ReferenceProducer, ReferenceRow, MODEL_NAME,
};
use nano_gen_mutagger_demo::{
    mutagger_shape_crossing, mutagger_shape_crossing_base, mutagger_weight_systematic,
};
use nano_gen_mutagger_demo::{GenRow, GeneratedProducer};
use nano_inference::MockPredictor;
use nano_spec::interpret::{
    interpret_and_fill, interpret_and_fill_systematic, InterpretedHistograms, OutputRow, Value,
};
use nano_spec::{
    lower, to_adl_string, validate, AnalysisSpec, Catalogue, ShapeCorrectionDef, SystematicDef,
    WeightSystematicDef,
};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const MUTAGGER_TOML: &str = include_str!("../../nano-spec/examples/mutagger_cr.toml");
const MUTAGGER_ADL: &str = include_str!("../../nano-spec/examples/mutagger_cr.adl");
const MUTAGGER_SHAPE_CROSSING_TOML: &str = r#"
[analysis]
name = "mutagger_shape_crossing"
year = "Run2018"

[objects.good_muon]
source = "Muon"
cuts = [
  "pt > 5 GeV",
  "abs(eta) < 2.4",
]

[objects.tagged_muon]
source = "Muon"
cuts = [
  "pt > 5 GeV",
  "abs(eta) < 2.4",
  "topscore > 0.5",
]

[[model]]
name = "muon_tagger"
inputs = ["Muon_pt", "Muon_eta", "Muon_phi"]
output = "Muon_topscore"
batch = "Muon"

[model.provider]
kind = "mock"

[regions.control]
require = ["count(tagged_muon) >= 1"]

[[outputs]]
name = "n_selected_muons"
expr = "count(good_muon)"

[[outputs]]
name = "n_tagged_muons"
expr = "count(tagged_muon)"

[[outputs]]
name = "leading_muon_pt"
expr = "leading(tagged_muon).pt"

[[outputs]]
name = "leading_muon_score"
expr = "leading(tagged_muon).topscore"

[[histogram]]
name = "leading_muon_score"
expr = "leading(tagged_muon).topscore"
bins = 10
range = [0.0, 1.0]
"#;

#[test]
fn mutagger_cr_adl_matches_toml_including_model_surface() {
    let toml_spec = AnalysisSpec::from_toml_str(MUTAGGER_TOML).expect("parse TOML spec");
    let adl_spec = AnalysisSpec::from_adl_str(MUTAGGER_ADL).expect("parse ADL spec");

    assert_eq!(adl_spec, toml_spec, "ADL and TOML AnalysisSpec differ");
    assert_eq!(adl_spec.models.len(), 1);
    assert_eq!(adl_spec.models, toml_spec.models);
    let emitted_adl = to_adl_string(&toml_spec);
    let emitted_spec = AnalysisSpec::from_adl_str(&emitted_adl).expect("parse emitted ADL spec");
    assert_eq!(
        emitted_spec, toml_spec,
        "model-bearing ADL emitter round-trip changed the spec"
    );

    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let toml_core = lower(&toml_spec, &catalogue).expect("lower TOML spec");
    let adl_core = lower(&adl_spec, &catalogue).expect("lower ADL spec");
    assert_eq!(adl_core, toml_core, "ADL and TOML Core IR differ");

    let toml_plan = validate(&toml_spec, &catalogue).expect("validate TOML spec");
    let adl_plan = validate(&adl_spec, &catalogue).expect("validate ADL spec");
    assert_eq!(
        adl_plan.spec, toml_plan.spec,
        "ADL and TOML plan specs differ"
    );
    assert_eq!(
        adl_plan.read_branches.specs(),
        toml_plan.read_branches.specs(),
        "ADL and TOML plan read branches differ"
    );
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
            .unwrap_or_else(|error| panic!("entry {entry}: interpret failed: {error}"))
            .map(interpreted_row_bits);

        assert_eq!(
            interpreted, generated,
            "entry {entry}: interpreted != generated"
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
    let interpreted_histogram = interpreted_histograms
        .get("leading_muon_pt")
        .expect("interpreted leading_muon_pt histogram")
        .get("Nominal".to_string());
    assert_eq!(
        interpreted_histogram, &generated_histogram,
        "interpreted leading-pt histogram differs from generated"
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

#[test]
fn model_weight_systematic_histogram_fanout_matches_interpreter_and_preserves_nominal() {
    let mut systematic_spec = AnalysisSpec::from_toml_str(MUTAGGER_TOML).expect("parse spec");
    systematic_spec.name = "mutagger_cr_weight_systematic".to_string();
    systematic_spec.systematics = vec![
        SystematicDef::Nominal,
        SystematicDef::Weight(WeightSystematicDef {
            name: "muon_weight".to_string(),
            up: 2.0,
            down: 0.5,
        }),
    ];
    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let systematic_plan = validate(&systematic_spec, &catalogue).expect("validate spec");
    let predictor = MockPredictor::new(MODEL_NAME);
    let mut generated_nominal = Hist1D::new(30, 30.0, 330.0);
    let mut generated_systematic = mutagger_weight_systematic::GenHistograms::new();
    let mut interpreted_systematic = InterpretedHistograms::new(&systematic_plan);

    for entry in 0..8 {
        let event = synthetic_event(entry);
        let nominal_row = GeneratedProducer::analyze(&event, &predictor)
            .unwrap_or_else(|error| panic!("entry {entry}: nominal generated failed: {error}"))
            .inspect(|row| generated_nominal.fill_weighted(f64::from(row.leading_muon_pt), 1.0))
            .map(row_bits);
        let systematic_row = mutagger_weight_systematic::GeneratedProducer::analyze_and_fill(
            &event,
            &mut generated_systematic,
            mutagger_weight_systematic::Systematic::Nominal,
            &predictor,
        )
        .unwrap_or_else(|error| panic!("entry {entry}: systematic generated failed: {error}"))
        .map(systematic_row_bits);
        let interpreted = interpret_and_fill(&systematic_plan, &event, &mut interpreted_systematic)
            .unwrap_or_else(|error| panic!("entry {entry}: interpret failed: {error}"))
            .map(interpreted_row_bits);

        assert_eq!(nominal_row, systematic_row, "entry {entry}");
        assert_eq!(systematic_row, interpreted, "entry {entry}");
    }

    let interpreted = interpreted_systematic
        .get("leading_muon_pt")
        .expect("interpreted leading_muon_pt histogram");
    let generated = &generated_systematic.leading_muon_pt;
    for systematic in [
        mutagger_weight_systematic::Systematic::Nominal,
        mutagger_weight_systematic::Systematic::MuonWeightUp,
        mutagger_weight_systematic::Systematic::MuonWeightDown,
    ] {
        let interpreted_key = format!("{systematic:?}");
        assert_eq!(
            generated.get(systematic),
            interpreted.get(interpreted_key),
            "{systematic:?}"
        );
    }

    assert_eq!(
        generated.get(mutagger_weight_systematic::Systematic::Nominal),
        &generated_nominal
    );
    assert_eq!(
        generated
            .get(mutagger_weight_systematic::Systematic::Nominal)
            .sumw(),
        3.0
    );
    assert_eq!(
        generated
            .get(mutagger_weight_systematic::Systematic::MuonWeightUp)
            .sumw(),
        6.0
    );
    assert_eq!(
        generated
            .get(mutagger_weight_systematic::Systematic::MuonWeightDown)
            .sumw(),
        1.5
    );
}

#[test]
fn model_shape_correction_reruns_inference_per_variation_and_preserves_nominal() {
    let mut shape_spec =
        AnalysisSpec::from_toml_str(MUTAGGER_SHAPE_CROSSING_TOML).expect("parse shape spec");
    shape_spec.shape_corrections = vec![ShapeCorrectionDef {
        name: "muon_pt_shape".to_string(),
        collection: "tagged_muon".to_string(),
        attr: "pt".to_string(),
        up: 1.5,
        down: 0.5,
    }];
    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let shape_plan = validate(&shape_spec, &catalogue).expect("validate shape spec");
    let predictor = MockPredictor::new(MODEL_NAME);

    let no_correction_row = mutagger_shape_crossing_base::GeneratedProducer::analyze(
        &shape_crossing_event(),
        &predictor,
    )
    .expect("base generated analyze")
    .map(shape_base_row_bits);

    let mut generated_histograms = mutagger_shape_crossing::GenHistograms::new();
    let mut interpreted_histograms = InterpretedHistograms::new(&shape_plan);

    let nominal_generated = mutagger_shape_crossing::GeneratedProducer::analyze_and_fill(
        &shape_crossing_event(),
        &mut generated_histograms,
        mutagger_shape_crossing::Systematic::Nominal,
        &predictor,
    )
    .expect("generated nominal")
    .map(shape_row_bits);
    let up_generated = mutagger_shape_crossing::GeneratedProducer::analyze_and_fill(
        &shape_crossing_event(),
        &mut generated_histograms,
        mutagger_shape_crossing::Systematic::MuonPtShapeUp,
        &predictor,
    )
    .expect("generated up")
    .map(shape_row_bits);
    let down_generated = mutagger_shape_crossing::GeneratedProducer::analyze_and_fill(
        &shape_crossing_event(),
        &mut generated_histograms,
        mutagger_shape_crossing::Systematic::MuonPtShapeDown,
        &predictor,
    )
    .expect("generated down")
    .map(shape_row_bits);

    let nominal_interpreted = interpret_and_fill_systematic(
        &shape_plan,
        &shape_crossing_event(),
        &mut interpreted_histograms,
        "Nominal",
    )
    .expect("interpreted nominal")
    .map(interpreted_shape_row_bits);
    let up_interpreted = interpret_and_fill_systematic(
        &shape_plan,
        &shape_crossing_event(),
        &mut interpreted_histograms,
        "MuonPtShapeUp",
    )
    .expect("interpreted up")
    .map(interpreted_shape_row_bits);
    let down_interpreted = interpret_and_fill_systematic(
        &shape_plan,
        &shape_crossing_event(),
        &mut interpreted_histograms,
        "MuonPtShapeDown",
    )
    .expect("interpreted down")
    .map(interpreted_shape_row_bits);

    assert_eq!(nominal_generated, no_correction_row);
    assert_eq!(nominal_generated, nominal_interpreted);
    assert_eq!(up_generated, up_interpreted);
    assert_eq!(down_generated, down_interpreted);
    assert_eq!(nominal_generated, None);
    assert_eq!(down_generated, None);
    let up = up_generated.expect("pt-up variation should pass the score cut");
    assert_eq!(up.0, 1);
    assert_eq!(up.1, 1);
    assert!(f32::from_bits(up.3) > 0.5);

    let generated = &generated_histograms.leading_muon_score;
    let interpreted = interpreted_histograms
        .get("leading_muon_score")
        .expect("interpreted leading_muon_score histogram");
    for systematic in [
        mutagger_shape_crossing::Systematic::Nominal,
        mutagger_shape_crossing::Systematic::MuonPtShapeUp,
        mutagger_shape_crossing::Systematic::MuonPtShapeDown,
    ] {
        let key = format!("{systematic:?}");
        assert_eq!(generated.get(systematic), interpreted.get(key));
    }
    assert_eq!(
        generated
            .get(mutagger_shape_crossing::Systematic::Nominal)
            .sumw(),
        0.0
    );
    assert_eq!(
        generated
            .get(mutagger_shape_crossing::Systematic::MuonPtShapeUp)
            .sumw(),
        1.0
    );
    assert_eq!(
        generated
            .get(mutagger_shape_crossing::Systematic::MuonPtShapeDown)
            .sumw(),
        0.0
    );
}

fn synthetic_event(entry: usize) -> Event {
    Event::from_columns(schema(), columns(), entry).unwrap()
}

fn shape_crossing_event() -> Event {
    Event::from_columns(
        schema(),
        [
            ("nMuon", BranchColumn::U32(vec![1])),
            ("Muon_pt", BranchColumn::VecF32(vec![vec![10.75]])),
            ("Muon_eta", BranchColumn::VecF32(vec![vec![0.1]])),
            ("Muon_phi", BranchColumn::VecF32(vec![vec![0.2]])),
        ],
        0,
    )
    .unwrap()
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

fn systematic_row_bits(row: mutagger_weight_systematic::GenRow) -> (u32, u32, u64) {
    (
        row.n_selected_muons,
        row.n_tagged_muons,
        f64::from(row.leading_muon_pt).to_bits(),
    )
}

fn interpreted_row_bits(row: OutputRow) -> (u32, u32, u64) {
    (
        output_u32(&row, "n_selected_muons"),
        output_u32(&row, "n_tagged_muons"),
        output_f64(&row, "leading_muon_pt").to_bits(),
    )
}

fn shape_base_row_bits(row: mutagger_shape_crossing_base::GenRow) -> (u32, u32, u32, u32) {
    (
        row.n_selected_muons,
        row.n_tagged_muons,
        row.leading_muon_pt.to_bits(),
        row.leading_muon_score.to_bits(),
    )
}

fn shape_row_bits(row: mutagger_shape_crossing::GenRow) -> (u32, u32, u32, u32) {
    (
        row.n_selected_muons,
        row.n_tagged_muons,
        row.leading_muon_pt.to_bits(),
        row.leading_muon_score.to_bits(),
    )
}

fn interpreted_shape_row_bits(row: OutputRow) -> (u32, u32, u32, u32) {
    (
        output_u32(&row, "n_selected_muons"),
        output_u32(&row, "n_tagged_muons"),
        (output_f64(&row, "leading_muon_pt") as f32).to_bits(),
        (output_f64(&row, "leading_muon_score") as f32).to_bits(),
    )
}

fn output_u32(row: &OutputRow, name: &str) -> u32 {
    match row
        .get(name)
        .unwrap_or_else(|| panic!("missing output `{name}`"))
    {
        Value::U32(value) => value,
        other => panic!("output `{name}` has unexpected value {other:?}"),
    }
}

fn output_f64(row: &OutputRow, name: &str) -> f64 {
    match row
        .get(name)
        .unwrap_or_else(|| panic!("missing output `{name}`"))
    {
        Value::F64(value) => value,
        other => panic!("output `{name}` has unexpected value {other:?}"),
    }
}
