use std::env;
use std::error::Error;
use std::path::PathBuf;

use nano_io::writer::{write_events, OutputBranch};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or("usage: write_synthetic <output.root> [n]")?,
    );
    let n = args
        .next()
        .as_deref()
        .unwrap_or("1000")
        .parse::<usize>()
        .map_err(|err| format!("invalid event count: {err}"))?;

    let n_muon = (0..n).map(|index| (index % 4) as u32).collect::<Vec<_>>();
    let muon_pt = n_muon
        .iter()
        .enumerate()
        .map(|(index, count)| {
            (0..*count)
                .map(|muon| 20.0 + (index % 100) as f32 + muon as f32)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let muon_eta = n_muon
        .iter()
        .map(|count| {
            (0..*count)
                .map(|muon| -2.0 + 0.1 * muon as f32)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    write_events(
        &path,
        &[
            OutputBranch::u32("nMuon", n_muon),
            OutputBranch::vec_f32("Muon_pt", muon_pt),
            OutputBranch::vec_f32("Muon_eta", muon_eta),
            OutputBranch::f32(
                "MET_pt",
                (0..n).map(|index| 40.0 + (index % 80) as f32).collect(),
            ),
            OutputBranch::i32("run", vec![315_252; n]),
            OutputBranch::u64(
                "event",
                (0..n).map(|index| 100_000 + index as u64).collect(),
            ),
            OutputBranch::bool(
                "pass_preselection",
                (0..n).map(|index| index % 2 == 0).collect(),
            ),
        ],
    )?;
    Ok(())
}
