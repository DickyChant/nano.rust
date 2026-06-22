use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nano_analysis::Hist1D;
use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_corrections::{CorrectionSet, Value as CorrectionValue};
use nano_gen_demo::GeneratedProducer;
use nano_io::writer::{write_events, OutputBranch};
use nano_producers::{MuonProducer, MuonSkimRow};
use nano_spec::interpret::{
    interpret, interpret_and_fill, interpret_and_fill_systematic, InterpretedHistograms, Value,
};
use nano_spec::{validate, AnalysisSpec, Catalogue};
use nano_workflow::{
    merge_partials, muon_schema, plan_workflow_with_kernel_id, run_chunk, ExecutionMode, Executor,
    KernelRegistry, RunChunkRequest,
};

use nano_gen_demo::muon_hist_nominal::Systematic as MuonHistNominalSystematic;
use nano_gen_demo::muon_hist_shape_correction::Systematic as ShapeSystematic;
use nano_gen_demo::muon_hist_shape_nominal::Systematic as ShapeNominalSystematic;
use nano_gen_demo::muon_hist_weight_systematic::Systematic as WeightSystematic;
use nano_gen_demo::muon_sf::Systematic as SfSystematic;

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const SELECTION_ALL_SPEC: &str = include_str!("../../nano-spec/examples/selection_all.toml");
const MUON_HIST_NOMINAL_SPEC: &str =
    include_str!("../../nano-spec/examples/muon_hist_nominal.toml");
const MUON_HIST_WEIGHT_SYSTEMATIC_SPEC: &str =
    include_str!("../../nano-spec/examples/muon_hist_weight_systematic.toml");
const MUON_HIST_SHAPE_NOMINAL_SPEC: &str =
    include_str!("../../nano-spec/examples/muon_hist_shape_nominal.toml");
const MUON_HIST_SHAPE_CORRECTION_SPEC: &str =
    include_str!("../../nano-spec/examples/muon_hist_shape_correction.toml");
const MUON_SF_SPEC: &str = include_str!("../../nano-spec/examples/muon_sf.toml");

#[test]
fn generated_muon_producer_matches_handwritten_producer_on_synthetic_events() {
    for entry in 0..5 {
        let event = synthetic_event(entry);

        let generated = GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt));
        let handwritten = MuonProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt));

        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

