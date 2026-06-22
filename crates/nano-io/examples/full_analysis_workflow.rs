use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use nano_core::{BranchColumn, BranchSchema, Event};
use nano_io::datacard::DatacardOutput;
use nano_io::samples::{
    run_interpreted_samples_with_events, NormalizedProcessHistograms, SampleTable,
};
use nano_spec::{validate, AnalysisSpec, Catalogue};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const CHANNEL: &str = "signal_region";
const NOMINAL: &str = "Nominal";
const JES_UP: &str = "JesTotalUp";
const JES_DOWN: &str = "JesTotalDown";
const MUON_SF_UP: &str = "MuonSfUp";
const MUON_SF_DOWN: &str = "MuonSfDown";

pub const SAMPLE_TABLE_TOML: &str = r#"
lumi = "10 FbInv"

[[sample]]
process = "signal"
signal = true
files = ["signal.root"]
xsec = "0.5 Fb"
sumw = 10.0

[[sample]]
process = "ttbar"
files = ["ttbar_semileptonic.root"]
xsec = "2 Fb"
sumw = 20.0

[[sample]]
process = "ttbar"
files = ["ttbar_dileptonic.root"]
xsec = "1 Fb"
sumw = 20.0

[[sample]]
process = "zjets"
files = ["zjets.root"]
xsec = "1 Fb"
sumw = 25.0

[[sample]]
process = "data_obs"
data = true
files = ["data.root"]
"#;

pub struct WorkflowResult {
    pub output_dir: PathBuf,
    pub datacard: DatacardOutput,
    pub datacard_text: String,
    pub histograms: NormalizedProcessHistograms,
    pub summary: String,
}

pub fn run_to_dir(output_dir: impl AsRef<Path>) -> nano_io::Result<WorkflowResult> {
    let output_dir = output_dir.as_ref().to_path_buf();
    let spec = AnalysisSpec::from_toml_str(&analysis_spec_toml())
        .map_err(|error| nano_io::RootError::other(error.to_string()))?;
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9")
        .map_err(|error| nano_io::RootError::other(error.to_string()))?;
    let plan = validate(&spec, &catalogue)
        .map_err(|errors| nano_io::RootError::other(format_spec_errors(errors)))?;
    let table = SampleTable::from_toml_str(SAMPLE_TABLE_TOML)?;
    let schema = plan.read_branches.clone();

    let histograms = run_interpreted_samples_with_events(&table, &plan, move |path| {
        Ok(synthetic_events(path, schema.clone())
            .into_iter()
            .map(Ok)
            .collect::<Vec<nano_io::Result<Event>>>())
    })?;
    let datacard = histograms.write_datacard(&output_dir)?;
    let datacard_text = std::fs::read_to_string(&datacard.datacard_path)?;
    let summary = render_summary(&histograms, &datacard);

    Ok(WorkflowResult {
        output_dir,
        datacard,
        datacard_text,
        histograms,
        summary,
    })
}

#[cfg(not(test))]
pub fn default_output_dir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "nano_io_full_analysis_workflow_{}_{}",
        std::process::id(),
        unique_suffix()
    ))
}

