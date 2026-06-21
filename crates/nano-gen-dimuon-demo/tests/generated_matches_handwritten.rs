use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_dimuon_demo::GeneratedProducer;

#[cfg(feature = "http")]
const DF102_URL: &str = "https://eospublic.cern.ch//eos/opendata/cms/derived-data/AOD2NanoAODOutreachTool/Run2012BC_DoubleMuParked_Muons.root";

#[test]
fn generated_dimuon_producer_matches_handwritten_reference_on_synthetic_events() {
    for entry in 0..6 {
        let event = synthetic_event(entry);

        let generated = GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row.dimuon_mass);
        let handwritten = leading_opposite_charge_mass(
            event.vector_ref::<f32>("Muon_pt").unwrap(),
            event.vector_ref::<f32>("Muon_eta").unwrap(),
            event.vector_ref::<f32>("Muon_phi").unwrap(),
            event.vector_ref::<f32>("Muon_mass").unwrap(),
            event.vector_ref::<i32>("Muon_charge").unwrap(),
        );

        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

#[cfg(feature = "http")]
#[test]
fn generated_dimuon_producer_matches_handwritten_reference_on_df102_opendata() {
    if env_flag("NANO_NO_NET") {
        eprintln!("SKIP: NANO_NO_NET is set");
        return;
    }

    let schema = schema();
    let mut events = match nano_io::events_url_chunked(DF102_URL, &schema, 500) {
        Ok(events) => events,
        Err(err) if should_skip_network(&err.to_string()) => {
            eprintln!("SKIP: ROOT df102 NanoAOD unavailable: {err}");
            return;
        }
        Err(err) => panic!("failed to open ROOT df102 NanoAOD: {err}"),
    };

    let mut rows = 0_usize;
    let mut matched_masses = 0_usize;
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

        let generated = GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row.dimuon_mass);
        let handwritten = leading_opposite_charge_mass(
            event.vector_ref::<f32>("Muon_pt").unwrap(),
            event.vector_ref::<f32>("Muon_eta").unwrap(),
            event.vector_ref::<f32>("Muon_phi").unwrap(),
            event.vector_ref::<f32>("Muon_mass").unwrap(),
            event.vector_ref::<i32>("Muon_charge").unwrap(),
        );
        if generated.is_some() {
            matched_masses += 1;
        }
        assert_eq!(generated, handwritten, "entry {}", event.entry());
    }

    assert_eq!(rows, 500);
    assert!(
        matched_masses > 0,
        "expected at least one opposite-charge dimuon mass"
    );
}

fn synthetic_event(entry: usize) -> Event {
    Event::from_columns(schema(), columns(), entry).unwrap()
}

fn schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
        BranchSpec::new("Muon_mass", BranchType::VecF32),
        BranchSpec::new("Muon_charge", BranchType::VecI32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    vec![
        (
            "nMuon".to_string(),
            BranchColumn::U32(vec![2, 2, 3, 3, 1, 2]),
        ),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![50.0, 40.0],
                vec![45.0, 30.0],
                vec![20.0, 80.0, 70.0],
                vec![60.0, 60.0, 20.0],
                vec![100.0],
                vec![25.0, 15.0],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.1, -0.2],
                vec![0.4, -1.1],
                vec![1.2, -0.7, 0.3],
                vec![0.2, -0.2, 0.9],
                vec![0.0],
                vec![0.5, 0.5],
            ]),
        ),
        (
            "Muon_phi".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.0, std::f32::consts::PI],
                vec![0.3, -2.4],
                vec![2.1, -0.5, 1.0],
                vec![0.1, -0.8, 2.4],
                vec![1.0],
                vec![0.2, -0.2],
            ]),
        ),
        (
            "Muon_mass".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.105, 0.105],
                vec![0.105, 0.105],
                vec![0.105, 0.105, 0.105],
                vec![0.105, 0.105, 0.105],
                vec![0.105],
                vec![0.105, 0.105],
            ]),
        ),
        (
            "Muon_charge".to_string(),
            BranchColumn::VecI32(vec![
                vec![1, -1],
                vec![1, 1],
                vec![1, 1, -1],
                vec![1, -1, -1],
                vec![1],
                vec![1, -1],
            ]),
        ),
    ]
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
                Muon::new(pt[left], eta[left], phi[left], mass[left]),
                Muon::new(pt[right], eta[right], phi[right], mass[right]),
            );
            if value.is_finite() && value > 0.0 {
                return Some(value);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct Muon {
    pt: f64,
    eta: f64,
    phi: f64,
    mass: f64,
}

impl Muon {
    fn new(pt: f32, eta: f32, phi: f32, mass: f32) -> Self {
        Self {
            pt: f64::from(pt),
            eta: f64::from(eta),
            phi: f64::from(phi),
            mass: f64::from(mass),
        }
    }
}

fn dimuon_mass(first: Muon, second: Muon) -> f64 {
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

fn four_vector(muon: Muon) -> (f64, f64, f64, f64) {
    let px = muon.pt * muon.phi.cos();
    let py = muon.pt * muon.phi.sin();
    let pz = muon.pt * muon.eta.sinh();
    let energy = (px * px + py * py + pz * pz + muon.mass * muon.mass).sqrt();
    (energy, px, py, pz)
}

#[cfg(feature = "http")]
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

#[cfg(feature = "http")]
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
