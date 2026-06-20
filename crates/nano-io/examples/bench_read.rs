//! Local read benchmark: time nano.rust's streaming reader over N events,
//! reading the same branches as the ROOT/uproot comparison.
//! Usage: bench_read <file.root> [n_events]
use std::path::Path;
use std::time::Instant;

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::events;

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
    for ev in events(path, &schema).expect("open").take(n_events) {
        let ev = ev.expect("event");
        nmu += u64::from(ev.scalar::<u32>("nMuon").expect("nMuon"));
        for pt in ev.vector::<f32>("Muon_pt").expect("Muon_pt") {
            sum += f64::from(pt);
        }
        sum += f64::from(ev.scalar::<f32>("MET_pt").expect("MET_pt"));
        n += 1;
    }
    let dt = t0.elapsed().as_secs_f64();
    eprintln!("nano.rust events() local read {n} events: {dt:.3}s  (muons={nmu}, checksum={sum:.1})");
}
