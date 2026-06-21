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
    if env_flag("NANO_NO_NET") {
        eprintln!("skipping networked Higgs stack Open Data test because NANO_NO_NET is set");
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
    assert!(
        low <= 125.0 && 125.0 < high,
        "signal peak should be in the Higgs bin, got [{low}, {high})"
    );
}
