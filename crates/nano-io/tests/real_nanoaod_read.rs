// Real-data smoke test against a CMS Open Data NanoAODv9 file (DoubleMuon
// Run2016H). It exercises the actual ROOT TTree version handling and lazy,
// BOUNDED basket reads (only the first few entries are pulled, so memory stays
// tiny regardless of the ~2 GB file). Skips gracefully if the file is absent
// (it is gitignored / not in CI).
//
// NOTE: this is bounded on purpose. nano-io's eager `read_events` materialises
// every entry and must NOT be pointed at a full NanoAOD file until the lazy /
// streaming reader lands (see docs/rust-migration.md).
use std::path::Path;

use futures::executor::block_on;
use futures::StreamExt;
use nom::number::complete::{be_f32, be_u32};
use root_io::RootFile;

#[test]
fn reads_first_entries_of_real_nanoaod() {
    let path =
        Path::new("../../tests/data/muon_validation/inputs/DoubleMuon_Run2016H_NANOAODv9.root");
    if !path.exists() {
        eprintln!("SKIP: {} absent (gitignored test input)", path.display());
        return;
    }
    block_on(async {
        let file = RootFile::new(path).await.expect("open file");
        let item = file
            .items()
            .iter()
            .find(|it| it.name().contains("`Events`") && it.verbose_info().contains("TTree"))
            .expect("Events tree");
        // Parsing the real NanoAODv9 Events tree exercises the TTree/TBranch
        // version handling.
        let tree = item.as_tree().await.expect("parse Events tree");

        // Lazy, bounded read: only the first 5 entries -> first basket only.
        let n_muon: Vec<u32> = tree
            .branch_by_name("nMuon")
            .expect("nMuon branch")
            .as_fixed_size_iterator(|i| be_u32(i))
            .take(5)
            .collect()
            .await;
        assert_eq!(n_muon.len(), 5, "expected at least 5 entries");
        assert!(
            n_muon.iter().all(|&n| n < 100),
            "implausible nMuon: {n_muon:?}"
        );

        // Jagged read for those same entries, using nMuon as the per-entry count.
        let muon_pt: Vec<Vec<f32>> = tree
            .branch_by_name("Muon_pt")
            .expect("Muon_pt branch")
            .as_var_size_iterator(|i| be_f32(i), n_muon.clone())
            .collect()
            .await;
        assert_eq!(muon_pt.len(), n_muon.len());
        for (count, pts) in n_muon.iter().zip(&muon_pt) {
            assert_eq!(*count as usize, pts.len(), "jagged length mismatch");
            for pt in pts {
                assert!(pt.is_finite() && *pt > 0.0, "bad muon pt {pt}");
            }
        }
        eprintln!("real NanoAODv9: first 5 nMuon={n_muon:?}, Muon_pt={muon_pt:?}");
    });
}
