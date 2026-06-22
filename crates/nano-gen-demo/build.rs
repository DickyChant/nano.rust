use std::env;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nano_spec::codegen::generate_producer_source;
use nano_spec::{validate, AnalysisSpec, Catalogue, Expr, SystematicDef};

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
        (
            "muon_hist_shape_nominal.toml",
            "generated_muon_hist_shape_nominal.rs",
        ),
        (
            "muon_hist_shape_correction.toml",
            "generated_muon_hist_shape_correction.rs",
        ),
        ("muon_sf.toml", "generated_muon_sf.rs"),
        ("lumi_mask_trigger.toml", "generated_lumi_mask_trigger.rs"),
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
    let union_specs = fuzz_specs::generated_union_specs();
    let model_histogram_specs = fuzz_specs::generated_model_histogram_specs();
    let model_weight_systematic_specs = fuzz_specs::generated_model_weight_systematic_specs();
    let model_shape_specs = fuzz_specs::generated_model_shape_specs();
    let derived_model_specs = fuzz_specs::generated_derived_under_model_specs();
    let weight_shape_specs = fuzz_specs::generated_weight_shape_specs();
    let mut modules = String::new();
    let derived_cases = specs.iter().filter(|case| case.has_derived_object).count();
    let candidate_cases = specs
        .iter()
        .filter(|case| case.has_candidate_object)
        .count();
    let model_cases = specs.iter().filter(|case| case.has_model).count();
    let model_histogram_cases = model_histogram_specs.len();
    let model_weight_systematic_cases = model_weight_systematic_specs.len();
    let model_shape_cases = model_shape_specs.len();
    let derived_model_cases = derived_model_specs.len();
    let weight_shape_cases = weight_shape_specs.len();
    writeln!(
        modules,
        "pub const FUZZ_DERIVED_CASES: usize = {derived_cases};"
    )?;
    writeln!(
        modules,
        "pub const FUZZ_CANDIDATE_CASES: usize = {candidate_cases};"
    )?;
    writeln!(
        modules,
        "pub const FUZZ_MODEL_CASES: usize = {model_cases};"
    )?;
    writeln!(
        modules,
        "pub const FUZZ_UNION_CASES: usize = {};",
        union_specs.len()
    )?;
    writeln!(
        modules,
        "pub const FUZZ_MODEL_HISTOGRAM_CASES: usize = {model_histogram_cases};"
    )?;
    writeln!(
        modules,
        "pub const FUZZ_MODEL_WEIGHT_SYSTEMATIC_CASES: usize = {model_weight_systematic_cases};"
    )?;
    writeln!(
        modules,
        "pub const FUZZ_MODEL_SHAPE_CASES: usize = {model_shape_cases};"
    )?;
    writeln!(
        modules,
        "pub const FUZZ_DERIVED_MODEL_CASES: usize = {derived_model_cases};"
    )?;
    writeln!(
        modules,
        "pub const FUZZ_WEIGHT_SHAPE_CASES: usize = {weight_shape_cases};"
    )?;
    writeln!(modules)?;
    writeln!(modules, "#[derive(Debug, Clone, Copy, PartialEq)]")?;
    writeln!(modules, "pub enum FuzzValue {{")?;
    writeln!(modules, "    F64(f64),")?;
    writeln!(modules, "    U32(u32),")?;
    writeln!(modules, "    Bool(bool),")?;
    writeln!(modules, "    I64(i64),")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    writeln!(modules, "#[derive(Debug, Clone, PartialEq)]")?;
    writeln!(modules, "pub struct FuzzRow {{")?;
    writeln!(modules, "    pub values: Vec<(String, FuzzValue)>,")?;
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
    writeln!(modules, "    pub systematic: String,")?;
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
    writeln!(modules, "#[derive(Debug, Clone, PartialEq)]")?;
    writeln!(modules, "pub struct FuzzUnionRow {{")?;
    writeln!(modules, "    pub channel: String,")?;
    writeln!(modules, "    pub values: Vec<(String, FuzzValue)>,")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    writeln!(modules, "#[derive(Debug, Clone, PartialEq)]")?;
    writeln!(modules, "pub struct FuzzUnionCaseResult {{")?;
    writeln!(modules, "    pub rows: Vec<Vec<FuzzUnionRow>>,")?;
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
        writeln!(modules, "#[allow(dead_code, non_snake_case, unused_parens, clippy::approx_constant, clippy::collapsible_if, clippy::double_parens, clippy::manual_range_contains, clippy::neg_cmp_op_on_partial_ord, clippy::unnecessary_cast)]")?;
        writeln!(modules, "pub mod {module_name} {{")?;
        writeln!(
            modules,
            "    include!(concat!(env!(\"OUT_DIR\"), \"/{file_name}\"));"
        )?;
        writeln!(modules, "}}")?;
        writeln!(modules)?;
        emit_run_case(&mut modules, &module_name, generated)?;
    }

    let mut union_arms = String::new();
    for generated in &union_specs {
        let module_name = format!("union_case_{:03}", generated.index);
        let file_name = format!("generated_fuzz_union_{:03}.rs", generated.index);
        emit_generated_module(
            catalogue,
            out_dir,
            generated,
            &module_name,
            &file_name,
            &mut modules,
        )?;
        writeln!(
            union_arms,
            "        {} => run_{}(events),",
            generated.index, module_name
        )?;
        emit_run_union_case(&mut modules, &module_name, generated)?;
    }

    let mut model_histogram_arms = String::new();
    for generated in &model_histogram_specs {
        let module_name = format!("model_hist_case_{:03}", generated.index);
        let file_name = format!("generated_fuzz_model_hist_{:03}.rs", generated.index);
        emit_generated_module(
            catalogue,
            out_dir,
            generated,
            &module_name,
            &file_name,
            &mut modules,
        )?;
        writeln!(
            model_histogram_arms,
            "        {} => run_{}(events),",
            generated.index, module_name
        )?;
        emit_run_case(&mut modules, &module_name, generated)?;
    }

    let mut model_weight_systematic_arms = String::new();
    for generated in &model_weight_systematic_specs {
        let module_name = format!("model_weight_case_{:03}", generated.index);
        let file_name = format!("generated_fuzz_model_weight_{:03}.rs", generated.index);
        emit_generated_module(
            catalogue,
            out_dir,
            generated,
            &module_name,
            &file_name,
            &mut modules,
        )?;
        writeln!(
            model_weight_systematic_arms,
            "        {} => run_{}(events),",
            generated.index, module_name
        )?;
        emit_run_case(&mut modules, &module_name, generated)?;
    }

    let mut model_shape_arms = String::new();
    for generated in &model_shape_specs {
        let module_name = format!("model_shape_case_{:03}", generated.index);
        let file_name = format!("generated_fuzz_model_shape_{:03}.rs", generated.index);
        emit_generated_module(
            catalogue,
            out_dir,
            generated,
            &module_name,
            &file_name,
            &mut modules,
        )?;
        writeln!(
            model_shape_arms,
            "        {} => run_{}(events),",
            generated.index, module_name
        )?;
        emit_run_case(&mut modules, &module_name, generated)?;
    }

    let mut derived_model_arms = String::new();
    for generated in &derived_model_specs {
        let module_name = format!("derived_model_case_{:03}", generated.index);
        let file_name = format!("generated_fuzz_derived_model_{:03}.rs", generated.index);
        emit_generated_module(
            catalogue,
            out_dir,
            generated,
            &module_name,
            &file_name,
            &mut modules,
        )?;
        writeln!(
            derived_model_arms,
            "        {} => run_{}(events),",
            generated.index, module_name
        )?;
        emit_run_case(&mut modules, &module_name, generated)?;
    }

    let mut weight_shape_arms = String::new();
    for generated in &weight_shape_specs {
        let module_name = format!("weight_shape_case_{:03}", generated.index);
        let file_name = format!("generated_fuzz_weight_shape_{:03}.rs", generated.index);
        emit_generated_module(
            catalogue,
            out_dir,
            generated,
            &module_name,
            &file_name,
            &mut modules,
        )?;
        writeln!(
            weight_shape_arms,
            "        {} => run_{}(events),",
            generated.index, module_name
        )?;
        emit_run_case(&mut modules, &module_name, generated)?;
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

    writeln!(
        modules,
        "pub fn run_union_case(case: usize, events: &[nano_core::Event]) -> nano_core::Result<FuzzUnionCaseResult> {{"
    )?;
    writeln!(modules, "    match case {{")?;
    modules.push_str(&union_arms);
    writeln!(
        modules,
        "        _ => Err(nano_core::NanoError::MissingBranch {{ branch: format!(\"unknown union fuzz case {{case}}\") }}),"
    )?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;

    writeln!(
        modules,
        "pub fn run_model_histogram_case(case: usize, events: &[nano_core::Event]) -> nano_core::Result<FuzzCaseResult> {{"
    )?;
    writeln!(modules, "    match case {{")?;
    modules.push_str(&model_histogram_arms);
    writeln!(
        modules,
        "        _ => Err(nano_core::NanoError::MissingBranch {{ branch: format!(\"unknown model histogram fuzz case {{case}}\") }}),"
    )?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;

    writeln!(
        modules,
        "pub fn run_model_weight_systematic_case(case: usize, events: &[nano_core::Event]) -> nano_core::Result<FuzzCaseResult> {{"
    )?;
    writeln!(modules, "    match case {{")?;
    modules.push_str(&model_weight_systematic_arms);
    writeln!(
        modules,
        "        _ => Err(nano_core::NanoError::MissingBranch {{ branch: format!(\"unknown model weight-systematic fuzz case {{case}}\") }}),"
    )?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;

    writeln!(
        modules,
        "pub fn run_weight_shape_case(case: usize, events: &[nano_core::Event]) -> nano_core::Result<FuzzCaseResult> {{"
    )?;
    writeln!(modules, "    match case {{")?;
    modules.push_str(&weight_shape_arms);
    writeln!(
        modules,
        "        _ => Err(nano_core::NanoError::MissingBranch {{ branch: format!(\"unknown weight-shape fuzz case {{case}}\") }}),"
    )?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;

    writeln!(
        modules,
        "pub fn run_model_shape_case(case: usize, events: &[nano_core::Event]) -> nano_core::Result<FuzzCaseResult> {{"
    )?;
    writeln!(modules, "    match case {{")?;
    modules.push_str(&model_shape_arms);
    writeln!(
        modules,
        "        _ => Err(nano_core::NanoError::MissingBranch {{ branch: format!(\"unknown model-shape fuzz case {{case}}\") }}),"
    )?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;

    writeln!(
        modules,
        "pub fn run_derived_model_case(case: usize, events: &[nano_core::Event]) -> nano_core::Result<FuzzCaseResult> {{"
    )?;
    writeln!(modules, "    match case {{")?;
    modules.push_str(&derived_model_arms);
    writeln!(
        modules,
        "        _ => Err(nano_core::NanoError::MissingBranch {{ branch: format!(\"unknown derived-model fuzz case {{case}}\") }}),"
    )?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;

    fs::write(out_dir.join("generated_fuzz_modules.rs"), modules)?;
    Ok(())
}