#[test]
fn generated_selection_all_matches_handwritten_reference() {
    for entry in 0..selection_columns_len() {
        let event = selection_event(entry);
        let generated = nano_gen_demo::selection_all::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row.n_good_muon);
        let handwritten = handwritten_all(&event).map(|row| row.n_good_muon);
        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

#[test]
fn generated_charge_balance_matches_handwritten_reference() {
    for entry in 0..selection_columns_len() {
        let event = selection_event(entry);
        let generated = nano_gen_demo::selection_charge_balance::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row.n_good_muon);
        let handwritten = handwritten_charge_balance(&event).map(|row| row.n_good_muon);
        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

#[test]
fn generated_sip3d_arithmetic_matches_handwritten_reference() {
    for entry in 0..selection_columns_len() {
        let event = selection_event(entry);
        let generated = nano_gen_demo::selection_sip3d::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row.n_good_muon);
        let handwritten = handwritten_sip3d(&event).map(|row| row.n_good_muon);
        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

#[test]
fn generated_pair_dr_filter_matches_handwritten_reference() {
    for entry in 0..selection_columns_len() {
        let event = selection_event(entry);
        let generated = nano_gen_demo::selection_pair_dr::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row.z_mass.to_bits());
        let handwritten = handwritten_pair_dr(&event).map(|row| row.z_mass.to_bits());
        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

#[test]
fn interpreted_selection_all_matches_generated_code() {
    let spec = AnalysisSpec::from_toml_str(SELECTION_ALL_SPEC).unwrap();
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let plan = validate(&spec, &catalogue).unwrap();

    for entry in 0..selection_columns_len() {
        let event = selection_event(entry);
        let generated = nano_gen_demo::selection_all::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row.n_good_muon);
        let interpreted =
            interpret(&plan, &event)
                .unwrap()
                .map(|row| match row.get("n_good_muon").unwrap() {
                    Value::U32(value) => value,
                    value => panic!("unexpected interpreted value {value:?}"),
                });
        assert_eq!(generated, interpreted, "entry {entry}");
    }
}

#[test]
fn weight_systematic_histogram_fanout_matches_interpreter_and_preserves_nominal() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let nominal_spec = AnalysisSpec::from_toml_str(MUON_HIST_NOMINAL_SPEC).unwrap();
    let systematic_spec = AnalysisSpec::from_toml_str(MUON_HIST_WEIGHT_SYSTEMATIC_SPEC).unwrap();
    let nominal_plan = validate(&nominal_spec, &catalogue).unwrap();
    let systematic_plan = validate(&systematic_spec, &catalogue).unwrap();
    let mut generated_nominal = nano_gen_demo::muon_hist_nominal::GenHistograms::new();
    let mut generated_systematic = nano_gen_demo::muon_hist_weight_systematic::GenHistograms::new();
    let mut interpreted_systematic = InterpretedHistograms::new(&systematic_plan);

    for entry in 0..5 {
        let event = synthetic_event(entry);
        let nominal_row = nano_gen_demo::muon_hist_nominal::GeneratedProducer::analyze_and_fill(
            &event,
            &mut generated_nominal,
            MuonHistNominalSystematic::Nominal,
        )
        .unwrap()
        .map(|row| row.n_good_muon);
        let systematic_row =
            nano_gen_demo::muon_hist_weight_systematic::GeneratedProducer::analyze_and_fill(
                &event,
                &mut generated_systematic,
                WeightSystematic::Nominal,
            )
            .unwrap()
            .map(|row| row.n_good_muon);
        let interpreted_row =
            interpret_and_fill(&systematic_plan, &event, &mut interpreted_systematic)
                .unwrap()
                .map(|row| match row.get("n_good_muon").unwrap() {
                    Value::U32(value) => value,
                    value => panic!("unexpected interpreted value {value:?}"),
                });
        let nominal_interpreted = interpret(&nominal_plan, &event).unwrap().map(|row| {
            match row.get("n_good_muon").unwrap() {
                Value::U32(value) => value,
                value => panic!("unexpected interpreted value {value:?}"),
            }
        });

        assert_eq!(nominal_row, systematic_row, "entry {entry}");
        assert_eq!(systematic_row, interpreted_row, "entry {entry}");
        assert_eq!(nominal_row, nominal_interpreted, "entry {entry}");
    }

    let interpreted = interpreted_systematic
        .get("n_good_muon_hist")
        .expect("interpreted histogram");
    let generated = &generated_systematic.n_good_muon_hist;
    for systematic in [
        WeightSystematic::Nominal,
        WeightSystematic::MuonWeightUp,
        WeightSystematic::MuonWeightDown,
    ] {
        let interpreted_key = format!("{systematic:?}");
        assert_eq!(
            generated.get(systematic),
            interpreted.get(interpreted_key),
            "{systematic:?}"
        );
    }

    assert_eq!(
        generated_nominal.n_good_muon_hist,
        *generated.get(WeightSystematic::Nominal)
    );
    assert_eq!(generated.get(WeightSystematic::Nominal).sumw(), 3.0);
    assert_eq!(generated.get(WeightSystematic::MuonWeightUp).sumw(), 6.0);
    assert_eq!(generated.get(WeightSystematic::MuonWeightDown).sumw(), 1.5);
}

#[test]
fn shape_correction_recomputes_rows_and_histograms_per_variation() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let nominal_spec = AnalysisSpec::from_toml_str(MUON_HIST_SHAPE_NOMINAL_SPEC).unwrap();
    let shape_spec = AnalysisSpec::from_toml_str(MUON_HIST_SHAPE_CORRECTION_SPEC).unwrap();
    validate(&nominal_spec, &catalogue).unwrap();
    let shape_plan = validate(&shape_spec, &catalogue).unwrap();
    let mut generated_nominal = nano_gen_demo::muon_hist_shape_nominal::GenHistograms::new();
    let mut generated_shape = nano_gen_demo::muon_hist_shape_correction::GenHistograms::new();
    let mut interpreted_shape = InterpretedHistograms::new(&shape_plan);

    let mut generated_rows = Vec::new();
    let mut interpreted_rows = Vec::new();
    let mut nominal_rows = Vec::new();
    for entry in 0..5 {
        let event = synthetic_event(entry);
        let nominal_row =
            nano_gen_demo::muon_hist_shape_nominal::GeneratedProducer::analyze_and_fill(
                &event,
                &mut generated_nominal,
                ShapeNominalSystematic::Nominal,
            )
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt.to_bits()));
        nominal_rows.push(nominal_row);

        for systematic in [
            ShapeSystematic::Nominal,
            ShapeSystematic::JesUp,
            ShapeSystematic::JesDown,
        ] {
            let generated =
                nano_gen_demo::muon_hist_shape_correction::GeneratedProducer::analyze_and_fill(
                    &event,
                    &mut generated_shape,
                    systematic,
                )
                .unwrap()
                .map(|row| (row.n_good_muon, row.lead_muon_pt.to_bits()));
            let interpreted = interpret_and_fill_systematic(
                &shape_plan,
                &event,
                &mut interpreted_shape,
                &format!("{systematic:?}"),
            )
            .unwrap()
            .map(|row| {
                let n_good_muon = match row.get("n_good_muon").unwrap() {
                    Value::U32(value) => value,
                    value => panic!("unexpected interpreted count {value:?}"),
                };
                let lead_muon_pt = match row.get("lead_muon_pt").unwrap() {
                    Value::F64(value) => (value as f32).to_bits(),
                    value => panic!("unexpected interpreted leading pt {value:?}"),
                };
                (n_good_muon, lead_muon_pt)
            });
            assert_eq!(generated, interpreted, "entry {entry} {systematic:?}");
            generated_rows.push((entry, systematic, generated));
            interpreted_rows.push((entry, systematic, interpreted));
        }
    }

    for systematic in [
        ShapeSystematic::Nominal,
        ShapeSystematic::JesUp,
        ShapeSystematic::JesDown,
    ] {
        let interpreted_key = format!("{systematic:?}");
        assert_eq!(
            generated_shape.n_good_muon_hist.get(systematic),
            interpreted_shape
                .get("n_good_muon_hist")
                .expect("interpreted histogram")
                .get(interpreted_key),
            "{systematic:?}"
        );
    }

    let shape_nominal_rows = generated_rows
        .iter()
        .filter_map(|(entry, systematic, row)| {
            (*systematic == ShapeSystematic::Nominal).then_some((*entry, *row))
        })
        .collect::<Vec<_>>();
    for (entry, row) in shape_nominal_rows {
        assert_eq!(row, nominal_rows[entry], "nominal row entry {entry}");
    }
    assert_eq!(
        generated_shape
            .n_good_muon_hist
            .get(ShapeSystematic::Nominal),
        &generated_nominal.n_good_muon_hist
    );

    let entry_one_nominal = generated_rows
        .iter()
        .find(|(entry, systematic, _)| *entry == 1 && *systematic == ShapeSystematic::Nominal)
        .and_then(|(_, _, row)| *row);
    let entry_one_up = generated_rows
        .iter()
        .find(|(entry, systematic, _)| *entry == 1 && *systematic == ShapeSystematic::JesUp)
        .and_then(|(_, _, row)| *row);
    let entry_zero_nominal = generated_rows
        .iter()
        .find(|(entry, systematic, _)| *entry == 0 && *systematic == ShapeSystematic::Nominal)
        .and_then(|(_, _, row)| *row);
    let entry_zero_down = generated_rows
        .iter()
        .find(|(entry, systematic, _)| *entry == 0 && *systematic == ShapeSystematic::JesDown)
        .and_then(|(_, _, row)| *row);

    assert_eq!(entry_one_nominal, None);
    assert!(
        entry_one_up.is_some(),
        "JesUp should migrate entry 1 above threshold"
    );
    assert!(entry_zero_nominal.is_some());
    assert_eq!(
        entry_zero_down, None,
        "JesDown should migrate entry 0 below threshold"
    );
    assert_ne!(
        generated_shape.n_good_muon_hist.get(ShapeSystematic::JesUp),
        generated_shape
            .n_good_muon_hist
            .get(ShapeSystematic::Nominal)
    );
    assert_ne!(
        generated_shape
            .n_good_muon_hist
            .get(ShapeSystematic::JesDown),
        generated_shape
            .n_good_muon_hist
            .get(ShapeSystematic::Nominal)
    );
    assert_eq!(generated_rows, interpreted_rows);
}

