#![cfg(feature = "http")]

#[allow(dead_code)]
#[path = "../examples/higgs4l_opendata.rs"]
mod higgs4l_opendata;

use higgs4l_opendata::{analyze_source, histogram_bins, histogram_counts, DEFAULT_URL};

const ROOT_DF103_COUNT_4MU: usize = 9_115;
const ROOT_DF103_COUNT_4E: usize = 5_528;
const ROOT_DF103_COUNT_2E2MU: usize = 12_065;
const ROOT_DF103_H_MASS_COUNTS: [usize; 11] = [52, 85, 105, 311, 2_080, 23_370, 647, 28, 10, 5, 2];

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

    assert_eq!(
        report.count_4mu, ROOT_DF103_COUNT_4MU,
        "4mu selected count must match ROOT df103 exactly"
    );
    assert_eq!(
        report.count_4e, ROOT_DF103_COUNT_4E,
        "4e selected count must match ROOT df103 exactly"
    );
    assert_eq!(
        report.count_2e2mu, ROOT_DF103_COUNT_2E2MU,
        "2e2mu selected count must match ROOT df103 exactly"
    );
    assert_eq!(
        report.total_selected(),
        ROOT_DF103_COUNT_4MU + ROOT_DF103_COUNT_4E + ROOT_DF103_COUNT_2E2MU,
        "total selected count must match ROOT df103 exactly"
    );
    assert_eq!(
        report.h_masses.len(),
        report.total_selected(),
        "one H_mass per selected candidate"
    );

    let bins = histogram_bins();
    let counts = histogram_counts(&report.h_masses, &bins);
    assert_eq!(
        counts, ROOT_DF103_H_MASS_COUNTS,
        "H_mass histogram bins must match ROOT df103 exactly"
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
