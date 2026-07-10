use nano_core::{BranchColumn, BranchType, Event};
use nano_producers::{wz_schema, WzConfig, WzProducer};

fn signal_like_event(jet_btags: Vec<f32>, tau: Option<TauFixture>) -> Event {
    let jet_count = jet_btags.len();
    let mut jet_pts = vec![120.0, 100.0];
    let mut jet_etas = vec![3.0, -3.0];
    let mut jet_phis = vec![0.5, -0.5];
    let mut jet_masses = vec![20.0, 20.0];
    if jet_count > 2 {
        jet_pts.push(35.0);
        jet_etas.push(0.5);
        jet_phis.push(2.5);
        jet_masses.push(10.0);
    }

    Event::from_columns(
        wz_schema(),
        [
            ("event", BranchColumn::U64(vec![42])),
            ("PuppiMET_pt", BranchColumn::F32(vec![40.0])),
            ("PuppiMET_phi", BranchColumn::F32(vec![1.2])),
            ("Muon_pt", BranchColumn::VecF32(vec![vec![50.0, 45.0]])),
            ("Muon_eta", BranchColumn::VecF32(vec![vec![0.0, 0.0]])),
            (
                "Muon_phi",
                BranchColumn::VecF32(vec![vec![0.0, std::f32::consts::PI]]),
            ),
            ("Muon_mass", BranchColumn::VecF32(vec![vec![0.105, 0.105]])),
            ("Muon_charge", BranchColumn::VecI32(vec![vec![1, -1]])),
            ("Muon_dxy", BranchColumn::VecF32(vec![vec![0.01, 0.01]])),
            ("Muon_dz", BranchColumn::VecF32(vec![vec![0.02, 0.02]])),
            (
                "Muon_looseId",
                BranchColumn::VecBool(vec![vec![true, true]]),
            ),
            (
                "Muon_mediumPromptId",
                BranchColumn::VecBool(vec![vec![true, true]]),
            ),
            ("Muon_pfIsoId", BranchColumn::VecU8(vec![vec![2, 2]])),
            ("Muon_jetRelIso", BranchColumn::VecF32(vec![vec![0.1, 0.1]])),
            ("Muon_jetIdx", BranchColumn::VecI16(vec![vec![-1, -1]])),
            ("Electron_pt", BranchColumn::VecF32(vec![vec![35.0]])),
            ("Electron_eta", BranchColumn::VecF32(vec![vec![0.3]])),
            ("Electron_phi", BranchColumn::VecF32(vec![vec![0.7]])),
            ("Electron_mass", BranchColumn::VecF32(vec![vec![0.0005]])),
            ("Electron_charge", BranchColumn::VecI32(vec![vec![1]])),
            ("Electron_dxy", BranchColumn::VecF32(vec![vec![0.01]])),
            ("Electron_dz", BranchColumn::VecF32(vec![vec![0.02]])),
            ("Electron_cutBased", BranchColumn::VecU8(vec![vec![3]])),
            ("Electron_jetRelIso", BranchColumn::VecF32(vec![vec![0.1]])),
            ("Electron_jetIdx", BranchColumn::VecI16(vec![vec![-1]])),
            ("Jet_pt", BranchColumn::VecF32(vec![jet_pts])),
            ("Jet_eta", BranchColumn::VecF32(vec![jet_etas])),
            ("Jet_phi", BranchColumn::VecF32(vec![jet_phis])),
            ("Jet_mass", BranchColumn::VecF32(vec![jet_masses])),
            ("Jet_jetId", BranchColumn::VecU8(vec![vec![2; jet_count]])),
            (
                "Jet_btagRobustParTAK4B",
                BranchColumn::VecF32(vec![jet_btags]),
            ),
            (
                "Tau_pt",
                BranchColumn::VecF32(vec![tau.iter().map(|t| t.pt).collect()]),
            ),
            (
                "Tau_eta",
                BranchColumn::VecF32(vec![tau.iter().map(|t| t.eta).collect()]),
            ),
            (
                "Tau_idDeepTau2018v2p5VSjet",
                BranchColumn::VecU8(vec![tau.iter().map(|t| t.vs_jet).collect()]),
            ),
            (
                "Tau_idDeepTau2018v2p5VSe",
                BranchColumn::VecU8(vec![tau.iter().map(|t| t.vs_e).collect()]),
            ),
            (
                "Tau_idDeepTau2018v2p5VSmu",
                BranchColumn::VecU8(vec![tau.iter().map(|t| t.vs_mu).collect()]),
            ),
        ],
        0,
    )
    .unwrap()
}

