#![cfg(feature = "http")]

#[allow(dead_code)]
#[path = "../examples/higgs4l_opendata.rs"]
mod higgs4l_opendata;

use higgs4l_opendata::{analyze_source, histogram_bins, histogram_counts, DEFAULT_URL};

#[test]
fn streams_df103_higgs4l_signal_and_finds_higgs_peak() {
    if env_flag("NANO_NO_NET") {
        eprintln!("SKIP: NANO_NO_NET is set");
        return;
    }
    std::env::set_var("NANO_HTTP_INSECURE", "1");

    let report = match analyze_source(DEFAULT_URL, None) {
        Ok(report) => report,
        Err(err) if should_skip_network(&err.to_string()) => {
            eprintln!("SKIP: ROOT df103 Higgs signal unavailable: {err}");
            return;
        }
        Err(err) => panic!("failed to analyze ROOT df103 Higgs signal: {err}"),
    };

    assert!(
        report.total_selected() >= 10,
        "expected a nontrivial selected Higgs signal sample, got {}",
        report.total_selected()
    );
    assert_eq!(
        report.h_masses.len(),
        report.total_selected(),
        "one H_mass per selected candidate"
    );
    assert!(
        report.count_4mu > 0 && report.count_4e > 0 && report.count_2e2mu > 0,
        "expected all three channels to contribute: 4mu={}, 4e={}, 2e2mu={}",
        report.count_4mu,
        report.count_4e,
        report.count_2e2mu
    );

    let bins = histogram_bins();
    let counts = histogram_counts(&report.h_masses, &bins);
    let peak_bin = counts
        .iter()
        .enumerate()
        .max_by_key(|(_, count)| *count)
        .map(|(index, _)| index)
        .expect("nonempty histogram");
    let peak_range = bins[peak_bin];
    let higgs_window = report
        .h_masses
        .iter()
        .filter(|mass| **mass >= 115.0 && **mass < 135.0)
        .count();

    assert!(
        (120.0..130.0).contains(&peak_range.0)
            || higgs_window * 2 >= report.total_selected(),
        "expected Higgs signal to peak near 125 GeV; peak bin={peak_range:?}, 115-135 count={higgs_window}, total={}",
        report.total_selected()
    );

    let bytes_fetched = report.bytes_fetched;
    eprintln!(
        "df103 Higgs signal: selected={} 4mu={} 4e={} 2e2mu={} fetched={} of {} bytes",
        report.total_selected(),
        report.count_4mu,
        report.count_4e,
        report.count_2e2mu,
        bytes_fetched,
        report.file_size
    );
    assert!(report.file_size > 0);
    assert!(bytes_fetched > 0);
}

fn should_skip_network(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "http range request failed",
        "certificate",
        "tls",
        "dns",
        "timed out",
        "timeout",
        "connection",
        "status 404",
        "status 429",
        "status 500",
        "status 502",
        "status 503",
        "status 504",
        "not found",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