pub fn analysis_spec_toml() -> String {
    let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../nano-spec/tests/data");
    let lumi_mask = data_dir.join("synthetic_golden.json");
    let muon_sf = data_dir.join("muon_sf.json");
    let jes = data_dir.join("jes_uncertainty.json");

    format!(
        r#"
[analysis]
name = "full_analysis_workflow"
year = "Run2018"

[lumi_mask]
file = "{}"

[objects.good_muon]
source = "Muon"
cuts = ["pt > 30 GeV", "abs(eta) < 2.4"]

[objects.good_jet]
source = "Jet"
cuts = ["pt > 30 GeV", "abs(eta) < 5.0"]

[regions.signal]
require = [
  "HLT_IsoMu24",
  "Flag_goodVertices",
  "count(good_muon) >= 1",
  "count(good_jet) >= 1",
]

[[outputs]]
name = "n_good_muon"
expr = "count(good_muon)"

[[outputs]]
name = "n_good_jet"
expr = "count(good_jet)"

[[outputs]]
name = "leading_jet_pt"
expr = "leading(good_jet).pt"

[[histogram]]
name = "{CHANNEL}"
expr = "leading(good_jet).pt"
bins = 3
range = [30.0, 180.0]

[[correction]]
name = "muon_sf"
kind = "scale_factor"
file = "{}"
correction = "synthetic_muon_sf"
collection = "good_muon"
inputs = [
  {{ name = "eta", from = "eta" }},
  {{ name = "pt", from = "pt" }},
]
systematic = {{ name = "scale_factors", nominal = "nominal", up = "systup", down = "systdown" }}

[[correction]]
name = "jes_total"
kind = "jes"
file = "{}"
correction = "synthetic_jes_uncertainty"
collection = "good_jet"
attr = "pt"
inputs = [
  {{ name = "eta", from = "eta" }},
  {{ name = "pt", from = "pt" }},
]
"#,
        lumi_mask.display(),
        muon_sf.display(),
        jes.display()
    )
}

pub fn process_nominal_yields(histograms: &NormalizedProcessHistograms) -> BTreeMap<String, f64> {
    histograms
        .processes()
        .iter()
        .map(|(process, by_channel)| {
            (
                process.clone(),
                by_channel[CHANNEL]
                    .get(NOMINAL.to_string())
                    .bins()
                    .iter()
                    .sum(),
            )
        })
        .collect()
}

pub fn data_obs_yield(histograms: &NormalizedProcessHistograms) -> f64 {
    histograms.data_obs()[CHANNEL]
        .get(NOMINAL.to_string())
        .bins()
        .iter()
        .sum()
}

pub fn nominal_bins(histograms: &NormalizedProcessHistograms, process: &str) -> Vec<f64> {
    histograms.processes()[process][CHANNEL]
        .get(NOMINAL.to_string())
        .bins()
        .to_vec()
}

fn render_summary(histograms: &NormalizedProcessHistograms, datacard: &DatacardOutput) -> String {
    let mut out = String::new();
    writeln!(out, "Full analysis workflow").expect("write summary");
    writeln!(out, "Channel: {CHANNEL}").expect("write summary");
    writeln!(out, "Samples:").expect("write summary");
    for report in histograms.sample_reports() {
        writeln!(
            out,
            "  sample #{:02} process={} kind={} norm={:.6} selected={}/{}",
            report.sample_index,
            report.process,
            if report.data {
                "data"
            } else if report.signal {
                "signal"
            } else {
                "background"
            },
            report.normalization_factor,
            report.selected,
            report.events_read
        )
        .expect("write summary");
    }
    writeln!(out, "Per-process nominal yields:").expect("write summary");
    for (process, yield_) in process_nominal_yields(histograms) {
        writeln!(
            out,
            "  {process}: {:.6} bins={:?}",
            yield_,
            nominal_bins(histograms, &process)
        )
        .expect("write summary");
    }
    writeln!(
        out,
        "  data_obs: {:.6} bins={:?}",
        data_obs_yield(histograms),
        histograms.data_obs()[CHANNEL]
            .get(NOMINAL.to_string())
            .bins()
    )
    .expect("write summary");
    writeln!(
        out,
        "Systematics: {JES_UP}/{JES_DOWN} as Combine shape, {MUON_SF_UP}/{MUON_SF_DOWN} as Combine shape"
    )
    .expect("write summary");
    writeln!(out, "Datacard: {}", datacard.datacard_path.display()).expect("write summary");
    writeln!(out, "Shapes: {}", datacard.shapes_path.display()).expect("write summary");
    out
}