#[derive(Debug, Clone, Copy)]
struct TauFixture {
    pt: f32,
    eta: f32,
    vs_jet: u8,
    vs_e: u8,
    vs_mu: u8,
}

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() < tolerance,
        "actual {actual} != expected {expected}"
    );
}

#[test]
fn nominal_wz_vbs_signal_event_produces_validation_row() {
    let event = signal_like_event(vec![0.1, 0.2], None);
    let row = WzProducer::analyze(&event)
        .unwrap()
        .expect("selected WZ row");

    assert_eq!(row.event_num, 42);
    assert_eq!(row.n_fake_mu, 2);
    assert_eq!(row.n_fake_el, 1);
    assert_eq!(row.nbtag_goodbtag_jet_bjet, 0);
    assert_eq!(row.ngood_jets, 2);
    assert_eq!(row.flavorl1_z, 0);
    assert_eq!(row.flavorl2_z, 0);
    assert_eq!(row.flavorl_w, 1);
    assert_eq!(row.tri_lepton_flavor, 1);
    assert!(row.mll_z < 15.0);
    assert!(row.m3l > 100.0);
    assert!(row.vbs_mjj > 500.0);
    assert!(row.vbs_detajj > 2.5);
    assert!(row.vbs_zepvv < 1.0);
    assert_close(row.ptl_w, 35.0, 1.0e-6);
}

#[test]
fn btagged_event_is_rejected_from_signal_and_kept_in_control_region() {
    let event = signal_like_event(vec![0.1, 0.2, 0.95], None);

    assert!(WzProducer::analyze(&event).unwrap().is_none());
    let row = WzProducer::analyze_with_config(&event, WzConfig::btagged_control())
        .unwrap()
        .expect("b-tagged control row");

    assert_eq!(row.nbtag_goodbtag_jet_bjet, 1);
    assert!(row.vbs_mjj > 500.0);
}

#[test]
fn default_run2024_btag_working_point_matches_legacy_wz_selection() {
    let event = signal_like_event(vec![0.1, 0.2, 0.6], None);

    assert!(WzProducer::analyze(&event).unwrap().is_none());
    let row = WzProducer::analyze_with_config(&event, WzConfig::btagged_control())
        .unwrap()
        .expect("legacy Run2024 b-tag working point should classify the event");

    assert_eq!(row.nbtag_goodbtag_jet_bjet, 1);
}

