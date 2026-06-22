use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use futures::executor::block_on;
use nano_analysis::{Hist1D, HistSet1D};
use nano_io::writer::{write_histogram_sets, write_histograms};
use root_io::RootFile;

#[test]
fn writes_hist1d_as_roundtrippable_th1f() {
    let path = temp_root_path("single");
    let hist = fixture_hist();

    write_histograms(&path, &[("mass", &hist)]).unwrap();

    let file = block_on(RootFile::new(path.as_path())).unwrap();
    assert_eq!(file.items().len(), 1);
    let read = block_on(file.items()[0].as_th1f()).unwrap();

    assert_eq!(read.name, "mass");
    assert_eq!(read.title, "mass");
    assert_eq!(read.axis.nbins, 3);
    assert_eq!(read.axis.xmin, 0.0);
    assert_eq!(read.axis.xmax, 3.0);
    assert!(read.axis.edges.is_empty());
    assert_close(read.entries, hist.entries());
    assert_close(read.tsumw, hist.bins().iter().sum());
    assert_close(read.tsumw2, hist.bin_sumw2().iter().sum());
    assert_close(read.tsumwx, hist.sumwx());
    assert_close(read.tsumwx2, hist.sumwx2());
    assert_cells_close(
        &read.contents,
        &[
            hist.underflow(),
            hist.bins()[0],
            hist.bins()[1],
            hist.bins()[2],
            hist.overflow(),
        ],
    );
    assert_values_close(
        &read.sumw2,
        &[
            hist.underflow_sumw2(),
            hist.bin_sumw2()[0],
            hist.bin_sumw2()[1],
            hist.bin_sumw2()[2],
            hist.overflow_sumw2(),
        ],
    );

    std::fs::remove_file(path).unwrap();
}

#[test]
fn writes_histset_variations_as_named_th1fs() {
    let path = temp_root_path("set");
    let mut set = HistSet1D::new(["Nominal".to_string(), "JESUp".to_string()], 2, 0.0, 2.0);
    set.get_mut("Nominal".to_string()).fill_weighted(0.5, 1.0);
    set.get_mut("JESUp".to_string()).fill_weighted(1.5, 2.0);

    write_histogram_sets(&path, &[("yield", &set)]).unwrap();

    let file = block_on(RootFile::new(path.as_path())).unwrap();
    assert_eq!(file.items().len(), 2);
    let mut read = BTreeMap::new();
    for item in file.items() {
        let hist = block_on(item.as_th1f()).unwrap();
        read.insert(hist.name.clone(), hist);
    }

    let nominal = read.get("yield_Nominal").unwrap();
    assert_cells_close(&nominal.contents, &[0.0, 1.0, 0.0, 0.0]);
    assert_values_close(&nominal.sumw2, &[0.0, 1.0, 0.0, 0.0]);
    let up = read.get("yield_JESUp").unwrap();
    assert_cells_close(&up.contents, &[0.0, 0.0, 2.0, 0.0]);
    assert_values_close(&up.sumw2, &[0.0, 0.0, 4.0, 0.0]);

    std::fs::remove_file(path).unwrap();
}

fn fixture_hist() -> Hist1D {
    let mut hist = Hist1D::new(3, 0.0, 3.0);
    hist.fill_weighted(-1.0, 2.0);
    hist.fill_weighted(0.5, 1.5);
    hist.fill_weighted(1.5, 2.5);
    hist.fill_weighted(5.0, 3.5);
    hist
}

fn temp_root_path(label: impl fmt::Display) -> PathBuf {
    std::env::temp_dir().join(format!(
        "nano_io_histogram_{label}_{}_{}.root",
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