fn synthetic_events(path: &Path, schema: BranchSchema) -> Vec<Event> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("synthetic file name");
    let (events, mc) = match name {
        "signal.root" => (vec![event_a(), event_c()], true),
        "ttbar_semileptonic.root" => (vec![event_a(), event_b(), event_d()], true),
        "ttbar_dileptonic.root" => (vec![event_c(), event_d()], true),
        "zjets.root" => (vec![event_b(), event_d()], true),
        "data.root" => (
            vec![
                event_a(),
                event_b(),
                event_c(),
                event_d(),
                EventSpec {
                    run: 9999,
                    luminosity_block: 1,
                    ..event_a()
                },
            ],
            false,
        ),
        other => panic!("unexpected synthetic file {other}"),
    };

    (0..events.len())
        .map(|entry| {
            Event::from_columns(schema.clone(), columns(&events, mc), entry)
                .expect("synthetic event columns")
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct EventSpec {
    run: u32,
    luminosity_block: u32,
    trigger: bool,
    good_vertices: bool,
    muon_pt: f32,
    muon_eta: f32,
    jet_pt: f32,
    jet_eta: f32,
}

fn event_a() -> EventSpec {
    EventSpec {
        run: 1001,
        luminosity_block: 1,
        trigger: true,
        good_vertices: true,
        muon_pt: 45.0,
        muon_eta: 0.1,
        jet_pt: 78.0,
        jet_eta: 1.0,
    }
}

fn event_b() -> EventSpec {
    EventSpec {
        run: 1001,
        luminosity_block: 2,
        trigger: true,
        good_vertices: true,
        muon_pt: 45.0,
        muon_eta: 0.1,
        jet_pt: 110.0,
        jet_eta: 1.0,
    }
}

fn event_c() -> EventSpec {
    EventSpec {
        run: 1002,
        luminosity_block: 5,
        trigger: true,
        good_vertices: true,
        muon_pt: 45.0,
        muon_eta: -0.1,
        jet_pt: 132.0,
        jet_eta: -1.0,
    }
}

fn event_d() -> EventSpec {
    EventSpec {
        run: 1001,
        luminosity_block: 3,
        trigger: true,
        good_vertices: true,
        muon_pt: 35.0,
        muon_eta: 0.2,
        jet_pt: 60.0,
        jet_eta: 1.0,
    }
}

fn columns(events: &[EventSpec], mc: bool) -> Vec<(&'static str, BranchColumn)> {
    let mut columns = vec![
        (
            "Flag_goodVertices",
            BranchColumn::Bool(events.iter().map(|event| event.good_vertices).collect()),
        ),
        (
            "HLT_IsoMu24",
            BranchColumn::Bool(events.iter().map(|event| event.trigger).collect()),
        ),
        (
            "Jet_eta",
            BranchColumn::VecF32(events.iter().map(|event| vec![event.jet_eta]).collect()),
        ),
        (
            "Jet_pt",
            BranchColumn::VecF32(events.iter().map(|event| vec![event.jet_pt]).collect()),
        ),
        (
            "Muon_eta",
            BranchColumn::VecF32(events.iter().map(|event| vec![event.muon_eta]).collect()),
        ),
        (
            "Muon_pt",
            BranchColumn::VecF32(events.iter().map(|event| vec![event.muon_pt]).collect()),
        ),
        (
            "luminosityBlock",
            BranchColumn::U32(events.iter().map(|event| event.luminosity_block).collect()),
        ),
        ("nJet", BranchColumn::U32(vec![1; events.len()])),
        ("nMuon", BranchColumn::U32(vec![1; events.len()])),
        (
            "run",
            BranchColumn::U32(events.iter().map(|event| event.run).collect()),
        ),
    ];
    if mc {
        columns.push(("genWeight", BranchColumn::F32(vec![1.0; events.len()])));
    }
    columns
}

#[cfg(not(test))]
fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX_EPOCH")
        .as_nanos()
}

fn format_spec_errors(errors: Vec<nano_spec::SpecError>) -> String {
    errors
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(not(test))]
fn main() -> nano_io::Result<()> {
    let result = run_to_dir(default_output_dir())?;
    print!("{}", result.summary);
    println!();
    println!("datacard.txt:");
    print!("{}", result.datacard_text);
    Ok(())
}
