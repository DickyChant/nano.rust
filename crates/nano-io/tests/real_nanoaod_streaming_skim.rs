use std::path::{Path, PathBuf};

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::events;
use nano_io::writer::{write_events, OutputBranch};
use nano_producers::MuonProducer;

#[test]
fn streams_first_100k_real_nanoaod_events_to_muon_skim() {
    let input =
        Path::new("../../tests/data/muon_validation/inputs/DoubleMuon_Run2016H_NANOAODv9.root");
    if !input.exists() {
        eprintln!("SKIP: {} absent (gitignored test input)", input.display());
        return;
    }

    let input_schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
    ])
    .unwrap();

    let mut streamed = 0_usize;
    let mut selected_rows = Vec::new();
    for event in events(input, &input_schema).unwrap().take(100_000) {
        streamed += 1;
        if let Some(row) = MuonProducer::analyze(&event.unwrap()).unwrap() {
            selected_rows.push(row);
        }
    }
    assert_eq!(streamed, 100_000);
    assert!(
        !selected_rows.is_empty(),
        "expected real DoubleMuon input to pass the muon skim"
    );

    let skim_path = temp_root_path("real_nanoaod_muon_skim.root");
    let _ = std::fs::remove_file(&skim_path);
    write_events(
        &skim_path,
        &[
            OutputBranch::u32(
                "n_good_muon",
                selected_rows.iter().map(|row| row.n_good_muon).collect(),
            ),
            OutputBranch::f32(
                "lead_muon_pt",
                selected_rows.iter().map(|row| row.lead_muon_pt).collect(),
            ),
        ],
    )
    .unwrap();

    let skim_schema = BranchSchema::new([
        BranchSpec::new("n_good_muon", BranchType::U32),
        BranchSpec::new("lead_muon_pt", BranchType::F32),
    ])
    .unwrap();

    let mut skim_entries = 0_usize;
    for event in events(&skim_path, &skim_schema).unwrap() {
        let event = event.unwrap();
        let n_good_muon = event.scalar::<u32>("n_good_muon").unwrap();
        let lead_muon_pt = event.scalar::<f32>("lead_muon_pt").unwrap();
        assert!(n_good_muon >= 1);
        assert!(lead_muon_pt.is_finite() && lead_muon_pt > 0.0);
        skim_entries += 1;
    }

    assert_eq!(skim_entries, selected_rows.len());
    eprintln!(
        "real NanoAOD streaming skim: {}/{} events passed",
        selected_rows.len(),
        streamed
    );

    let _ = std::fs::remove_file(&skim_path);
}

fn temp_root_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("nano-io-{}-{name}", std::process::id()));
    path
}