fn emit_generated_module(
    catalogue: &Catalogue,
    out_dir: &Path,
    generated: &fuzz_specs::GeneratedSpec,
    module_name: &str,
    file_name: &str,
    modules: &mut String,
) -> Result<(), Box<dyn Error>> {
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
    fs::write(out_dir.join(file_name), source)?;
    writeln!(modules)?;
    writeln!(modules, "#[allow(dead_code, non_snake_case, unused_parens, clippy::approx_constant, clippy::collapsible_if, clippy::double_parens, clippy::manual_range_contains, clippy::neg_cmp_op_on_partial_ord, clippy::unnecessary_cast)]")?;
    writeln!(modules, "pub mod {module_name} {{")?;
    writeln!(
        modules,
        "    include!(concat!(env!(\"OUT_DIR\"), \"/{file_name}\"));"
    )?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    Ok(())
}

fn emit_run_case(
    modules: &mut String,
    module_name: &str,
    generated: &fuzz_specs::GeneratedSpec,
) -> Result<(), Box<dyn Error>> {
    let has_histogram = generated.has_histogram;
    let has_histogram_systematic =
        generated.has_weight_systematic || generated.has_shape_correction;
    let histogram_name = generated
        .spec
        .histograms
        .first()
        .map(|histogram| histogram.name.as_str());
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
        if generated.has_model {
            let model_name = generated
                .spec
                .models
                .first()
                .map(|model| model.name.as_str())
                .expect("model flag implies model spec");
            writeln!(
                modules,
                "    let predictor = nano_inference::MockPredictor::new({model_name:?});"
            )?;
        }
        if generated.has_shape_correction {
            writeln!(modules, "    for event in events {{")?;
            writeln!(
                modules,
                "        rows.push({analyze_call});",
                analyze_call = if generated.has_model {
                    format!("{module_name}::GeneratedProducer::analyze(event, &predictor).map_err(|error| nano_core::NanoError::MissingAttachment {{ name: error.to_string() }})?.map(normalize_{module_name})")
                } else {
                    format!("{module_name}::GeneratedProducer::analyze(event)?.map(normalize_{module_name})")
                }
            )?;
            for (_, variant) in systematic_variants(&generated.spec) {
                writeln!(
                    modules,
                    "        let _ = {fill_call};",
                    fill_call = if generated.has_model {
                        format!("{module_name}::GeneratedProducer::analyze_and_fill(event, &mut histograms, {module_name}::Systematic::{variant}, &predictor).map_err(|error| nano_core::NanoError::MissingAttachment {{ name: error.to_string() }})?")
                    } else {
                        format!("{module_name}::GeneratedProducer::analyze_and_fill(event, &mut histograms, {module_name}::Systematic::{variant})?")
                    }
                )?;
            }
            writeln!(modules, "    }}")?;
        } else {
            writeln!(modules, "    for event in events {{")?;
            writeln!(
                modules,
                "        rows.push({fill_call}.map(normalize_{module_name}));",
                fill_call = if generated.has_model {
                    format!("{module_name}::GeneratedProducer::analyze_and_fill(event, &mut histograms, {module_name}::Systematic::Nominal, &predictor).map_err(|error| nano_core::NanoError::MissingAttachment {{ name: error.to_string() }})?")
                } else {
                    format!("{module_name}::GeneratedProducer::analyze_and_fill(event, &mut histograms, {module_name}::Systematic::Nominal)?")
                }
            )?;
            writeln!(modules, "    }}")?;
        }
        let histogram_name = histogram_name.expect("histogram exists when flag is set");
        if has_histogram_systematic {
            writeln!(modules, "    let histogram = Some(vec![")?;
            for (name, variant) in systematic_variants(&generated.spec) {
                writeln!(
                    modules,
                    "        FuzzHistVariation {{ systematic: \"{name}\".to_string(), hist: snapshot_hist(histograms.{histogram_name}.get({module_name}::Systematic::{variant})) }},"
                )?;
            }
            writeln!(modules, "    ]);")?;
        } else {
            writeln!(modules, "    let histogram = Some(vec![")?;
            writeln!(
                modules,
                "        FuzzHistVariation {{ systematic: \"Nominal\".to_string(), hist: snapshot_hist(&histograms.{histogram_name}) }},"
            )?;
            writeln!(modules, "    ]);")?;
        }
    } else {
        writeln!(modules, "    for event in events {{")?;
        if generated.has_model {
            let model_name = generated
                .spec
                .models
                .first()
                .map(|model| model.name.as_str())
                .expect("model flag implies model spec");
            writeln!(
                modules,
                "        let predictor = nano_inference::MockPredictor::new({model_name:?});"
            )?;
            writeln!(
                modules,
                "        rows.push({module_name}::GeneratedProducer::analyze(event, &predictor).map_err(|error| nano_core::NanoError::MissingAttachment {{ name: error.to_string() }})?.map(normalize_{module_name}));"
            )?;
        } else {
            writeln!(
                modules,
                "        rows.push({module_name}::GeneratedProducer::analyze(event)?.map(normalize_{module_name}));"
            )?;
        }
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
    writeln!(modules, "        values: vec![")?;
    for output in &generated.spec.outputs {
        writeln!(
            modules,
            "            ({}.to_string(), {}),",
            format_args!("{:?}", output.name),
            fuzz_value_expr(&output.expr, &output.name)
        )?;
    }
    writeln!(modules, "        ],")?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    Ok(())
}

