use nano_analysis::{fill, Ev, Hist1D, Systematic};
use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_producers::{
    select_muon_signal_region_with_weight, JetCorrectionInput, JmeJetCorrections, MuonSkimRow,
    WeightedMuonSkimRow,
};
use std::path::Path;

fn real_jme_corrections() -> JmeJetCorrections {
    JmeJetCorrections::from_path(Path::new(
        "../../data/jme-derived/Run2-2016postVFP-UL-NanoAODv9/latest/jet_jerc.json.gz",
    ))
    .unwrap()
}

fn event_with_jets(jet_pts: Vec<f32>, jet_etas: Vec<f32>) -> Event {
    Event::from_columns(
        BranchSchema::new([
            BranchSpec::new("Muon_pt", BranchType::VecF32),
            BranchSpec::new("Muon_eta", BranchType::VecF32),
            BranchSpec::new("Jet_pt", BranchType::VecF32),
            BranchSpec::new("Jet_eta", BranchType::VecF32),
        ])
        .unwrap(),
        [
            ("Muon_pt", BranchColumn::VecF32(vec![vec![45.0]])),
            ("Muon_eta", BranchColumn::VecF32(vec![vec![0.2]])),
            ("Jet_pt", BranchColumn::VecF32(vec![jet_pts])),
            ("Jet_eta", BranchColumn::VecF32(vec![jet_etas])),
        ],
        0,
    )
    .unwrap()
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1.0e-12,
        "actual {actual} != expected {expected}"
    );
}

#[test]
fn real_jme_weight_differs_across_all_shape_systematics() {
    let corrections = real_jme_corrections();
    let event = event_with_jets(vec![100.0], vec![0.5]);

    let nominal = corrections
        .event_weight(&event, Systematic::Nominal)
        .unwrap()
        .value();
    let jes_up = corrections
        .event_weight(&event, Systematic::JesUp)
        .unwrap()
        .value();
    let jes_down = corrections
        .event_weight(&event, Systematic::JesDown)
        .unwrap()
        .value();
    let jer_up = corrections
        .event_weight(&event, Systematic::JerUp)
        .unwrap()
        .value();
    let jer_down = corrections
        .event_weight(&event, Systematic::JerDown)
        .unwrap()
        .value();

    assert_close(nominal, 1.0);
    assert_close(jes_up, 1.0108);
    assert_close(jes_down, 0.9892);
    assert_close(jer_up, 1.0993);
    assert_close(jer_down, 0.9096697898662786);

    for varied in [jes_up, jes_down, jer_up, jer_down] {
        assert_ne!(nominal, varied);
    }
}

#[test]
fn deterministic_expected_weight_for_fixed_real_payload_jet() {
    let corrections = real_jme_corrections();
    let jet = JetCorrectionInput {
        pt: 100.0,
        eta: 0.5,
    };

    assert_close(corrections.jes_total_uncertainty(jet).unwrap(), 0.0108);
    assert_close(corrections.jer_scale_factor(jet).unwrap(), 1.0993);
    assert_close(
        corrections.jet_factor(Systematic::JesUp, jet).unwrap(),
        1.0108,
    );
    assert_close(
        corrections.jet_factor(Systematic::JerDown, jet).unwrap(),
        0.9096697898662786,
    );
}

#[test]
fn selected_event_weights_and_fills_histograms_for_nominal_and_variations() {
    let corrections = real_jme_corrections();
    let event = event_with_jets(vec![100.0], vec![0.5]);

    for systematic in Systematic::all() {
        let weighted =
            select_muon_signal_region_with_weight(Ev::new(&event), &corrections, systematic)
                .unwrap()
                .unwrap();
        let mut hist = Hist1D::new(4, 0.0, 100.0);

        fill(&mut hist, &weighted, 45.0);

        assert_eq!(weighted.region_name(), "signal");
        assert_close(hist.sumw(), weighted.weight().value());
        assert_close(hist.bins()[1], weighted.weight().value());
    }
}

#[test]
fn weighted_muon_skim_row_exposes_event_weight_additively() {
    let row = MuonSkimRow {
        n_good_muon: 1,
        lead_muon_pt: 45.0,
    };
    let weighted = WeightedMuonSkimRow::from_row(row, nano_analysis::EventWeight::nominal());

    assert_eq!(
        weighted,
        WeightedMuonSkimRow {
            n_good_muon: 1,
            lead_muon_pt: 45.0,
            event_weight: 1.0,
        }
    );
}
