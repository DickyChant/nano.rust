use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use futures::executor::block_on;
use nano_analysis::Hist1D;
use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_analysis_demo::{GeneratedProducer, Systematic};
use nano_io::datacard::{Channel, SingleProcessDatacard};
use nano_spec::codegen::generate_producer_source;
use nano_spec::interpret::{
    interpret_and_fill_systematic, InterpretedHistograms, OutputRow, Value,
};
use nano_spec::{validate, AnalysisSpec, Catalogue};
use root_io::RootFile;

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const END_TO_END_SPEC: &str = include_str!("../specs/end_to_end_analysis.toml");

#[test]
fn sf_jes_lumi_mask_systematics_codegen_histograms_and_datacard_compose() {
    let catalogue = Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").unwrap();
    let spec = AnalysisSpec::from_toml_str(END_TO_END_SPEC).unwrap();
    let plan = validate(&spec, &catalogue).unwrap();
    let kir = nano_spec::kir::lower_plan_to_kir(&plan).unwrap();
    nano_spec::kir::verify(&kir).unwrap();
    let generated_source = generate_producer_source(&plan).unwrap();
    assert!(generated_source.contains("pub enum Systematic"));
    assert_eq!(
        Systematic::ALL,
        [
            Systematic::Nominal,
            Systematic::MuonSfUp,
            Systematic::MuonSfDown,
            Systematic::JesTotalUp,
            Systematic::JesTotalDown,
        ]
    );

    let mut generated_histograms = nano_gen_analysis_demo::GenHistograms::new();
    let mut interpreted_histograms = InterpretedHistograms::new(&plan);

    for entry in 0..analysis_event_count() {
        let event = analysis_event(entry);
        for systematic in Systematic::ALL {
            let generated =
                GeneratedProducer::analyze_and_fill(&event, &mut generated_histograms, systematic)
                    .unwrap()
                    .map(generated_row_bits);
            let interpreted = interpret_and_fill_systematic(
                &plan,
                &event,
                &mut interpreted_histograms,
                &format!("{systematic:?}"),
            )
            .unwrap()
            .map(interpreted_row_bits);

            assert_eq!(generated, interpreted, "entry {entry} {systematic:?}");
        }
    }

    assert_eq!(
        GeneratedProducer::analyze(&analysis_event(0))
            .unwrap()
            .map(generated_row_bits),
        Some((1, 2, 110.0_f32.to_bits(), 155.0_f64.to_bits()))
    );
    assert_eq!(
        GeneratedProducer::analyze(&analysis_event(1)).unwrap(),
        None
    );
    assert_eq!(
        GeneratedProducer::analyze(&analysis_event(2)).unwrap(),
        None
    );
    assert_eq!(
        GeneratedProducer::analyze(&analysis_event(3)).unwrap(),
        None
    );

    for systematic in Systematic::ALL {
        assert_eq!(
            generated_histograms.n_good_jet_hist.get(systematic),
            interpreted_histograms
                .get("n_good_jet_hist")
                .unwrap()
                .get(format!("{systematic:?}")),
            "{systematic:?}"
        );
    }
    assert_ne!(
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::MuonSfUp),
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::Nominal)
    );
    assert_ne!(
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::JesTotalUp),
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::Nominal)
    );
    assert_eq!(
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::Nominal)
            .sumw(),
        1.08
    );
    assert!(
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::JesTotalUp)
            .sumw()
            > generated_histograms
                .n_good_jet_hist
                .get(Systematic::Nominal)
                .sumw()
    );

    let output_dir = temp_output_dir("sf_jes_lumi");
    let nominal = generated_histograms
        .n_good_jet_hist
        .get(Systematic::Nominal);
    let request = SingleProcessDatacard::new("signal").with_channel(
        Channel::new("signal_region", nominal, nominal)
            .with_shape_systematic(
                "MuonSf",
                generated_histograms
                    .n_good_jet_hist
                    .get(Systematic::MuonSfUp),
                generated_histograms
                    .n_good_jet_hist
                    .get(Systematic::MuonSfDown),
            )
            .with_shape_systematic(
                "JesTotal",
                generated_histograms
                    .n_good_jet_hist
                    .get(Systematic::JesTotalUp),
                generated_histograms
                    .n_good_jet_hist
                    .get(Systematic::JesTotalDown),
            ),
    );
    let output = request.write(&output_dir).unwrap();
    let datacard = std::fs::read_to_string(&output.datacard_path).unwrap();
    let parsed = ParsedDatacard::parse(&datacard);
    assert_eq!(parsed.imax, 1);
    assert_eq!(parsed.jmax, 0);
    assert_eq!(parsed.kmax, 2);
    assert_eq!(parsed.shapes_file, "shapes.root");
    assert_eq!(parsed.processes, ["signal"]);
    assert_eq!(parsed.process_indices, [0]);
    assert_values_close(&parsed.observations, &[nominal.bins().iter().sum()]);
    assert_values_close(&parsed.rates, &[nominal.bins().iter().sum()]);
    assert_eq!(
        parsed.systematic("MuonSf"),
        Some(&SystematicLine {
            kind: "shape".to_string(),
            entries: vec!["1".to_string()],
        })
    );
    assert_eq!(
        parsed.systematic("JesTotal"),
        Some(&SystematicLine {
            kind: "shape".to_string(),
            entries: vec!["1".to_string()],
        })
    );

    let shapes = read_shapes(&output.shapes_path);
    assert_eq!(shapes.len(), 6);
    assert_th1_matches(shapes.get("signal_region/signal").unwrap(), nominal);
    assert_th1_matches(shapes.get("signal_region/data_obs").unwrap(), nominal);
    assert_th1_matches(
        shapes.get("signal_region/signal_MuonSfUp").unwrap(),
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::MuonSfUp),
    );
    assert_th1_matches(
        shapes.get("signal_region/signal_MuonSfDown").unwrap(),
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::MuonSfDown),
    );
    assert_th1_matches(
        shapes.get("signal_region/signal_JesTotalUp").unwrap(),
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::JesTotalUp),
    );
    assert_th1_matches(
        shapes.get("signal_region/signal_JesTotalDown").unwrap(),
        generated_histograms
            .n_good_jet_hist
            .get(Systematic::JesTotalDown),
    );
    std::fs::remove_dir_all(output_dir).unwrap();
}

