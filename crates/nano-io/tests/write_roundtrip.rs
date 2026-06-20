use std::path::{Path, PathBuf};

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::events;
use nano_io::writer::{write_events, OutputBranch};

#[test]
fn writes_10_events_and_reads_them_back() {
    let path = temp_root_path("ten_event_roundtrip.root");
    let _ = std::fs::remove_file(&path);

    write_10_events(&path);

    let schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("MET_pt", BranchType::F32),
        BranchSpec::new("run", BranchType::I32),
        BranchSpec::new("event", BranchType::U64),
        BranchSpec::new("pass_preselection", BranchType::Bool),
    ])
    .unwrap();

    let rows = events(&path, &schema)
        .unwrap()
        .map(|event| {
            let event = event.unwrap();
            (
                event.scalar::<u32>("nMuon").unwrap(),
                event.vector::<f32>("Muon_pt").unwrap(),
                event.vector::<f32>("Muon_eta").unwrap(),
                event.scalar::<f32>("MET_pt").unwrap(),
                event.scalar::<i32>("run").unwrap(),
                event.scalar::<u64>("event").unwrap(),
                event.scalar::<bool>("pass_preselection").unwrap(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(rows.len(), 10);
    for (index, (n_muon, pt, eta, met, run, event, pass)) in rows.iter().enumerate() {
        assert_eq!(*n_muon as usize, pt.len());
        assert_eq!(*n_muon as usize, eta.len());
        assert_eq!(*run, 315_252);
        assert_eq!(*event, 10_000 + index as u64);
        assert_eq!(*pass, index % 2 == 0);
        assert_eq!(*met, 50.0 + index as f32);
    }

    let _ = std::fs::remove_file(&path);
}

fn write_10_events(path: &Path) {
    let n_muon = (0..10).map(|index| (index % 3) as u32).collect::<Vec<_>>();
    let muon_pt = n_muon
        .iter()
        .enumerate()
        .map(|(index, count)| {
            (0..*count)
                .map(|muon| 20.0 + index as f32 + muon as f32)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let muon_eta = n_muon
        .iter()
        .map(|count| {
            (0..*count)
                .map(|muon| -0.2 + 0.1 * muon as f32)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    write_events(
        path,
        &[
            OutputBranch::u32("nMuon", n_muon),
            OutputBranch::vec_f32("Muon_pt", muon_pt),
            OutputBranch::vec_f32("Muon_eta", muon_eta),
            OutputBranch::f32("MET_pt", (0..10).map(|index| 50.0 + index as f32).collect()),
            OutputBranch::i32("run", vec![315_252; 10]),
            OutputBranch::u64("event", (0..10).map(|index| 10_000 + index).collect()),
            OutputBranch::bool(
                "pass_preselection",
                (0..10).map(|index| index % 2 == 0).collect(),
            ),
        ],
    )
    .unwrap();
}

fn temp_root_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("nano-io-{}-{name}", std::process::id()));
    path
}
