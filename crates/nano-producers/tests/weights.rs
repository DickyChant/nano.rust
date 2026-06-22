use nano_analysis::{fill, Ev, Hist1D};
use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_producers::{
    select_muon_signal_region_with_varied_jets, select_muon_signal_region_with_weight,
    JetCorrectionInput, JetSystematic, JmeJetCorrections, MuonSkimRow, VariedJetSelection,
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
    let jet_count = jet_pts.len();
    Event::from_columns(
        BranchSchema::new([
            BranchSpec::new("Muon_pt", BranchType::VecF32),
            BranchSpec::new("Muon_eta", BranchType::VecF32),
            BranchSpec::new("Jet_pt", BranchType::VecF32),
            BranchSpec::new("Jet_eta", BranchType::VecF32),
            BranchSpec::new("Jet_phi", BranchType::VecF32),
            BranchSpec::new("Jet_mass", BranchType::VecF32),
        ])
        .unwrap(),
        [
            ("Muon_pt", BranchColumn::VecF32(vec![vec![45.0]])),
            ("Muon_eta", BranchColumn::VecF32(vec![vec![0.2]])),
            ("Jet_pt", BranchColumn::VecF32(vec![jet_pts])),
            ("Jet_eta", BranchColumn::VecF32(vec![jet_etas])),
            ("Jet_phi", BranchColumn::VecF32(vec![vec![0.1; jet_count]])),
            (
                "Jet_mass",
                BranchColumn::VecF32(vec![vec![20.0; jet_count]]),
            ),
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
fn real_jme_variation_changes_jet_four_vector_across_systematics() {
    let corrections = real_jme_corrections();
    let event = event_with_jets(vec![100.0], vec![0.5]);

    let nominal = corrections
        .varied_jets(&event, JetSystematic::Nominal)
        .unwrap();
    let jes_up = corrections
        .varied_jets(&event, JetSystematic::JesUp)
        .unwrap();
    let jes_down = corrections
        .varied_jets(&event, JetSystematic::JesDown)
        .unwrap();
    let jer_up = corrections
        .varied_jets(&event, JetSystematic::JerUp)
        .unwrap();
    let jer_down = corrections
        .varied_jets(&event, JetSystematic::JerDown)
        .unwrap();

    assert_close(nominal[0].pt, 100.0);
    assert_close(nominal[0].mass, 20.0);
    assert_close(jes_up[0].pt, 101.08);
    assert_close(jes_up[0].mass, 20.216);
    assert_close(jes_down[0].pt, 98.92);
    assert_close(jes_down[0].mass, 19.784);
    assert_close(jer_up[0].pt, 109.93);
    assert_close(jer_up[0].mass, 21.986);
    assert_close(jer_down[0].pt, 90.96697898662786);
    assert_close(jer_down[0].mass, 18.193_395_797_325_57);

    for varied in [jes_up[0].pt, jes_down[0].pt, jer_up[0].pt, jer_down[0].pt] {
        assert_ne!(nominal[0].pt, varied);
    }
}

#[test]
fn deterministic_expected_scales_for_fixed_real_payload_jet() {
    let corrections = real_jme_corrections();
    let jet = JetCorrectionInput {
        pt: 100.0,
        eta: 0.5,
    };

    assert_close(corrections.jes_total_uncertainty(jet).unwrap(), 0.0108);
    assert_close(corrections.jer_scale_factor(jet).unwrap(), 1.0993);
    assert_close(
        corrections.jet_scale(JetSystematic::JesUp, jet).unwrap(),
        1.0108,
    );
    assert_close(
        corrections.jet_scale(JetSystematic::JerDown, jet).unwrap(),
        0.9096697898662786,
    );
}

#[test]
fn jme_weight_api_is_only_normalization_not_shape_bookkeeping() {
    let corrections = real_jme_corrections();
    let event = event_with_jets(vec![100.0], vec![0.5]);

    for systematic in JetSystematic::all() {
        assert_close(
            corrections
                .event_weight(&event, systematic)
                .unwrap()
                .value(),
            1.0,
        );
    }
}

#[test]
fn varied_selection_recomputes_jet_threshold_under_shape_variation() {
    let corrections = real_jme_corrections();
    let event = event_with_jets(vec![29.75], vec![0.5]);
    let jet_selection = VariedJetSelection::new(30.0, 2.4);

    let nominal = select_muon_signal_region_with_varied_jets(
        Ev::new(&event),
        &corrections,
        JetSystematic::Nominal,
        jet_selection,
    )
    .unwrap()
    .unwrap();
    let jes_up = select_muon_signal_region_with_varied_jets(
        Ev::new(&event),
        &corrections,
        JetSystematic::JesUp,
        jet_selection,
    )
    .unwrap()
    .unwrap();
    let jes_down = select_muon_signal_region_with_varied_jets(
        Ev::new(&event),
        &corrections,
        JetSystematic::JesDown,
        jet_selection,
    )
    .unwrap()
    .unwrap();

    assert_eq!(nominal.n_selected_jets(), 0);
    assert_eq!(jes_up.n_selected_jets(), 1);
    assert_eq!(jes_down.n_selected_jets(), 0);
    assert_close(jes_up.lead_selected_jet_pt().unwrap(), 30.359875);
    assert_close(jes_down.jets()[0].pt, 29.140125);
    assert_close(jes_up.normalization_weight().value(), 1.0);
}

#[test]
fn selected_event_normalization_weight_fills_histograms_for_all_systematics() {
    let corrections = real_jme_corrections();
    let event = event_with_jets(vec![100.0], vec![0.5]);

    for systematic in JetSystematic::all() {
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
