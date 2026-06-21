use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nano_spec::codegen::generate_producer_source;
use nano_spec::{validate, AnalysisSpec, Catalogue};

#[path = "fuzz_specs.rs"]
mod fuzz_specs;

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

    generate_fuzz_modules(&catalogue, &out_dir)?;

    Ok(())
}

fn generate_fuzz_modules(catalogue: &Catalogue, out_dir: &Path) -> Result<(), Box<dyn Error>> {
    let specs = fuzz_specs::generated_specs();
    let mut modules = String::new();
    writeln!(modules, "#[derive(Debug, Clone, Copy, PartialEq)]")?;
    writeln!(modules, "pub struct FuzzRow {{")?;
    writeln!(modules, "    pub n_muon: u32,")?;
    writeln!(modules, "    pub n_electron: u32,")?;
    writeln!(modules, "    pub n_jet: u32,")?;
    writeln!(modules, "    pub lead_muon_pt: f64,")?;
    writeln!(modules, "    pub lead_electron_pt: f64,")?;
    writeln!(modules, "    pub lead_jet_pt: f64,")?;
    writeln!(modules, "    pub sum_muon_pt: f64,")?;
    writeln!(modules, "    pub sum_electron_pt: f64,")?;
    writeln!(modules, "    pub sum_jet_pt: f64,")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    writeln!(modules, "#[derive(Debug, Clone, PartialEq)]")?;
    writeln!(modules, "pub struct FuzzHist1D {{")?;
    writeln!(modules, "    pub bins: Vec<f64>,")?;
    writeln!(modules, "    pub underflow: f64,")?;
    writeln!(modules, "    pub overflow: f64,")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    writeln!(modules, "#[derive(Debug, Clone, PartialEq)]")?;
    writeln!(modules, "pub struct FuzzHistVariation {{")?;
    writeln!(modules, "    pub systematic: &'static str,")?;
    writeln!(modules, "    pub hist: FuzzHist1D,")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    writeln!(modules, "#[derive(Debug, Clone, PartialEq)]")?;
    writeln!(modules, "pub struct FuzzCaseResult {{")?;
    writeln!(modules, "    pub rows: Vec<Option<FuzzRow>>,")?;
    writeln!(
        modules,
        "    pub histogram: Option<Vec<FuzzHistVariation>>,"
    )?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    writeln!(
        modules,
        "fn snapshot_hist(hist: &nano_analysis::Hist1D) -> FuzzHist1D {{"
    )?;
    writeln!(modules, "    FuzzHist1D {{")?;
    writeln!(modules, "        bins: hist.bins().to_vec(),")?;
    writeln!(modules, "        underflow: hist.underflow(),")?;
    writeln!(modules, "        overflow: hist.overflow(),")?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;
    let mut arms = String::new();

    for generated in &specs {
        let module_name = format!("case_{:03}", generated.index);
        let file_name = format!("generated_fuzz_{:03}.rs", generated.index);
        let plan = validate(&generated.spec, catalogue).map_err(|errors| {
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
        fs::write(out_dir.join(&file_name), source)?;

        writeln!(
            arms,
            "        {} => run_{}(events),",
            generated.index, module_name
        )?;
        writeln!(modules)?;
        writeln!(modules, "#[allow(dead_code, non_snake_case, unused_parens, clippy::collapsible_if, clippy::double_parens, clippy::manual_range_contains, clippy::neg_cmp_op_on_partial_ord, clippy::unnecessary_cast)]")?;
        writeln!(modules, "pub mod {module_name} {{")?;
        writeln!(
            modules,
            "    include!(concat!(env!(\"OUT_DIR\"), \"/{file_name}\"));"
        )?;
        writeln!(modules, "}}")?;
        writeln!(modules)?;
        emit_run_case(
            &mut modules,
            &module_name,
            generated.has_histogram,
            generated.has_weight_systematic,
        )?;
    }

    writeln!(
        modules,
        "pub fn run_case(case: usize, events: &[nano_core::Event]) -> nano_core::Result<FuzzCaseResult> {{"
    )?;
    writeln!(modules, "    match case {{")?;
    modules.push_str(&arms);
    writeln!(
        modules,
        "        _ => Err(nano_core::NanoError::MissingBranch {{ branch: format!(\"unknown fuzz case {{case}}\") }}),"
    )?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;

    fs::write(out_dir.join("generated_fuzz_modules.rs"), modules)?;
    Ok(())
}

fn emit_run_case(
    modules: &mut String,
    module_name: &str,
    has_histogram: bool,
    has_weight_systematic: bool,
) -> Result<(), Box<dyn Error>> {
    writeln!(
        modules,
        "fn run_{module_name}(events: &[nano_core::Event]) -> nano_core::Result<FuzzCaseResult> {{"
    )?;
    writeln!(
        modules,
        "    let mut rows = Vec::with_capacity(events.len());"
    )?;
    if has_histogram {
        writeln!(
            modules,
            "    let mut histograms = {module_name}::GenHistograms::new();"
        )?;
        writeln!(modules, "    for event in events {{")?;
        writeln!(
            modules,
            "        rows.push({module_name}::GeneratedProducer::analyze_and_fill(event, &mut histograms, nano_analysis::Systematic::Nominal)?.map(normalize_{module_name}));"
        )?;
        writeln!(modules, "    }}")?;
        if has_weight_systematic {
            writeln!(modules, "    let histogram = Some(vec![")?;
            for (name, variant) in [
                ("Nominal", "Nominal"),
                ("JesUp", "JesUp"),
                ("JesDown", "JesDown"),
            ] {
                writeln!(
                    modules,
                    "        FuzzHistVariation {{ systematic: \"{name}\", hist: snapshot_hist(histograms.lead_muon_pt_hist.get(nano_analysis::Systematic::{variant})) }},"
                )?;
            }
            writeln!(modules, "    ]);")?;
        } else {
            writeln!(modules, "    let histogram = Some(vec![")?;
            writeln!(
                modules,
                "        FuzzHistVariation {{ systematic: \"Nominal\", hist: snapshot_hist(&histograms.lead_muon_pt_hist) }},"
            )?;
            writeln!(modules, "    ]);")?;
        }
    } else {
        writeln!(modules, "    for event in events {{")?;
        writeln!(
            modules,
            "        rows.push({module_name}::GeneratedProducer::analyze(event)?.map(normalize_{module_name}));"
        )?;
        writeln!(modules, "    }}")?;
        writeln!(modules, "    let histogram = None;")?;
    }
    writeln!(modules, "    Ok(FuzzCaseResult {{ rows, histogram }})")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    writeln!(
        modules,
        "fn normalize_{module_name}(row: {module_name}::GenRow) -> FuzzRow {{"
    )?;
    writeln!(modules, "    FuzzRow {{")?;
    writeln!(modules, "        n_muon: row.n_muon,")?;
    writeln!(modules, "        n_electron: row.n_electron,")?;
    writeln!(modules, "        n_jet: row.n_jet,")?;
    writeln!(
        modules,
        "        lead_muon_pt: f64::from(row.lead_muon_pt),"
    )?;
    writeln!(
        modules,
        "        lead_electron_pt: f64::from(row.lead_electron_pt),"
    )?;
    writeln!(modules, "        lead_jet_pt: f64::from(row.lead_jet_pt),")?;
    writeln!(modules, "        sum_muon_pt: row.sum_muon_pt,")?;
    writeln!(modules, "        sum_electron_pt: row.sum_electron_pt,")?;
    writeln!(modules, "        sum_jet_pt: row.sum_jet_pt,")?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    Ok(())
}
