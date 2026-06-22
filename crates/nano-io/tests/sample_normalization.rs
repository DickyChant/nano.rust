use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use futures::executor::block_on;
use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_io::samples::{run_interpreted_samples_with_events, SampleTable};
use nano_spec::{validate, AnalysisSpec, Catalogue};
use root_io::RootFile;

const SPEC_TOML: &str = r#"
[analysis]
name = "synthetic_shapes"
year = "Run2018"

[objects.met]
source = "Jet"
cuts = []

[regions.signal]
require = ["count(met) >= 1"]

[[outputs]]
name = "met"
expr = "leading(met).pt"

[[histogram]]
name = "sr"
expr = "leading(met).pt"
bins = 2
range = [0.0, 2.0]
"#;

const CATALOGUE_YAML: &str = r#"
nano_branches:
  test:
    source: "synthetic"
    trees:
      Events:
        branches:
          "nJet":
            type: "uint32"
            root_type: "UInt_t"
          "Jet_pt":
            type: "vec_float"
            root_type: "Float_t"
"#;

const SAMPLE_TOML: &str = r#"
lumi = "3 FbInv"

[[sample]]
process = "signal"
signal = true
files = ["signal.root"]
xsec = "0.5 Pb"
sumw = 1000.0

[[sample]]
process = "ttbar"
files = ["ttbar_a.root"]
xsec = "2 Pb"
sumw = 12000.0

[[sample]]
process = "ttbar"
files = ["ttbar_b.root"]
xsec = "1 Pb"
sumw = 6000.0

[[sample]]
process = "qcd"
files = ["qcd.root"]
xsec = "0.25 Pb"
sumw = 3000.0

[[sample]]
process = "data_obs"
data = true
files = ["data.root"]
"#;

#[test]
fn sample_table_normalizes_accumulates_and_writes_multiprocess_datacard() {
    let spec = AnalysisSpec::from_toml_str(SPEC_TOML).expect("parse analysis spec");
    let catalogue =
        Catalogue::from_nanoaod_yaml_str(CATALOGUE_YAML, "test").expect("parse catalogue");
    let plan = validate(&spec, &catalogue).expect("validate plan");
    let table = SampleTable::from_toml_str(SAMPLE_TOML).expect("parse sample table");

    let output = run_interpreted_samples_with_events(&table, &plan, |path| {
        Ok(synthetic_events(path)
            .into_iter()
            .map(Ok)
            .collect::<Vec<nano_io::Result<Event>>>())
    })
    .expect("run synthetic samples");

    let factors = output
        .sample_reports()
        .iter()
        .map(|report| (report.process.as_str(), report.normalization_factor))
        .collect::<Vec<_>>();
    assert_values_close(
        &factors.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
        &[1.5, 0.5, 0.5, 0.25, 1.0],
    );

    let signal = output.processes()["signal"]["sr"].get("Nominal".to_string());
    let ttbar = output.processes()["ttbar"]["sr"].get("Nominal".to_string());
    let qcd = output.processes()["qcd"]["sr"].get("Nominal".to_string());
    let data = output.data_obs()["sr"].get("Nominal".to_string());
    assert_values_close(signal.bins(), &[1.5, 1.5]);
    assert_values_close(ttbar.bins(), &[1.0, 1.5]);
    assert_values_close(qcd.bins(), &[0.5, 0.5]);
    assert_values_close(data.bins(), &[3.0, 4.0]);

    let output_dir = temp_output_dir("sample_normalization");
    let datacard = output.write_datacard(&output_dir).expect("write datacard");
    let text = std::fs::read_to_string(&datacard.datacard_path).expect("read datacard");
    let parsed = ParsedDatacard::parse(&text);
    assert_eq!(parsed.processes, ["qcd", "signal", "ttbar"]);
    assert_eq!(parsed.process_indices, [1, 0, 2]);
    assert_values_close(&parsed.observations, &[7.0]);
    assert_values_close(&parsed.rates, &[1.0, 3.0, 2.5]);

    let read = read_shapes(&datacard.shapes_path);
    assert_cells_close(&read["sr/signal"].contents, &[0.0, 1.5, 1.5, 0.0]);
    assert_cells_close(&read["sr/ttbar"].contents, &[0.0, 1.0, 1.5, 0.0]);
    assert_cells_close(&read["sr/qcd"].contents, &[0.0, 0.5, 0.5, 0.0]);
    assert_cells_close(&read["sr/data_obs"].contents, &[0.0, 3.0, 4.0, 0.0]);

    std::fs::remove_dir_all(output_dir).unwrap();
}

