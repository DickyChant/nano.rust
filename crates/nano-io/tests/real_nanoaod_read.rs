// Real-data smoke test against a CMS Open Data NanoAODv9 file (DoubleMuon
// Run2016H). It exercises the actual ROOT TTree version handling and lazy,
// BOUNDED local streaming via nano-io's nano-rootio reader path. Skips
// gracefully if the file is absent (it is gitignored / not in CI).
use std::path::Path;

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::events_chunked;

#[test]
fn reads_first_entries_of_real_nanoaod() {
    let path =
        Path::new("../../tests/data/muon_validation/inputs/DoubleMuon_Run2016H_NANOAODv9.root");
    if !path.exists() {
        eprintln!("SKIP: {} absent (gitignored test input)", path.display());
        return;
    }

    let schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
    ])
    .unwrap();

    let rows = events_chunked(path, &schema, 5)
        .expect("open local NanoAOD")
        .take(5)
        .map(|event| {
            let event = event.expect("read event");
            let n_muon = event.scalar::<u32>("nMuon").expect("nMuon");
            let muon_pt = event.vector::<f32>("Muon_pt").expect("Muon_pt");
            (n_muon, muon_pt)
        })
        .collect::<Vec<_>>();

    let n_muon = rows.iter().map(|(count, _)| *count).collect::<Vec<_>>();
    assert_eq!(n_muon.len(), 5, "expected at least 5 entries");
    assert!(
        n_muon.iter().all(|&n| n < 100),
        "implausible nMuon: {n_muon:?}"
    );

    let muon_pt = rows.iter().map(|(_, pts)| pts.clone()).collect::<Vec<_>>();
    assert_eq!(muon_pt.len(), n_muon.len());
    for (count, pts) in n_muon.iter().zip(&muon_pt) {
        assert_eq!(*count as usize, pts.len(), "jagged length mismatch");
        for pt in pts {
            assert!(pt.is_finite() && *pt > 0.0, "bad muon pt {pt}");
        }
    }
    eprintln!("real NanoAODv9: first 5 nMuon={n_muon:?}, Muon_pt={muon_pt:?}");
}
