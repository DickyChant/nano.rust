use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use futures::executor::block_on;
use nano_analysis::Hist1D;
use root_io::RootFile;

#[path = "../examples/full_analysis_workflow.rs"]
mod full_analysis_workflow;

#[test]
fn full_analysis_workflow_emits_valid_multiprocess_datacard_and_shapes() {
    let output_dir = temp_output_dir("full_analysis_workflow");
    let result = full_analysis_workflow::run_to_dir(&output_dir).expect("run worked example");
    let parsed = ParsedDatacard::parse(&result.datacard_text);

    assert!(result.summary.contains("Per-process nominal yields:"));
    assert_eq!(parsed.imax, 1);
    assert_eq!(parsed.jmax, 2);
    assert_eq!(parsed.kmax, 2);
    assert_eq!(parsed.shapes_file, "shapes.root");
    assert!(result.output_dir.join(&parsed.shapes_file).exists());
    assert_eq!(parsed.channels, ["signal_region"]);
    assert_eq!(
        parsed.rate_channels,
        ["signal_region", "signal_region", "signal_region"]
    );
    assert_eq!(parsed.processes, ["signal", "ttbar", "zjets"]);
    assert_eq!(parsed.process_indices, [0, 1, 2]);
    assert_values_close(&parsed.observations, &[4.18]);
    assert_values_close(&parsed.rates, &[1.025, 4.22, 0.852]);
    assert_eq!(parsed.rates.len(), parsed.processes.len());

    let nominal_yields = full_analysis_workflow::process_nominal_yields(&result.histograms);
    assert_values_close(&[nominal_yields["signal"]], &[1.025]);
    assert_values_close(&[nominal_yields["ttbar"]], &[4.22]);
    assert_values_close(&[nominal_yields["zjets"]], &[0.852]);
    assert_values_close(
        &[full_analysis_workflow::data_obs_yield(&result.histograms)],
        &[4.18],
    );

    for systematic in parsed.systematics.values() {
        assert_eq!(systematic.entries.len(), parsed.rates.len());
    }
    assert_eq!(
        parsed.systematic("JesTotal"),
        Some(&SystematicLine {
            kind: "shape".to_string(),
            entries: vec!["1".to_string(), "1".to_string(), "1".to_string()],
        })
    );
    assert_eq!(
        parsed.systematic("MuonSf"),
        Some(&SystematicLine {
            kind: "shape".to_string(),
            entries: vec!["1".to_string(), "1".to_string(), "1".to_string()],
        })
    );

    let shapes = read_shapes(&result.datacard.shapes_path);
    assert_eq!(shapes.len(), 16);
    assert_th1_contents(
        shapes.get("signal_region/signal").unwrap(),
        &[0.54, 0.0, 0.485],
    );
    assert_th1_contents(
        shapes.get("signal_region/signal_JesTotalUp").unwrap(),
        &[0.0, 0.54, 0.485],
    );
    assert_th1_contents(
        shapes.get("signal_region/signal_JesTotalDown").unwrap(),
        &[0.54, 0.485, 0.0],
    );
    assert_th1_contents(
        shapes.get("signal_region/signal_MuonSfUp").unwrap(),
        &[0.59, 0.0, 0.535],
    );
    assert_th1_contents(
        shapes.get("signal_region/signal_MuonSfDown").unwrap(),
        &[0.49, 0.0, 0.435],
    );
    assert_th1_contents(
        shapes.get("signal_region/ttbar").unwrap(),
        &[2.655, 1.08, 0.485],
    );
    assert_th1_contents(
        shapes.get("signal_region/ttbar_JesTotalUp").unwrap(),
        &[1.575, 2.16, 0.485],
    );
    assert_th1_contents(
        shapes.get("signal_region/ttbar_JesTotalDown").unwrap(),
        &[2.655, 1.565, 0.0],
    );
    assert_th1_contents(
        shapes.get("signal_region/zjets").unwrap(),
        &[0.42, 0.432, 0.0],
    );
    assert_th1_contents(
        shapes.get("signal_region/data_obs").unwrap(),
        &[2.13, 1.08, 0.97],
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

#[derive(Debug, PartialEq, Eq)]
struct SystematicLine {
    kind: String,
    entries: Vec<String>,
}

#[derive(Debug)]
struct ParsedDatacard {
    imax: usize,
    jmax: usize,
    kmax: usize,
    shapes_file: String,
    channels: Vec<String>,
    rate_channels: Vec<String>,
    observations: Vec<f64>,
    processes: Vec<String>,
    process_indices: Vec<i32>,
    rates: Vec<f64>,
    systematics: BTreeMap<String, SystematicLine>,
}

impl ParsedDatacard {
    fn parse(text: &str) -> Self {
        let lines = text.lines().collect::<Vec<_>>();
        let imax = parse_header(&lines, "imax");
        let jmax = parse_header(&lines, "jmax");
        let kmax = parse_header(&lines, "kmax");
        let shapes_file = lines
            .iter()
            .find_map(|line| {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                (fields.first() == Some(&"shapes")).then(|| fields[3].to_string())
            })
            .expect("shapes line");
        let bin_rows = rows(&lines, "bin");
        assert_eq!(bin_rows.len(), 2);
        let channels = bin_rows[0][1..]
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        let rate_channels = bin_rows[1][1..]
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        let observations = parse_f64_row(&lines, "observation");
        let rates = parse_f64_row(&lines, "rate");

        let process_rows = rows(&lines, "process");
        assert_eq!(process_rows.len(), 2);
        let processes = process_rows[0][1..]
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        let process_indices = process_rows[1][1..]
            .iter()
            .map(|value| value.parse::<i32>().unwrap())
            .collect::<Vec<_>>();

        let systematics = lines
            .iter()
            .filter_map(|line| {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                if fields.len() >= 2 && matches!(fields[1], "shape" | "lnN") {
                    Some((
                        fields[0].to_string(),
                        SystematicLine {
                            kind: fields[1].to_string(),
                            entries: fields[2..].iter().map(|value| value.to_string()).collect(),
                        },
                    ))
                } else {
                    None
                }
            })
            .collect::<BTreeMap<_, _>>();

        Self {
            imax,
            jmax,
            kmax,
            shapes_file,
            channels,
            rate_channels,
            observations,
            processes,
            process_indices,
            rates,
            systematics,
        }
    }

    fn systematic(&self, name: &str) -> Option<&SystematicLine> {
        self.systematics.get(name)
    }
}

fn rows<'a>(lines: &[&'a str], key: &str) -> Vec<Vec<&'a str>> {
    lines
        .iter()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            (fields.first() == Some(&key)).then_some(fields)
        })
        .collect()
}