fn emit_run_union_case(
    modules: &mut String,
    module_name: &str,
    generated: &fuzz_specs::GeneratedSpec,
) -> Result<(), Box<dyn Error>> {
    let histogram_name = generated
        .spec
        .histograms
        .first()
        .map(|histogram| histogram.name.as_str());
    writeln!(
        modules,
        "fn run_{module_name}(events: &[nano_core::Event]) -> nano_core::Result<FuzzUnionCaseResult> {{"
    )?;
    writeln!(
        modules,
        "    let mut rows = Vec::with_capacity(events.len());"
    )?;
    if generated.has_histogram {
        writeln!(
            modules,
            "    let mut histograms = {module_name}::GenHistograms::new();"
        )?;
        writeln!(modules, "    for event in events {{")?;
        writeln!(
            modules,
            "        rows.push({module_name}::GeneratedProducer::analyze_and_fill(event, &mut histograms, {module_name}::Systematic::Nominal)?.into_iter().map(normalize_{module_name}).collect());"
        )?;
        writeln!(modules, "    }}")?;
        let histogram_name = histogram_name.expect("histogram exists when flag is set");
        writeln!(modules, "    let histogram = Some(vec![")?;
        writeln!(
            modules,
            "        FuzzHistVariation {{ systematic: \"Nominal\".to_string(), hist: snapshot_hist(&histograms.{histogram_name}) }},"
        )?;
        writeln!(modules, "    ]);")?;
    } else {
        writeln!(modules, "    for event in events {{")?;
        writeln!(
            modules,
            "        rows.push({module_name}::GeneratedProducer::analyze(event)?.into_iter().map(normalize_{module_name}).collect());"
        )?;
        writeln!(modules, "    }}")?;
        writeln!(modules, "    let histogram = None;")?;
    }
    writeln!(modules, "    Ok(FuzzUnionCaseResult {{ rows, histogram }})")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    writeln!(
        modules,
        "fn normalize_{module_name}(row: {module_name}::GenRow) -> FuzzUnionRow {{"
    )?;
    writeln!(modules, "    FuzzUnionRow {{")?;
    writeln!(modules, "        channel: row.channel.to_string(),")?;
    writeln!(modules, "        values: vec![")?;
    let first_channel = generated
        .spec
        .channels
        .first()
        .expect("union spec has channels");
    for output in &first_channel.outputs {
        writeln!(
            modules,
            "            ({}.to_string(), {}),",
            format_args!("{:?}", output.name),
            fuzz_value_expr(&output.expr, &output.name)
        )?;
    }
    writeln!(modules, "        ],")?;
    writeln!(modules, "    }}")?;
    writeln!(modules, "}}")?;
    writeln!(modules)?;
    Ok(())
}