#[test]
fn scale_factor_weight_uses_correctionlib_in_generated_and_interpreter() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let nominal_spec = AnalysisSpec::from_toml_str(MUON_HIST_NOMINAL_SPEC).unwrap();
    let sf_spec = AnalysisSpec::from_toml_str(MUON_SF_SPEC).unwrap();
    let nominal_plan = validate(&nominal_spec, &catalogue).unwrap();
    let sf_plan = validate(&sf_spec, &catalogue).unwrap();

    let mut generated_nominal = nano_gen_demo::muon_hist_nominal::GenHistograms::new();
    let mut generated_sf = nano_gen_demo::muon_sf::GenHistograms::new();
    let mut interpreted_sf = InterpretedHistograms::new(&sf_plan);
    let mut reference_nominal = Hist1D::new(5, 0.0, 5.0);
    let mut reference_up = Hist1D::new(5, 0.0, 5.0);
    let mut reference_down = Hist1D::new(5, 0.0, 5.0);

    for entry in 0..5 {
        let event = synthetic_event(entry);
        let nominal_row = nano_gen_demo::muon_hist_nominal::GeneratedProducer::analyze_and_fill(
            &event,
            &mut generated_nominal,
            MuonHistNominalSystematic::Nominal,
        )
        .unwrap()
        .map(|row| row.n_good_muon);
        let sf_row = nano_gen_demo::muon_sf::GeneratedProducer::analyze_and_fill(
            &event,
            &mut generated_sf,
            SfSystematic::Nominal,
        )
        .unwrap()
        .map(|row| row.n_good_muon);
        let interpreted_row = interpret_and_fill(&sf_plan, &event, &mut interpreted_sf)
            .unwrap()
            .map(|row| match row.get("n_good_muon").unwrap() {
                Value::U32(value) => value,
                value => panic!("unexpected interpreted value {value:?}"),
            });
        let nominal_interpreted = interpret(&nominal_plan, &event).unwrap().map(|row| {
            match row.get("n_good_muon").unwrap() {
                Value::U32(value) => value,
                value => panic!("unexpected interpreted value {value:?}"),
            }
        });

        assert_eq!(nominal_row, sf_row, "entry {entry}");
        assert_eq!(sf_row, interpreted_row, "entry {entry}");
        assert_eq!(nominal_row, nominal_interpreted, "entry {entry}");

        if let Some(count) = sf_row {
            let value = f64::from(count);
            reference_nominal.fill_weighted(value, reference_sf_weight(&event, "nominal"));
            reference_up.fill_weighted(value, reference_sf_weight(&event, "systup"));
            reference_down.fill_weighted(value, reference_sf_weight(&event, "systdown"));
        }
    }

    let interpreted = interpreted_sf
        .get("n_good_muon_hist")
        .expect("interpreted histogram");
    let generated = &generated_sf.n_good_muon_hist;
    for systematic in [
        SfSystematic::Nominal,
        SfSystematic::MuonSfUp,
        SfSystematic::MuonSfDown,
    ] {
        let interpreted_key = format!("{systematic:?}");
        assert_eq!(
            generated.get(systematic),
            interpreted.get(interpreted_key),
            "{systematic:?}"
        );
    }

    assert_ne!(
        generated.get(SfSystematic::Nominal),
        &generated_nominal.n_good_muon_hist,
        "SF nominal should change the event weight"
    );
    assert_ne!(
        generated.get(SfSystematic::MuonSfUp),
        generated.get(SfSystematic::Nominal),
        "SF up variation should differ from nominal"
    );
    assert_ne!(
        generated.get(SfSystematic::MuonSfDown),
        generated.get(SfSystematic::Nominal),
        "SF down variation should differ from nominal"
    );
    assert_eq!(generated.get(SfSystematic::Nominal), &reference_nominal);
    assert_eq!(generated.get(SfSystematic::MuonSfUp), &reference_up);
    assert_eq!(generated.get(SfSystematic::MuonSfDown), &reference_down);
    assert_eq!(generated_nominal.n_good_muon_hist.sumw(), 3.0);
}

