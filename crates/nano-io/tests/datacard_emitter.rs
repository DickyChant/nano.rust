use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use futures::executor::block_on;
use nano_analysis::Hist1D;
use nano_io::datacard::{
    Channel, FlatWeightSystematic, MultiProcessChannel, MultiProcessDatacard, Process,
    SingleProcessDatacard,
};
use root_io::RootFile;

#[test]
fn writes_single_process_combine_datacard_and_shapes() {
    // This test validates the emitted text structure and ROOT shapes round-trip.
    // Running `combine` or `text2workspace.py` is the external validation step.
    let output_dir = temp_output_dir("single_process");

    let sr_nominal = hist(&[(0.25, 10.0), (1.25, 5.0), (-1.0, 2.0), (3.0, 4.0)]);
    let sr_jes_up = hist(&[(0.25, 11.0), (1.25, 5.5), (-1.0, 2.5), (3.0, 4.5)]);
    let sr_jes_down = hist(&[(0.25, 9.0), (1.25, 4.5), (-1.0, 1.5), (3.0, 3.5)]);
    let sr_data = hist(&[(0.25, 12.0), (1.25, 6.0), (-1.0, 1.0), (3.0, 1.0)]);

    let cr_nominal = hist(&[(0.25, 3.0), (1.25, 7.0)]);
    let cr_jes_up = hist(&[(0.25, 3.3), (1.25, 7.7)]);
    let cr_jes_down = hist(&[(0.25, 2.7), (1.25, 6.3)]);
    let cr_data = hist(&[(0.25, 4.0), (1.25, 8.0)]);

    let request = SingleProcessDatacard::new("signal")
        .with_channel(
            Channel::new("sr", &sr_nominal, &sr_data).with_shape_systematic(
                "JES",
                &sr_jes_up,
                &sr_jes_down,
            ),
        )
        .with_channel(
            Channel::new("cr", &cr_nominal, &cr_data).with_shape_systematic(
                "JES",
                &cr_jes_up,
                &cr_jes_down,
            ),
        )
        .with_flat_weight_systematic(FlatWeightSystematic::new("muon_weight", 1.10, 0.90));

    let output = request.write(&output_dir).unwrap();
    assert!(output.datacard_path.exists());
    assert!(output.shapes_path.exists());

    let text = std::fs::read_to_string(&output.datacard_path).unwrap();
    let parsed = ParsedDatacard::parse(&text);

    assert_eq!(parsed.imax, 2);
    assert_eq!(parsed.jmax, 0);
    assert_eq!(parsed.kmax, 2);
    assert_eq!(parsed.shapes_file, "shapes.root");
    assert!(output_dir.join(&parsed.shapes_file).exists());
    assert_eq!(parsed.processes, ["signal", "signal"]);
    assert_eq!(parsed.process_indices, [0, 0]);
    assert_values_close(&parsed.observations, &[18.0, 12.0]);
    assert_values_close(&parsed.rates, &[15.0, 10.0]);
    assert_eq!(parsed.rates.len(), parsed.processes.len());

    for systematic in parsed.systematics.values() {
        assert_eq!(systematic.entries.len(), parsed.rates.len());
    }
    assert_eq!(
        parsed.systematic("JES"),
        Some(&SystematicLine {
            kind: "shape".to_string(),
            entries: vec!["1".to_string(), "1".to_string()],
        })
    );
    assert_eq!(
        parsed.systematic("muon_weight"),
        Some(&SystematicLine {
            kind: "lnN".to_string(),
            entries: vec!["0.9/1.1".to_string(), "0.9/1.1".to_string()],
        })
    );

    let read = read_shapes(&output.shapes_path);
    assert_eq!(read.len(), 8);
    assert_th1_matches(read.get("sr/signal").unwrap(), &sr_nominal);
    assert_th1_matches(read.get("sr/signal_JESUp").unwrap(), &sr_jes_up);
    assert_th1_matches(read.get("sr/signal_JESDown").unwrap(), &sr_jes_down);
    assert_th1_matches(read.get("sr/data_obs").unwrap(), &sr_data);
    assert_th1_matches(read.get("cr/signal").unwrap(), &cr_nominal);
    assert_th1_matches(read.get("cr/signal_JESUp").unwrap(), &cr_jes_up);
    assert_th1_matches(read.get("cr/signal_JESDown").unwrap(), &cr_jes_down);
    assert_th1_matches(read.get("cr/data_obs").unwrap(), &cr_data);

    std::fs::remove_dir_all(output_dir).unwrap();
}

