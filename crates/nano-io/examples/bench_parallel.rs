//! Parallel analysis-kernel benchmark over owned `Event` batches.
//!
//! Usage: bench_parallel <file.root> [n_events] [repeat]
use std::error::Error;
use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use nano_core::{BranchSchema, BranchSpec, BranchType, Event};
use nano_io::events;
use nano_producers::{MuonProducer, MuonSkimRow};
use rayon::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Contribution {
    input_muons: u64,
    input_checksum: f64,
    row: Option<MuonSkimRow>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Summary {
    events: u64,
    input_muons: u64,
    selected_events: u64,
    n_good_muon: u64,
    lead_muon_pt_milli: i64,
    lead_muon_pt_bits_checksum: u64,
}

impl Summary {
    fn add(mut self, contribution: Contribution) -> Self {
        self.events += 1;
        self.input_muons += contribution.input_muons;
        if let Some(row) = contribution.row {
            self.selected_events += 1;
            self.n_good_muon += u64::from(row.n_good_muon);
            self.lead_muon_pt_milli += (f64::from(row.lead_muon_pt) * 1000.0).round() as i64;
            self.lead_muon_pt_bits_checksum = self
                .lead_muon_pt_bits_checksum
                .wrapping_add(u64::from(row.lead_muon_pt.to_bits()));
        }
        self
    }

    fn merge(self, other: Self) -> Self {
        Self {
            events: self.events + other.events,
            input_muons: self.input_muons + other.input_muons,
            selected_events: self.selected_events + other.selected_events,
            n_good_muon: self.n_good_muon + other.n_good_muon,
            lead_muon_pt_milli: self.lead_muon_pt_milli + other.lead_muon_pt_milli,
            lead_muon_pt_bits_checksum: self
                .lead_muon_pt_bits_checksum
                .wrapping_add(other.lead_muon_pt_bits_checksum),
        }
    }

    fn lead_muon_pt_sum(&self) -> f64 {
        self.lead_muon_pt_milli as f64 / 1000.0
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        "tests/data/muon_validation/inputs/DoubleMuon_Run2016H_NANOAODv9.root".into()
    });
    let n_events: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let repeat: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(50);
    let repeat = repeat.max(1);
    let path = Path::new(&path);
    if !path.exists() {
        eprintln!("SKIP: {} absent", path.display());
        return Ok(());
    }

    let schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("MET_pt", BranchType::F32),
    ])?;

    let read_start = Instant::now();
    let batch = events(path, &schema)?
        .take(n_events)
        .collect::<nano_io::Result<Vec<_>>>()?;
    let read_time = read_start.elapsed();
    let n_read = batch.len();

    let serial_contributions = collect_serial(&batch);
    let parallel_contributions = collect_parallel(&batch);
    assert_eq!(
        serial_contributions, parallel_contributions,
        "parallel event outputs differ from serial outputs"
    );

    let ordered_input_checksum: f64 = serial_contributions
        .iter()
        .map(|contribution| contribution.input_checksum)
        .sum();
    let serial_summary = summarize_serial(&serial_contributions);
    let parallel_summary = summarize_parallel(&parallel_contributions);
    assert_eq!(
        serial_summary, parallel_summary,
        "parallel reduction differs from serial reduction"
    );

    if n_events == 100_000 && n_read == 100_000 {
        assert_eq!(serial_summary.input_muons, 214_677);
        assert_eq!(format!("{ordered_input_checksum:.1}"), "9593696.7");
    }

    black_box(summarize_serial(&serial_contributions));
    black_box(summarize_parallel(&parallel_contributions));

    let serial_time = time_repeated(repeat, || {
        black_box(summarize_serial(&collect_serial(&batch)));
    });
    let parallel_time = time_repeated(repeat, || {
        black_box(summarize_parallel(&collect_parallel(&batch)));
    });
    let speedup = serial_time.as_secs_f64() / parallel_time.as_secs_f64();
    let threads = rayon::current_num_threads();

    eprintln!(
        "nano.rust parallel demo read {n_read} events into owned Event batch: {:.3}s",
        read_time.as_secs_f64()
    );
    eprintln!(
        "correctness: serial == parallel outputs and reductions; input muons={}, checksum={ordered_input_checksum:.1}",
        serial_summary.input_muons
    );
    eprintln!(
        "muon producer aggregate: selected_events={}, n_good_muon={}, lead_muon_pt_sum={:.3}, lead_pt_bits_checksum={}",
        serial_summary.selected_events,
        serial_summary.n_good_muon,
        serial_summary.lead_muon_pt_sum(),
        serial_summary.lead_muon_pt_bits_checksum
    );
    eprintln!(
        "serial analysis+reduce x{repeat}: {:.3}s ({:.6}s/pass)",
        serial_time.as_secs_f64(),
        serial_time.as_secs_f64() / repeat as f64
    );
    eprintln!(
        "parallel analysis+reduce x{repeat} on {threads} rayon threads: {:.3}s ({:.6}s/pass), speedup={speedup:.2}x",
        parallel_time.as_secs_f64(),
        parallel_time.as_secs_f64() / repeat as f64
    );

    Ok(())
}

fn collect_serial(events: &[Event]) -> Vec<Contribution> {
    events.iter().map(analyze_event).collect()
}

fn collect_parallel(events: &[Event]) -> Vec<Contribution> {
    events.par_iter().map(analyze_event).collect()
}

fn analyze_event(event: &Event) -> Contribution {
    let input_muons = u64::from(event.scalar::<u32>("nMuon").expect("nMuon"));
    let muon_pt = event.vector_ref::<f32>("Muon_pt").expect("Muon_pt");
    let met_pt = event.scalar::<f32>("MET_pt").expect("MET_pt");
    let input_checksum = muon_pt.iter().map(|&pt| f64::from(pt)).sum::<f64>() + f64::from(met_pt);
    let row = MuonProducer::analyze(event).expect("muon producer");
    Contribution {
        input_muons,
        input_checksum,
        row,
    }
}

fn summarize_serial(contributions: &[Contribution]) -> Summary {
    contributions
        .iter()
        .copied()
        .fold(Summary::default(), Summary::add)
}

fn summarize_parallel(contributions: &[Contribution]) -> Summary {
    contributions
        .par_iter()
        .copied()
        .map(|contribution| Summary::default().add(contribution))
        .reduce(Summary::default, Summary::merge)
}

fn time_repeated(repeat: usize, mut f: impl FnMut()) -> Duration {
    let start = Instant::now();
    for _ in 0..repeat {
        f();
    }
    start.elapsed()
}
