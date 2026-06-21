#![cfg(feature = "http")]

#[path = "../examples/higgs4l_stack_opendata.rs"]
#[allow(dead_code)]
mod higgs4l_stack_opendata;

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

#[test]
fn full_stack_integrals_are_sane_with_network() {
    // Heavy: reads all 8 skimmed Open Data samples over HTTPS (~minutes). Opt-in
    // only, so CI does not run it (CI does not set NANO_NET_HEAVY); run locally
    // with NANO_NET_HEAVY=1.
    if !env_flag("NANO_NET_HEAVY") {
        eprintln!("skipping heavy networked Higgs stack test (set NANO_NET_HEAVY=1 to run)");
        return;
    }

    std::env::set_var("NANO_HTTP_INSECURE", "1");
    let report = higgs4l_stack_opendata::analyze_stack(
        &higgs4l_stack_opendata::SourceConfig::default(),
        None,
    )
    .expect("Higgs stack Open Data samples should analyze");

    let histograms = &report.histograms;
    assert!(histograms.total_data() > 10.0, "data integral is too small");
    assert!(
        histograms.total_background() + histograms.total_signal() > 1.0,
        "total MC integral is too small"
    );
    assert!(
        histograms.total_background() > histograms.total_signal() * 0.1,
        "background integral is unexpectedly tiny"
    );

    let peak_bin = histograms
        .signal_peak_bin()
        .expect("signal histogram should have a maximum bin");
    let low = histograms.edges[peak_bin];
    let high = histograms.edges[peak_bin + 1];
    // The reconstructed m(4l) peak straddles 125 GeV across two adjacent bins
    // (e.g. [121.9,125) edges out [125,128) by a hair), so accept the peak bin
    // overlapping the Higgs window [120,130].
    assert!(
        high > 120.0 && low < 130.0,
        "signal peak should be in the Higgs window [120,130], got [{low}, {high})"
    );
}