fn fuzz_value_expr(expr: &Expr, field: &str) -> String {
    match expr {
        Expr::Count(_) | Expr::CountWhere { .. } => {
            format!("FuzzValue::U32(row.{field})")
        }
        Expr::All { .. } | Expr::Any { .. } | Expr::EitherPairPt { .. } => {
            format!("FuzzValue::Bool(row.{field})")
        }
        Expr::LeadingAttr { .. } => format!("FuzzValue::F64(f64::from(row.{field}))"),
        _ => format!("FuzzValue::F64(row.{field})"),
    }
}

fn systematic_variants(spec: &AnalysisSpec) -> Vec<(String, String)> {
    let mut variants = vec![("Nominal".to_string(), "Nominal".to_string())];
    for systematic in &spec.systematics {
        if let SystematicDef::Weight(systematic) = systematic {
            variants.push((
                format!("{}Up", upper_camel(&systematic.name)),
                format!("{}Up", upper_camel(&systematic.name)),
            ));
            variants.push((
                format!("{}Down", upper_camel(&systematic.name)),
                format!("{}Down", upper_camel(&systematic.name)),
            ));
        }
    }
    for correction in &spec.shape_corrections {
        variants.push((
            format!("{}Up", upper_camel(&correction.name)),
            format!("{}Up", upper_camel(&correction.name)),
        ));
        variants.push((
            format!("{}Down", upper_camel(&correction.name)),
            format!("{}Down", upper_camel(&correction.name)),
        ));
    }
    variants
}

fn upper_camel(value: &str) -> String {
    let mut ident = String::new();
    for part in value.split('_') {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            ident.push(first.to_ascii_uppercase());
            ident.extend(chars);
        }
    }
    ident
}
