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

    let spec_path = repo_root.join("crates/nano-spec/examples/muon_tagger.toml");
    let catalogue_path = repo_root.join("configs/branches/nanov9.yaml");
    println!("cargo:rerun-if-changed={}", spec_path.display());
    println!("cargo:rerun-if-changed={}", catalogue_path.display());

    let catalogue_text = fs::read_to_string(&catalogue_path)?;
    let spec = AnalysisSpec::from_path(&spec_path)?;
    let catalogue = Catalogue::from_nanoaod_yaml_str(&catalogue_text, "v9")?;
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
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(out_dir.join("generated_muon_tagger.rs"), source)?;

    Ok(())
}