#[test]
fn workflow_executes_generated_muon_kernel_like_handwritten_kernel() {
    let fixture = Fixture::new("generated-workflow");
    let input = fixture.path("input.root");
    write_synthetic_input(
        &input,
        vec![
            vec![(31.0, 0.1), (10.0, 0.2)],
            vec![(29.9, 0.0)],
            vec![(45.0, 2.39), (35.0, -2.0)],
            vec![],
            vec![(60.0, 2.39)],
        ],
    );

    let schema = muon_schema();
    let handwritten_plan = plan_workflow_with_kernel_id(
        [&input],
        schema.clone(),
        2,
        fixture.path("cache-handwritten"),
        fixture.path("handwritten.root"),
        MuonProducer::analyze,
        "muon",
    )
    .unwrap();
    let generated_plan = plan_workflow_with_kernel_id(
        [&input],
        schema.clone(),
        2,
        fixture.path("cache-generated"),
        fixture.path("generated.root"),
        generated_muon_as_skim,
        "generated_muon",
    )
    .unwrap();

    let executor = Executor::new();
    let handwritten = executor
        .run(&handwritten_plan, ExecutionMode::Serial)
        .unwrap()
        .merged;
    let generated = executor
        .run(&generated_plan, ExecutionMode::Serial)
        .unwrap()
        .merged;
    assert_eq!(generated, handwritten);

    let mut registry = KernelRegistry::with_muon();
    registry.register("generated_muon", schema, generated_muon_as_skim);
    let partials = generated_plan
        .maps
        .iter()
        .map(|map| {
            run_chunk(
                &RunChunkRequest {
                    source: map.chunk.source.clone(),
                    entry_range: map.chunk.entry_range.clone(),
                    kernel_id: "generated_muon".to_string(),
                },
                &registry,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(merge_partials(partials), handwritten);
}

fn generated_muon_as_skim(event: &Event) -> nano_core::Result<Option<MuonSkimRow>> {
    GeneratedProducer::analyze(event).map(|row| {
        row.map(|row| MuonSkimRow {
            n_good_muon: row.n_good_muon,
            lead_muon_pt: row.lead_muon_pt,
        })
    })
}

fn synthetic_event(entry: usize) -> Event {
    Event::from_columns(schema(), columns(), entry).unwrap()
}

fn reference_sf_weight(event: &Event, systematic: &str) -> f64 {
    let payload = CorrectionSet::from_path("../nano-spec/tests/data/muon_sf.json").unwrap();
    let correction = payload.correction("synthetic_muon_sf").unwrap();
    let collection = event.collection("Muon").unwrap();
    let mut weight = 1.0_f64;
    for muon in collection.iter() {
        let pt = muon.get::<f32>("pt").unwrap();
        let eta = muon.get::<f32>("eta").unwrap();
        if pt > 30.0 && eta.abs() < 2.4 {
            weight *= correction
                .evaluate(&[
                    CorrectionValue::Real(f64::from(eta)),
                    CorrectionValue::Real(f64::from(pt)),
                    CorrectionValue::Str(systematic.to_string()),
                ])
                .unwrap();
        }
    }
    weight
}

fn schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    vec![
        ("nMuon".to_string(), BranchColumn::U32(vec![2, 1, 2, 0, 1])),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![31.0, 10.0],
                vec![29.9],
                vec![45.0, 35.0],
                vec![],
                vec![60.0],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.1, 0.2],
                vec![0.0],
                vec![2.39, -2.0],
                vec![],
                vec![2.39],
            ]),
        ),
    ]
}

