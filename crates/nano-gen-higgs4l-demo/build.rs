use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nano_spec::codegen::generate_producer_source;
use nano_spec::{validate, AnalysisSpec, Catalogue};

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let repo_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not find repo root"))?;

    let catalogue_path = repo_root.join("configs/branches/nanov9.yaml");
    println!("cargo:rerun-if-changed={}", catalogue_path.display());
    let catalogue_text = fs::read_to_string(&catalogue_path)?;
    let catalogue = Catalogue::from_nanoaod_yaml_str(&catalogue_text, "v9")?;

    generate(
        repo_root,
        &catalogue,
        "crates/nano-spec/examples/higgs4mu_minimal.toml",
        "generated_higgs4mu.rs",
    )?;
    generate(
        repo_root,
        &catalogue,
        "crates/nano-spec/examples/higgs2e2mu_minimal.toml",
        "generated_higgs2e2mu.rs",
    )?;

    Ok(())
}

fn generate(
    repo_root: &Path,
    catalogue: &Catalogue,
    spec_relative: &str,
    output_name: &str,
) -> Result<(), Box<dyn Error>> {
    let spec_path = repo_root.join(spec_relative);
    println!("cargo:rerun-if-changed={}", spec_path.display());
    let spec = AnalysisSpec::from_path(&spec_path)?;
    let plan = validate(&spec, catalogue).map_err(|errors| {
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
    fs::write(out_dir.join(output_name), source)?;
    Ok(())
}
