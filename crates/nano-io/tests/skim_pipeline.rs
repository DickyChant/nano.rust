use std::path::{Path, PathBuf};

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::reader::read_events;
use nano_io::writer::{write_events, OutputBranch};
use nano_producers::MuonProducer;

// This validates Phase-1 plumbing with synthetic data. It is not physics
// validation against the C++ references or correction payloads.
#[tokio::test]
async fn synthetic_muon_skim_round_trips_through_root_io() {
    let input_path = temp_root_path("synthetic_muon_input.root");
    let skim_path = temp_root_path("synthetic_muon_skim.root");
    let _ = std::fs::remove_file(&input_path);
    let _ = std::fs::remove_file(&skim_path);

    write_synthetic_input(&input_path);

    let input_schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
        BranchSpec::new("Muon_mass", BranchType::VecF32),
        BranchSpec::new("MET_pt", BranchType::F32),
        BranchSpec::new("pass_preselection", BranchType::Bool),
        BranchSpec::new("event", BranchType::U64),
        BranchSpec::new("run", BranchType::I32),
    ])
    .unwrap();

    let events = read_events(&input_path, input_schema).await.unwrap();
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].scalar::<bool>("pass_preselection").unwrap(), true);
    assert_eq!(events[4].scalar::<u64>("event").unwrap(), 1005);
    assert_eq!(events[2].scalar::<i32>("run").unwrap(), 315252);

    let selected_rows = events
        .iter()
        .filter_map(|event| MuonProducer::analyze(event).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(selected_rows.len(), 3);
    assert_eq!(
        selected_rows
            .iter()
            .map(|row| row.n_good_muon)
            .collect::<Vec<_>>(),
        vec![1, 2, 1]
    );
    assert_eq!(
        selected_rows
            .iter()
            .map(|row| row.lead_muon_pt)
            .collect::<Vec<_>>(),
        vec![31.0, 45.0, 60.0]
    );

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
    let skim_events = read_events(&skim_path, skim_schema).await.unwrap();

    assert_eq!(skim_events.len(), 3);
    assert_eq!(
        skim_events
            .iter()
            .map(|event| event.scalar::<u32>("n_good_muon").unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 1]
    );
    assert_eq!(
        skim_events
            .iter()
            .map(|event| event.scalar::<f32>("lead_muon_pt").unwrap())
            .collect::<Vec<_>>(),
        vec![31.0, 45.0, 60.0]
    );

    let _ = std::fs::remove_file(&input_path);
    let _ = std::fs::remove_file(&skim_path);
}

fn write_synthetic_input(path: &Path) {
    write_events(
        path,
        &[
            OutputBranch::u32("nMuon", vec![2, 1, 2, 0, 1]),
            OutputBranch::vec_f32(
                "Muon_pt",
                vec![
                    vec![31.0, 10.0],
                    vec![29.9],
                    vec![45.0, 35.0],
                    vec![],
                    vec![60.0],
                ],
            ),
            OutputBranch::vec_f32(
                "Muon_eta",
                vec![
                    vec![0.1, 0.2],
                    vec![0.0],
                    vec![2.39, -2.0],
                    vec![],
                    vec![2.39],
                ],
            ),
            OutputBranch::vec_f32(
                "Muon_phi",
                vec![
                    vec![0.0, 1.0],
                    vec![2.0],
                    vec![1.5, -1.5],
                    vec![],
                    vec![0.7],
                ],
            ),
            OutputBranch::vec_f32(
                "Muon_mass",
                vec![
                    vec![0.105, 0.105],
                    vec![0.105],
                    vec![0.105, 0.105],
                    vec![],
                    vec![0.105],
                ],
            ),
            OutputBranch::f32("MET_pt", vec![80.0, 40.0, 120.0, 10.0, 90.0]),
            OutputBranch::bool("pass_preselection", vec![true, false, true, true, true]),
            OutputBranch::u64("event", vec![1001, 1002, 1003, 1004, 1005]),
            OutputBranch::i32("run", vec![315252, 315252, 315252, 315252, 315252]),
        ],
    )
    .unwrap();
}

fn temp_root_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("nano-io-{}-{name}", std::process::id()));
    path
}
