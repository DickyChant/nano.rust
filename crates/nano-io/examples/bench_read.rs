//! Local read benchmark: time nano.rust's streaming reader over N events,
//! reading the same branches as the ROOT/uproot comparison.
//! Usage: bench_read <file.root> [n_events]
use std::path::Path;
use std::time::Instant;

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::events;
use nano_rootio::{BasketPayloadCache, RootFile};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        "tests/data/muon_validation/inputs/DoubleMuon_Run2016H_NANOAODv9.root".into()
    });
    let n_events: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let path = Path::new(&path);
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return;
    }
    let schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("MET_pt", BranchType::F32),
    ])
    .expect("schema");

    let t0 = Instant::now();
    let (mut n, mut nmu, mut sum) = (0u64, 0u64, 0f64);
    let mut event_iter = events(path, &schema).expect("open").take(n_events);
    if let Some(ev) = event_iter.next() {
        let ev = ev.expect("event");
        let nmuon = ev.branch_handle("nMuon").expect("nMuon handle");
        let muon_pt = ev.branch_handle("Muon_pt").expect("Muon_pt handle");
        let met_pt = ev.branch_handle("MET_pt").expect("MET_pt handle");

        nmu += u64::from(ev.scalar_with::<u32>(&nmuon).expect("nMuon"));
        for &pt in ev.vector_ref_with::<f32>(&muon_pt).expect("Muon_pt") {
            sum += f64::from(pt);
        }
        sum += f64::from(ev.scalar_with::<f32>(&met_pt).expect("MET_pt"));
        n += 1;

        for ev in event_iter {
            let ev = ev.expect("event");
            nmu += u64::from(ev.scalar_with::<u32>(&nmuon).expect("nMuon"));
            for &pt in ev.vector_ref_with::<f32>(&muon_pt).expect("Muon_pt") {
                sum += f64::from(pt);
            }
            sum += f64::from(ev.scalar_with::<f32>(&met_pt).expect("MET_pt"));
            n += 1;
        }
    }
    let dt = t0.elapsed().as_secs_f64();
    eprintln!(
        "nano.rust events() local read {n} events: {dt:.3}s  (muons={nmu}, checksum={sum:.1})"
    );

    let t0 = Instant::now();
    let file = RootFile::open(path).expect("open ROOT file");
    let tree = file.tree("Events").expect("Events tree");
    let n = n_events.min(usize::try_from(tree.entries()).expect("entries"));
    let mut cache = BasketPayloadCache::new();
    let nmuons = tree
        .read_scalar_range_cached::<u32>("nMuon", 0, n, &mut cache)
        .expect("nMuon");
    let muon_pt = tree
        .read_jagged_flat_range_cached::<f32>("Muon_pt", "nMuon", 0, n, &mut cache)
        .expect("Muon_pt");
    let _muon_eta = tree
        .read_jagged_flat_range_cached::<f32>("Muon_eta", "nMuon", 0, n, &mut cache)
        .expect("Muon_eta");
    let met_pt = tree
        .read_scalar_range_cached::<f32>("MET_pt", 0, n, &mut cache)
        .expect("MET_pt");
    let mut nmu = 0u64;
    let mut sum = 0f64;
    for (index, (&count, &met)) in nmuons.iter().zip(&met_pt).enumerate() {
        nmu += u64::from(count);
        let start = muon_pt.offsets[index];
        let end = muon_pt.offsets[index + 1];
        let pts = &muon_pt.values[start..end];
        for &pt in pts {
            sum += f64::from(pt);
        }
        sum += f64::from(met);
    }
    let dt = t0.elapsed().as_secs_f64();
    eprintln!(
        "nano.rootio raw columns local read {n} events: {dt:.3}s  (muons={nmu}, checksum={sum:.1})"
    );
}