fn write_synthetic_input(path: &Path, muons: Vec<Vec<(f32, f32)>>) {
    let n_muon = muons
        .iter()
        .map(|event_muons| event_muons.len() as u32)
        .collect::<Vec<_>>();
    let muon_pt = muons
        .iter()
        .map(|event_muons| event_muons.iter().map(|(pt, _)| *pt).collect())
        .collect::<Vec<Vec<_>>>();
    let muon_eta = muons
        .iter()
        .map(|event_muons| event_muons.iter().map(|(_, eta)| *eta).collect())
        .collect::<Vec<Vec<_>>>();

    write_events(
        path,
        &[
            OutputBranch::u32("nMuon", n_muon),
            OutputBranch::vec_f32("Muon_pt", muon_pt),
            OutputBranch::vec_f32("Muon_eta", muon_eta),
        ],
    )
    .unwrap();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CountRow {
    n_good_muon: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ZRow {
    z_mass: f64,
}

fn handwritten_all(event: &Event) -> Option<CountRow> {
    let collection = event.collection("Muon").unwrap();
    let mut count = 0_u32;
    let mut all_pass = true;
    for item in collection.iter() {
        count += 1;
        all_pass &= item.get::<f32>("pt").unwrap() > 5.0;
    }
    all_pass.then_some(CountRow { n_good_muon: count })
}

fn handwritten_charge_balance(event: &Event) -> Option<CountRow> {
    let collection = event.collection("Muon").unwrap();
    let mut count = 0_u32;
    let mut charge_sum = 0_i32;
    let mut positives = 0_u32;
    for item in collection.iter() {
        if item.get::<f32>("pt").unwrap() > 5.0 {
            count += 1;
            let charge = item.get::<i32>("charge").unwrap();
            charge_sum += charge;
            if charge > 0 {
                positives += 1;
            }
        }
    }
    (charge_sum == 0 && positives == 1).then_some(CountRow { n_good_muon: count })
}

fn handwritten_sip3d(event: &Event) -> Option<CountRow> {
    let collection = event.collection("Muon").unwrap();
    let mut count = 0_u32;
    for item in collection.iter() {
        let dxy = item.get::<f32>("dxy").unwrap();
        let dz = item.get::<f32>("dz").unwrap();
        let dxy_err = item.get::<f32>("dxyErr").unwrap();
        let dz_err = item.get::<f32>("dzErr").unwrap();
        let sip3d = ((f64::from(dxy).powf(2.0) + f64::from(dz).powf(2.0)).sqrt())
            / ((f64::from(dxy_err).powf(2.0) + f64::from(dz_err).powf(2.0)).sqrt());
        if sip3d < 4.0 && dxy.abs() < 0.5 && dz.abs() < 1.0 {
            count += 1;
        }
    }
    Some(CountRow { n_good_muon: count })
}

fn handwritten_pair_dr(event: &Event) -> Option<ZRow> {
    let collection = event.collection("Muon").unwrap();
    let mut selected = collection
        .iter()
        .filter_map(|item| {
            let pt = item.get::<f32>("pt").unwrap();
            (pt > 5.0).then(|| SelectedMuon {
                pt,
                eta: item.get::<f32>("eta").unwrap(),
                phi: item.get::<f32>("phi").unwrap(),
                mass: item.get::<f32>("mass").unwrap(),
                charge: item.get::<i32>("charge").unwrap(),
            })
        })
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| right.pt.total_cmp(&left.pt));

    for (left_pos, first) in selected.iter().enumerate() {
        for second in &selected[left_pos + 1..] {
            if first.charge * second.charge >= 0 {
                continue;
            }
            if delta_r(first.eta, first.phi, second.eta, second.phi) <= 0.3 {
                continue;
            }
            if first.pt.max(second.pt) <= 20.0 || first.pt.min(second.pt) <= 10.0 {
                continue;
            }
            let mass = invariant_mass([*first, *second]);
            if mass.is_finite() && mass > 0.0 {
                return Some(ZRow { z_mass: mass });
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct SelectedMuon {
    pt: f32,
    eta: f32,
    phi: f32,
    mass: f32,
    charge: i32,
}

fn invariant_mass(items: [SelectedMuon; 2]) -> f64 {
    let mut energy = 0.0;
    let mut px = 0.0;
    let mut py = 0.0;
    let mut pz = 0.0;
    for item in items {
        let pt = f64::from(item.pt);
        let eta = f64::from(item.eta);
        let phi = f64::from(item.phi);
        let mass = f64::from(item.mass);
        let item_px = pt * phi.cos();
        let item_py = pt * phi.sin();
        let item_pz = pt * eta.sinh();
        energy += (item_px * item_px + item_py * item_py + item_pz * item_pz + mass * mass).sqrt();
        px += item_px;
        py += item_py;
        pz += item_pz;
    }
    (energy * energy - px * px - py * py - pz * pz)
        .max(0.0)
        .sqrt()
}

fn delta_r(left_eta: f32, left_phi: f32, right_eta: f32, right_phi: f32) -> f64 {
    let deta = f64::from(left_eta - right_eta);
    let mut dphi = f64::from(left_phi - right_phi);
    while dphi > std::f64::consts::PI {
        dphi -= 2.0 * std::f64::consts::PI;
    }
    while dphi <= -std::f64::consts::PI {
        dphi += 2.0 * std::f64::consts::PI;
    }
    (deta * deta + dphi * dphi).sqrt()
}

fn selection_event(entry: usize) -> Event {
    Event::from_columns(selection_schema(), selection_columns(), entry).unwrap()
}

fn selection_columns_len() -> usize {
    5
}

fn selection_schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_charge", BranchType::VecI32),
        BranchSpec::new("Muon_dxy", BranchType::VecF32),
        BranchSpec::new("Muon_dxyErr", BranchType::VecF32),
        BranchSpec::new("Muon_dz", BranchType::VecF32),
        BranchSpec::new("Muon_dzErr", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_mass", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
    ])
    .unwrap()
}

fn selection_columns() -> Vec<(String, BranchColumn)> {
    vec![
        ("nMuon".to_string(), BranchColumn::U32(vec![2, 2, 2, 2, 0])),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![25.0, 15.0],
                vec![6.0, 4.0],
                vec![22.0, 12.0],
                vec![30.0, 25.0],
                vec![],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.0, 1.0],
                vec![0.2, -0.1],
                vec![0.1, -0.3],
                vec![0.0, 0.01],
                vec![],
            ]),
        ),
        (
            "Muon_phi".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.0, 1.0],
                vec![0.1, 0.2],
                vec![0.4, 2.9],
                vec![0.0, 0.01],
                vec![],
            ]),
        ),
        (
            "Muon_mass".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.105, 0.105],
                vec![0.105, 0.105],
                vec![0.105, 0.105],
                vec![0.105, 0.105],
                vec![],
            ]),
        ),
        (
            "Muon_charge".to_string(),
            BranchColumn::VecI32(vec![
                vec![1, -1],
                vec![1, -1],
                vec![1, 1],
                vec![1, -1],
                vec![],
            ]),
        ),
        (
            "Muon_dxy".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.01, 0.02],
                vec![0.2, 0.01],
                vec![0.6, 0.01],
                vec![0.01, 0.01],
                vec![],
            ]),
        ),
        (
            "Muon_dz".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.02, 0.03],
                vec![0.2, 0.01],
                vec![0.1, 1.2],
                vec![0.01, 0.01],
                vec![],
            ]),
        ),
        (
            "Muon_dxyErr".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.02, 0.02],
                vec![0.03, 0.02],
                vec![0.02, 0.02],
                vec![0.02, 0.02],
                vec![],
            ]),
        ),
        (
            "Muon_dzErr".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.02, 0.02],
                vec![0.03, 0.02],
                vec![0.02, 0.02],
                vec![0.02, 0.02],
                vec![],
            ]),
        ),
    ]
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nano-gen-demo-{}-{timestamp}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