fn parse_header(lines: &[&str], key: &str) -> usize {
    lines
        .iter()
        .find_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            (fields.first() == Some(&key)).then(|| fields[1].parse::<usize>().unwrap())
        })
        .unwrap_or_else(|| panic!("missing {key} header"))
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

fn read_shapes(path: &Path) -> BTreeMap<String, root_io::Hist1F> {
    let file = block_on(RootFile::new(path)).unwrap();
    let mut read = BTreeMap::new();
    for item in file.items() {
        let hist = block_on(item.as_th1f()).unwrap();
        read.insert(hist.name.clone(), hist);
    }
    read
}

fn assert_th1_contents(actual: &root_io::Hist1F, bins: &[f64]) {
    let expected = hist_from_bins(bins);
    assert_eq!(actual.axis.nbins, expected.nbins() as i32);
    assert_close(&actual.name, actual.axis.xmin, expected.low());
    assert_close(&actual.name, actual.axis.xmax, expected.high());
    assert_cells_close(
        &actual.name,
        &actual.contents,
        &[
            expected.underflow(),
            expected.bins()[0],
            expected.bins()[1],
            expected.bins()[2],
            expected.overflow(),
        ],
    );
}

fn hist_from_bins(bins: &[f64]) -> Hist1D {
    let mut hist = Hist1D::new(3, 30.0, 180.0);
    for (index, weight) in bins.iter().enumerate() {
        hist.fill_weighted(55.0 + index as f64 * 50.0, *weight);
    }
    hist
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

fn assert_cells_close(context: &str, actual: &[f32], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (actual_cell, expected_cell) in actual.iter().zip(expected) {
        assert_close(
            &format!("{context}: actual_cells={actual:?} expected_cells={expected:?}"),
            *actual_cell as f64,
            *expected_cell,
        );
    }
}

fn assert_values_close(actual: &[f64], expected: &[f64]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_close("values", *actual, *expected);
    }
}

fn assert_close(context: &str, actual: f64, expected: f64) {
    let tolerance = 1.0e-6_f64.max(expected.abs() * 1.0e-6);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}: actual {actual} differs from expected {expected}"
    );
}