#[test]
fn event_without_same_flavor_opposite_sign_z_candidate_is_rejected() {
    let event = Event::from_columns(
        wz_schema(),
        [
            ("event", BranchColumn::U64(vec![7])),
            ("PuppiMET_pt", BranchColumn::F32(vec![40.0])),
            ("PuppiMET_phi", BranchColumn::F32(vec![1.2])),
            ("Muon_pt", BranchColumn::VecF32(vec![vec![50.0, 45.0]])),
            ("Muon_eta", BranchColumn::VecF32(vec![vec![0.0, 0.0]])),
            (
                "Muon_phi",
                BranchColumn::VecF32(vec![vec![0.0, std::f32::consts::PI]]),
            ),
            ("Muon_mass", BranchColumn::VecF32(vec![vec![0.105, 0.105]])),
            ("Muon_charge", BranchColumn::VecI32(vec![vec![1, 1]])),
            ("Muon_dxy", BranchColumn::VecF32(vec![vec![0.01, 0.01]])),
            ("Muon_dz", BranchColumn::VecF32(vec![vec![0.02, 0.02]])),
            (
                "Muon_looseId",
                BranchColumn::VecBool(vec![vec![true, true]]),
            ),
            (
                "Muon_mediumPromptId",
                BranchColumn::VecBool(vec![vec![true, true]]),
            ),
            ("Muon_pfIsoId", BranchColumn::VecU8(vec![vec![2, 2]])),
            ("Muon_jetRelIso", BranchColumn::VecF32(vec![vec![0.1, 0.1]])),
            ("Muon_jetIdx", BranchColumn::VecI16(vec![vec![-1, -1]])),
            ("Electron_pt", BranchColumn::VecF32(vec![vec![35.0]])),
            ("Electron_eta", BranchColumn::VecF32(vec![vec![0.3]])),
            ("Electron_phi", BranchColumn::VecF32(vec![vec![0.7]])),
            ("Electron_mass", BranchColumn::VecF32(vec![vec![0.0005]])),
            ("Electron_charge", BranchColumn::VecI32(vec![vec![-1]])),
            ("Electron_dxy", BranchColumn::VecF32(vec![vec![0.01]])),
            ("Electron_dz", BranchColumn::VecF32(vec![vec![0.02]])),
            ("Electron_cutBased", BranchColumn::VecU8(vec![vec![3]])),
            ("Electron_jetRelIso", BranchColumn::VecF32(vec![vec![0.1]])),
            ("Electron_jetIdx", BranchColumn::VecI16(vec![vec![-1]])),
            ("Jet_pt", BranchColumn::VecF32(vec![vec![120.0, 100.0]])),
            ("Jet_eta", BranchColumn::VecF32(vec![vec![3.0, -3.0]])),
            ("Jet_phi", BranchColumn::VecF32(vec![vec![0.5, -0.5]])),
            ("Jet_mass", BranchColumn::VecF32(vec![vec![20.0, 20.0]])),
            ("Jet_jetId", BranchColumn::VecU8(vec![vec![2, 2]])),
            (
                "Jet_btagRobustParTAK4B",
                BranchColumn::VecF32(vec![vec![0.1, 0.2]]),
            ),
            ("Tau_pt", BranchColumn::VecF32(vec![vec![]])),
            ("Tau_eta", BranchColumn::VecF32(vec![vec![]])),
            (
                "Tau_idDeepTau2018v2p5VSjet",
                BranchColumn::VecU8(vec![vec![]]),
            ),
            (
                "Tau_idDeepTau2018v2p5VSe",
                BranchColumn::VecU8(vec![vec![]]),
            ),
            (
                "Tau_idDeepTau2018v2p5VSmu",
                BranchColumn::VecU8(vec![vec![]]),
            ),
        ],
        0,
    )
    .unwrap();

    assert!(WzProducer::analyze(&event).unwrap().is_none());
}

#[test]
fn selected_hadronic_tau_vetoes_otherwise_passing_event() {
    let tau = TauFixture {
        pt: 25.0,
        eta: 1.1,
        vs_jet: 6,
        vs_e: 6,
        vs_mu: 4,
    };
    let event = signal_like_event(vec![0.1, 0.2], Some(tau));

    assert!(WzProducer::analyze(&event).unwrap().is_none());
}

#[test]
fn wz_schema_declares_nanoaod_v12_jet_index_contract() {
    let schema = wz_schema();

    assert_eq!(
        schema.find("Muon_jetIdx").map(|info| info.branch_type),
        Some(BranchType::VecI16)
    );
    assert_eq!(
        schema.find("Electron_jetIdx").map(|info| info.branch_type),
        Some(BranchType::VecI16)
    );
    assert!(schema
        .find("Jet_btagRobustParTAK4B")
        .is_some_and(|info| info.optional));
    assert!(schema
        .find("Jet_btagUParTAK4B")
        .is_some_and(|info| info.optional));
}