#[test]
fn writes_multi_process_combine_datacard_and_shapes() {
    // This slice covers emitting per-process histograms that are already
    // provided by the caller. Building those histograms from multiple samples
    // with per-sample xsec*lumi/sumw normalization belongs in the
    // sample/normalization layer. Running `combine` is still the external
    // validation step.
    let output_dir = temp_output_dir("multi_process");

    let signal = hist(&[(0.25, 8.0), (1.25, 4.0), (-1.0, 1.0), (3.0, 2.0)]);
    let signal_jes_up = hist(&[(0.25, 8.8), (1.25, 4.4)]);
    let signal_jes_down = hist(&[(0.25, 7.2), (1.25, 3.6)]);
    let ttbar = hist(&[(0.25, 5.0), (1.25, 3.0)]);
    let ttbar_jes_up = hist(&[(0.25, 5.5), (1.25, 3.3)]);
    let ttbar_jes_down = hist(&[(0.25, 4.5), (1.25, 2.7)]);
    let qcd = hist(&[(0.25, 2.0), (1.25, 1.0)]);
    let qcd_jes_up = hist(&[(0.25, 2.2), (1.25, 1.1)]);
    let qcd_jes_down = hist(&[(0.25, 1.8), (1.25, 0.9)]);
    let data = hist(&[(0.25, 15.0), (1.25, 8.0)]);

    let request = MultiProcessDatacard::new().with_channel(
        MultiProcessChannel::new("sr", &data)
            .with_process(Process::new("signal", 0, &signal).with_shape_systematic(
                "JES",
                &signal_jes_up,
                &signal_jes_down,
            ))
            .with_process(Process::new("ttbar", 1, &ttbar).with_shape_systematic(
                "JES",
                &ttbar_jes_up,
                &ttbar_jes_down,
            ))
            .with_process(
                Process::new("qcd", 2, &qcd)
                    .with_shape_systematic("JES", &qcd_jes_up, &qcd_jes_down)
                    .with_flat_weight_systematic(FlatWeightSystematic::new("qcd_norm", 1.20, 0.80)),
            ),
    );

    let output = request.write(&output_dir).unwrap();
    assert!(output.datacard_path.exists());
    assert!(output.shapes_path.exists());

    let text = std::fs::read_to_string(&output.datacard_path).unwrap();
    let parsed = ParsedDatacard::parse(&text);

    assert_eq!(parsed.imax, 1);
    assert_eq!(parsed.jmax, 2);
    assert_eq!(parsed.kmax, 2);
    assert_eq!(parsed.shapes_file, "shapes.root");
    assert_eq!(parsed.processes, ["signal", "ttbar", "qcd"]);
    assert_eq!(parsed.process_indices, [0, 1, 2]);
    assert_eq!(
        parsed
            .process_indices
            .iter()
            .filter(|index| **index <= 0)
            .count(),
        1
    );
    assert_values_close(&parsed.observations, &[23.0]);
    assert_values_close(&parsed.rates, &[12.0, 8.0, 3.0]);
    assert_eq!(parsed.rates.len(), parsed.processes.len());

    for systematic in parsed.systematics.values() {
        assert_eq!(systematic.entries.len(), parsed.rates.len());
    }
    assert_eq!(
        parsed.systematic("JES"),
        Some(&SystematicLine {
            kind: "shape".to_string(),
            entries: vec!["1".to_string(), "1".to_string(), "1".to_string()],
        })
    );
    assert_eq!(
        parsed.systematic("qcd_norm"),
        Some(&SystematicLine {
            kind: "lnN".to_string(),
            entries: vec!["-".to_string(), "-".to_string(), "0.8/1.2".to_string()],
        })
    );

    let read = read_shapes(&output.shapes_path);
    assert_eq!(read.len(), 10);
    assert_th1_matches(read.get("sr/signal").unwrap(), &signal);
    assert_th1_matches(read.get("sr/signal_JESUp").unwrap(), &signal_jes_up);
    assert_th1_matches(read.get("sr/signal_JESDown").unwrap(), &signal_jes_down);
    assert_th1_matches(read.get("sr/ttbar").unwrap(), &ttbar);
    assert_th1_matches(read.get("sr/ttbar_JESUp").unwrap(), &ttbar_jes_up);
    assert_th1_matches(read.get("sr/ttbar_JESDown").unwrap(), &ttbar_jes_down);
    assert_th1_matches(read.get("sr/qcd").unwrap(), &qcd);
    assert_th1_matches(read.get("sr/qcd_JESUp").unwrap(), &qcd_jes_up);
    assert_th1_matches(read.get("sr/qcd_JESDown").unwrap(), &qcd_jes_down);
    assert_th1_matches(read.get("sr/data_obs").unwrap(), &data);

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

fn hist(fills: &[(f64, f64)]) -> Hist1D {
    let mut hist = Hist1D::new(2, 0.0, 2.0);
    for (value, weight) in fills {
        hist.fill_weighted(*value, *weight);
    }
    hist
}

fn assert_th1_matches(actual: &root_io::Hist1F, expected: &Hist1D) {
    assert_eq!(actual.axis.nbins, expected.nbins() as i32);
    assert_close(actual.axis.xmin, expected.low());
    assert_close(actual.axis.xmax, expected.high());
    assert!(actual.axis.edges.is_empty());
    assert_close(actual.entries, expected.entries());
    assert_close(actual.tsumw, expected.bins().iter().sum());
    assert_close(actual.tsumw2, expected.bin_sumw2().iter().sum());
    assert_close(actual.tsumwx, expected.sumwx());
    assert_close(actual.tsumwx2, expected.sumwx2());
    assert_cells_close(
        &actual.contents,
        &[
            expected.underflow(),
            expected.bins()[0],
            expected.bins()[1],
            expected.overflow(),
        ],
    );
    assert_values_close(
        &actual.sumw2,
        &[
            expected.underflow_sumw2(),
            expected.bin_sumw2()[0],
            expected.bin_sumw2()[1],
            expected.overflow_sumw2(),
        ],
    );
}

fn temp_output_dir(label: impl fmt::Display) -> PathBuf {
    std::env::temp_dir().join(format!(
        "nano_io_datacard_{label}_{}_{}",
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
