#![cfg(feature = "http")]

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::events_url_chunked;

const CMS_OPEN_DATA_URL: &str = "https://eospublic.cern.ch//eos/opendata/cms/Run2016H/DoubleMuon/NANOAOD/UL2016_MiniAODv2_NanoAODv9-v1/2510000/127C2975-1B1C-A046-AABF-62B77E757A86.root";

#[test]
fn streams_first_10_events_from_https_byte_ranges() {
    if env_flag("NANO_NO_NET") {
        eprintln!("SKIP: NANO_NO_NET is set");
        return;
    }

    let schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("MET_pt", BranchType::F32),
        BranchSpec::new("run", BranchType::I32),
        BranchSpec::new("event", BranchType::U64),
    ])
    .unwrap();

    let mut events = match events_url_chunked(CMS_OPEN_DATA_URL, &schema, 10) {
        Ok(events) => events,
        Err(err) if should_skip_network(&err.to_string()) => {
            eprintln!("SKIP: remote NanoAOD unavailable: {err}");
            return;
        }
        Err(err) => panic!("failed to open remote NanoAOD: {err}"),
    };

    let file_size = events.file_size();
    let mut rows = 0_usize;
    for event in events.by_ref().take(10) {
        let event = match event {
            Ok(event) => event,
            Err(err) if should_skip_network(&err.to_string()) => {
                eprintln!("SKIP: remote NanoAOD read interrupted: {err}");
                return;
            }
            Err(err) => panic!("failed while reading remote NanoAOD event: {err}"),
        };
        let n_muon = event.scalar::<u32>("nMuon").unwrap();
        let muon_pt = event.vector::<f32>("Muon_pt").unwrap();
        let muon_eta = event.vector::<f32>("Muon_eta").unwrap();
        let met_pt = event.scalar::<f32>("MET_pt").unwrap();
        let run = event.scalar::<i32>("run").unwrap();
        let event_number = event.scalar::<u64>("event").unwrap();

        assert_eq!(muon_pt.len(), n_muon as usize);
        assert_eq!(muon_eta.len(), n_muon as usize);
        assert!(muon_pt.iter().all(|pt| pt.is_finite() && *pt > 0.0));
        assert!(muon_eta
            .iter()
            .all(|eta| eta.is_finite() && eta.abs() < 10.0));
        assert!(met_pt.is_finite() && met_pt >= 0.0);
        assert!(
            (280_000..=285_000).contains(&run),
            "implausible Run2016H run {run}"
        );
        assert!(event_number > 0);
        rows += 1;
    }

    assert_eq!(rows, 10);
    let bytes_fetched = events.bytes_fetched();
    eprintln!("remote NanoAOD first 10 events: fetched {bytes_fetched} bytes of {file_size} total");
    assert!(
        bytes_fetched < file_size / 10,
        "remote read fetched too much data: {bytes_fetched} of {file_size}"
    );
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
        "status 429",
        "status 500",
        "status 502",
        "status 503",
        "status 504",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