fn generated_row_bits(row: nano_gen_analysis_demo::GenRow) -> (u32, u32, u32, u64) {
    (
        row.n_good_muon,
        row.n_good_jet,
        row.leading_jet_pt.to_bits(),
        row.ht.to_bits(),
    )
}

fn interpreted_row_bits(row: OutputRow) -> (u32, u32, u32, u64) {
    let n_good_muon = match row.get("n_good_muon").unwrap() {
        Value::U32(value) => value,
        value => panic!("unexpected n_good_muon {value:?}"),
    };
    let n_good_jet = match row.get("n_good_jet").unwrap() {
        Value::U32(value) => value,
        value => panic!("unexpected n_good_jet {value:?}"),
    };
    let leading_jet_pt = match row.get("leading_jet_pt").unwrap() {
        Value::F64(value) => (value as f32).to_bits(),
        value => panic!("unexpected leading_jet_pt {value:?}"),
    };
    let ht = match row.get("ht").unwrap() {
        Value::F64(value) => value.to_bits(),
        value => panic!("unexpected ht {value:?}"),
    };
    (n_good_muon, n_good_jet, leading_jet_pt, ht)
}

fn analysis_event(entry: usize) -> Event {
    Event::from_columns(analysis_schema(), analysis_columns(), entry).unwrap()
}

fn analysis_event_count() -> usize {
    6
}

fn analysis_schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("Flag_goodVertices", BranchType::Bool),
        BranchSpec::new("HLT_IsoMu24", BranchType::Bool),
        BranchSpec::new("Jet_eta", BranchType::VecF32),
        BranchSpec::new("Jet_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("luminosityBlock", BranchType::U32),
        BranchSpec::new("nJet", BranchType::U32),
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("run", BranchType::U32),
    ])
    .unwrap()
}

fn analysis_columns() -> Vec<(String, BranchColumn)> {
    vec![
        (
            "Flag_goodVertices".to_string(),
            BranchColumn::Bool(vec![true, true, true, true, false, true]),
        ),
        (
            "HLT_IsoMu24".to_string(),
            BranchColumn::Bool(vec![true, true, false, true, true, true]),
        ),
        (
            "Jet_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![1.0, -1.0, 1.0],
                vec![1.0, -1.0, 1.0],
                vec![1.0, -1.0],
                vec![1.0],
                vec![1.0, -1.0],
                vec![1.0, -1.0],
            ]),
        ),
        (
            "Jet_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![29.0, 45.0, 110.0],
                vec![29.0, 45.0, 110.0],
                vec![45.0, 110.0],
                vec![29.0],
                vec![45.0, 110.0],
                vec![45.0, 110.0],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.1],
                vec![0.1],
                vec![0.1],
                vec![2.39, -2.0],
                vec![0.1],
                vec![0.1],
            ]),
        ),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![40.0],
                vec![40.0],
                vec![40.0],
                vec![45.0, 35.0],
                vec![40.0],
                vec![40.0],
            ]),
        ),
        (
            "luminosityBlock".to_string(),
            BranchColumn::U32(vec![1, 4, 2, 5, 10, 1]),
        ),
        (
            "nJet".to_string(),
            BranchColumn::U32(vec![3, 3, 2, 1, 2, 2]),
        ),
        (
            "nMuon".to_string(),
            BranchColumn::U32(vec![1, 1, 1, 2, 1, 1]),
        ),
        (
            "run".to_string(),
            BranchColumn::U32(vec![1001, 1001, 1001, 1002, 1001, 9999]),
        ),
    ]
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
            expected.bins()[2],
            expected.bins()[3],
            expected.bins()[4],
            expected.overflow(),
        ],
    );
    assert_values_close(
        &actual.sumw2,
        &[
            expected.underflow_sumw2(),
            expected.bin_sumw2()[0],
            expected.bin_sumw2()[1],
            expected.bin_sumw2()[2],
            expected.bin_sumw2()[3],
            expected.bin_sumw2()[4],
            expected.overflow_sumw2(),
        ],
    );
}

fn temp_output_dir(label: impl fmt::Display) -> PathBuf {
    std::env::temp_dir().join(format!(
        "nano_gen_analysis_demo_{label}_{}_{}",
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