#[test]
fn sample_table_rejects_cross_section_with_luminosity_unit() {
    let bad = SAMPLE_TOML.replace("0.5 Pb", "0.5 FbInv");
    let error = SampleTable::from_toml_str(&bad).expect_err("xsec unit should be rejected");
    let message = error.to_string();
    assert!(
        message.contains("xsec unit `FbInv` is not a cross-section; expected Pb or Fb"),
        "{message}"
    );
}

#[derive(Debug)]
struct ParsedDatacard {
    observations: Vec<f64>,
    processes: Vec<String>,
    process_indices: Vec<i32>,
    rates: Vec<f64>,
}

impl ParsedDatacard {
    fn parse(text: &str) -> Self {
        let lines = text.lines().collect::<Vec<_>>();
        let observations = parse_f64_row(&lines, "observation");
        let rates = parse_f64_row(&lines, "rate");
        let process_rows = lines
            .iter()
            .filter_map(|line| {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                (fields.first() == Some(&"process")).then_some(fields)
            })
            .collect::<Vec<_>>();
        assert_eq!(process_rows.len(), 2);
        let processes = process_rows[0][1..]
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        let process_indices = process_rows[1][1..]
            .iter()
            .map(|value| value.parse::<i32>().unwrap())
            .collect::<Vec<_>>();
        Self {
            observations,
            processes,
            process_indices,
            rates,
        }
    }
}

fn parse_f64_row(lines: &[&str], key: &str) -> Vec<f64> {
    lines
        .iter()
        .find_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            (fields.first() == Some(&key)).then(|| {
                fields[1..]
                    .iter()
                    .map(|value| value.parse::<f64>().unwrap())
                    .collect()
            })
        })
        .unwrap_or_else(|| panic!("missing {key} row"))
}

fn synthetic_events(path: &Path) -> Vec<Event> {
    let values = match path.file_name().and_then(|name| name.to_str()).unwrap() {
        "signal.root" => vec![0.25, 1.25],
        "ttbar_a.root" => vec![0.2, 0.4, 1.4],
        "ttbar_b.root" => vec![1.1, 1.5],
        "qcd.root" => vec![0.1, 0.3, 1.2, 1.8],
        "data.root" => vec![0.1, 0.6, 1.1, 1.2, 1.8, 1.9, 0.2],
        other => panic!("unexpected synthetic file {other}"),
    };
    values
        .iter()
        .enumerate()
        .map(|(entry, _)| {
            Event::from_columns(
                schema(),
                [
                    ("nJet", BranchColumn::U32(vec![1; values.len()])),
                    (
                        "Jet_pt",
                        BranchColumn::VecF32(
                            values.iter().map(|value| vec![*value as f32]).collect(),
                        ),
                    ),
                ],
                entry,
            )
            .unwrap()
        })
        .collect()
}

fn schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nJet", BranchType::U32),
        BranchSpec::new("Jet_pt", BranchType::VecF32),
    ])
    .unwrap()
}

fn read_shapes(path: &Path) -> BTreeMap<String, root_io::Hist1F> {
    let file = block_on(RootFile::new(path)).unwrap();
    let mut read = BTreeMap::new();
    for item in file.items() {
        let hist = block_on(item.as_th1f()).unwrap();
        read.insert(hist.name.clone(), hist);
    }
    read
}

fn temp_output_dir(label: impl fmt::Display) -> PathBuf {
    std::env::temp_dir().join(format!(
        "nano_io_{label}_{}_{}",
        std::process::id(),
        unique_suffix()
    ))
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn assert_cells_close(actual: &[f32], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_close(*actual as f64, *expected);
    }
}

fn assert_values_close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_close(*actual, *expected);
    }
}

fn assert_close(actual: f64, expected: f64) {
    let tolerance = 1.0e-6_f64.max(expected.abs() * 1.0e-6);
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual {actual} differs from expected {expected}"
    );
}
