use std::path::Path;

use nano_io::writer::{write_events, OutputBranch};

use crate::artifacts::MergedOutput;
use crate::error::Result;

pub fn write_muon_skim(path: &Path, output: &MergedOutput) -> Result<()> {
    write_events(
        path,
        &[
            OutputBranch::u32(
                "n_good_muon",
                output.rows.iter().map(|row| row.n_good_muon).collect(),
            ),
            OutputBranch::f32(
                "lead_muon_pt",
                output.rows.iter().map(|row| row.lead_muon_pt).collect(),
            ),
        ],
    )?;
    Ok(())
}
