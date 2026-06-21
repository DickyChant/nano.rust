#![cfg(feature = "jit")]

use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_demo::GeneratedProducer;
use nano_jit::{JitBuildProfile, JitMuonRunner};
use nano_producers::MuonProducer;
use nano_spec::{validate, AnalysisSpec, Catalogue};

const MUON_SPEC: &str = include_str!("../../nano-spec/examples/muon.toml");
const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");

#[test]
fn jit_muon_kernel_matches_aot_codegen_and_handwritten_producer() {
    if std::env::var("NANO_RUN_JIT").as_deref() != Ok("1") {
        eprintln!("skipping runtime JIT test; set NANO_RUN_JIT=1 to compile and dlopen a kernel");
        return;
    }

    let spec = AnalysisSpec::from_toml_str(MUON_SPEC).unwrap();
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let plan = validate(&spec, &catalogue).unwrap();
    std::env::set_var("NANO_JIT_CARGO_OFFLINE", "1");
    let jit = JitMuonRunner::compile_with_profile(&plan, JitBuildProfile::Debug).unwrap();

    for entry in 0..5 {
        let event = synthetic_event(entry);
        let jit = jit
            .analyze(&event)
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt));
        let aot = GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt));
        let handwritten = MuonProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt));

        assert_eq!(jit, aot, "JIT != AOT at entry {entry}");
        assert_eq!(jit, handwritten, "JIT != handwritten at entry {entry}");
    }
}

fn synthetic_event(entry: usize) -> Event {
    Event::from_columns(schema(), columns(), entry).unwrap()
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
