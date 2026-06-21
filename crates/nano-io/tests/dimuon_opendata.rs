#![cfg(feature = "http")]

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::events_url_chunked;

const DF102_URL: &str = "https://eospublic.cern.ch//eos/opendata/cms/derived-data/AOD2NanoAODOutreachTool/Run2012BC_DoubleMuParked_Muons.root";

#[test]
fn streams_df102_opendata_dimuon_masses_over_https_byte_ranges() {
    if env_flag("NANO_NO_NET") {
        eprintln!("SKIP: NANO_NO_NET is set");
        return;
    }

    let schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
        BranchSpec::new("Muon_mass", BranchType::VecF32),
        BranchSpec::new("Muon_charge", BranchType::VecI32),
    ])
    .unwrap();

    let mut events = match events_url_chunked(DF102_URL, &schema, 500) {
        Ok(events) => events,
        Err(err) if should_skip_network(&err.to_string()) => {
            eprintln!("SKIP: ROOT df102 NanoAOD unavailable: {err}");
            return;
        }
        Err(err) => panic!("failed to open ROOT df102 NanoAOD: {err}"),
    };

    let file_size = events.file_size();
    let mut rows = 0_usize;
    let mut found_mass = None;
    for event in events.by_ref().take(500) {
        let event = match event {
            Ok(event) => event,
            Err(err) if should_skip_network(&err.to_string()) => {
                eprintln!("SKIP: ROOT df102 NanoAOD read interrupted: {err}");
                return;
            }
            Err(err) => panic!("failed while reading ROOT df102 NanoAOD event: {err}"),
        };
        rows += 1;

        let n_muon = event.scalar::<u32>("nMuon").unwrap() as usize;
        let pt = event.vector_ref::<f32>("Muon_pt").unwrap();
        let eta = event.vector_ref::<f32>("Muon_eta").unwrap();
        let phi = event.vector_ref::<f32>("Muon_phi").unwrap();
        let mass = event.vector_ref::<f32>("Muon_mass").unwrap();
        let charge = event.vector_ref::<i32>("Muon_charge").unwrap();

        assert_eq!(pt.len(), n_muon);
        assert_eq!(eta.len(), n_muon);
        assert_eq!(phi.len(), n_muon);
        assert_eq!(mass.len(), n_muon);
        assert_eq!(charge.len(), n_muon);

        found_mass =
            found_mass.or_else(|| leading_opposite_charge_mass(pt, eta, phi, mass, charge));
    }

    assert_eq!(rows, 500);
    let mass = found_mass.expect("expected at least one opposite-charge dimuon mass");
    assert!(mass.is_finite() && mass > 0.0);

    let bytes_fetched = events.bytes_fetched();
    eprintln!("ROOT df102 first 500 events: fetched {bytes_fetched} bytes of {file_size} total");
    assert!(
        bytes_fetched < file_size / 10,
        "remote read fetched too much data: {bytes_fetched} of {file_size}"
    );
}

fn leading_opposite_charge_mass(
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
    charge: &[i32],
) -> Option<f64> {
    let mut order = (0..pt.len()).collect::<Vec<_>>();
    order.sort_by(|&left, &right| pt[right].total_cmp(&pt[left]));

    for (left_pos, &left) in order.iter().enumerate() {
        for &right in &order[left_pos + 1..] {
            if charge[left] * charge[right] >= 0 {
                continue;
            }
            let value = dimuon_mass(
                (pt[left], eta[left], phi[left], mass[left]),
                (pt[right], eta[right], phi[right], mass[right]),
            );
            if value.is_finite() && value > 0.0 {
                return Some(value);
            }
        }
    }
    None
}

fn dimuon_mass(first: (f32, f32, f32, f32), second: (f32, f32, f32, f32)) -> f64 {
    let (e1, px1, py1, pz1) = four_vector(first);
    let (e2, px2, py2, pz2) = four_vector(second);
    let energy = e1 + e2;
    let px = px1 + px2;
    let py = py1 + py2;
    let pz = pz1 + pz2;
    (energy * energy - px * px - py * py - pz * pz)
        .max(0.0)
        .sqrt()
}

fn four_vector((pt, eta, phi, mass): (f32, f32, f32, f32)) -> (f64, f64, f64, f64) {
    let pt = f64::from(pt);
    let eta = f64::from(eta);
    let phi = f64::from(phi);
    let mass = f64::from(mass);
    let px = pt * phi.cos();
    let py = pt * phi.sin();
    let pz = pt * eta.sinh();
    let energy = (px * px + py * py + pz * pz + mass * mass).sqrt();
    (energy, px, py, pz)
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
