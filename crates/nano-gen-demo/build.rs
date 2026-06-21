use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::PathBuf;

use nano_spec::codegen::generate_producer_source;
use nano_spec::{validate, AnalysisSpec, Catalogue};

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let repo_root = manifest_dir
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not find repo root"))?;

    let catalogue_path = repo_root.join("configs/branches/nanov9.yaml");
    println!("cargo:rerun-if-changed={}", catalogue_path.display());

    let catalogue_text = fs::read_to_string(&catalogue_path)?;
    let catalogue = Catalogue::from_nanoaod_yaml_str(&catalogue_text, "v9")?;
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    for (spec_file, generated_file) in [
        ("muon.toml", "generated_muon.rs"),
        ("selection_all.toml", "generated_selection_all.rs"),
        (
            "selection_charge_balance.toml",
            "generated_selection_charge_balance.rs",
        ),
        ("selection_sip3d.toml", "generated_selection_sip3d.rs"),
        ("selection_pair_dr.toml", "generated_selection_pair_dr.rs"),
        ("muon_hist_nominal.toml", "generated_muon_hist_nominal.rs"),
        (
            "muon_hist_weight_systematic.toml",
            "generated_muon_hist_weight_systematic.rs",
        ),
    ] {
        let spec_path = repo_root.join("crates/nano-spec/examples").join(spec_file);
        println!("cargo:rerun-if-changed={}", spec_path.display());
        let spec = AnalysisSpec::from_path(&spec_path)?;
        let plan = validate(&spec, &catalogue).map_err(|errors| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        })?;
        let source = generate_producer_source(&plan)?;
        fs::write(out_dir.join(generated_file), source)?;
    }

    Ok(())
}
