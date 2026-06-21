use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_higgs4l_demo::{higgs2e2mu, higgs4mu};
use nano_spec::interpret::{interpret, Value};
use nano_spec::{validate, AnalysisSpec, Catalogue};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const HIGGS4MU_SPEC: &str = include_str!("../../nano-spec/examples/higgs4mu_minimal.toml");

#[test]
fn generated_four_muon_harness_matches_handwritten_reference_on_synthetic_events() {
    for entry in 0..4 {
        let event = synthetic_event(entry);
        let generated = higgs4mu::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.z1_mass, row.z2_mass, row.h_mass));
        let handwritten = reco_zz_to_4l(
            event.vector_ref::<f32>("Muon_pt").unwrap(),
            event.vector_ref::<f32>("Muon_eta").unwrap(),
            event.vector_ref::<f32>("Muon_phi").unwrap(),
            event.vector_ref::<f32>("Muon_mass").unwrap(),
            event.vector_ref::<i32>("Muon_charge").unwrap(),
            91.1876,
        )
        .map(|idx| {
            let z1 = pair_vector(
                "good_muon",
                idx[0],
                event.vector_ref::<f32>("Muon_pt").unwrap(),
                event.vector_ref::<f32>("Muon_eta").unwrap(),
                event.vector_ref::<f32>("Muon_phi").unwrap(),
                event.vector_ref::<f32>("Muon_mass").unwrap(),
            );
            let z2 = pair_vector(
                "good_muon",
                idx[1],
                event.vector_ref::<f32>("Muon_pt").unwrap(),
                event.vector_ref::<f32>("Muon_eta").unwrap(),
                event.vector_ref::<f32>("Muon_phi").unwrap(),
                event.vector_ref::<f32>("Muon_mass").unwrap(),
            );
            let h = combine_vectors(&[z1, z2]);
            (z1.mass, z2.mass, h.mass)
        });

        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

#[test]
fn generated_two_e_two_mu_harness_matches_handwritten_reference_on_synthetic_events() {
    for entry in 0..4 {
        let event = synthetic_event(entry);
        let generated = higgs2e2mu::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.z_el_mass, row.z_mu_mass, row.h_mass));
        let z_el = pair_vector(
            "good_electron",
            [0, 1],
            event.vector_ref::<f32>("Electron_pt").unwrap(),
            event.vector_ref::<f32>("Electron_eta").unwrap(),
            event.vector_ref::<f32>("Electron_phi").unwrap(),
            event.vector_ref::<f32>("Electron_mass").unwrap(),
        );
        let z_mu = pair_vector(
            "good_muon",
            [0, 1],
            event.vector_ref::<f32>("Muon_pt").unwrap(),
            event.vector_ref::<f32>("Muon_eta").unwrap(),
            event.vector_ref::<f32>("Muon_phi").unwrap(),
            event.vector_ref::<f32>("Muon_mass").unwrap(),
        );
        let h = combine_vectors(&[z_mu, z_el]);
        let handwritten = Some((z_el.mass, z_mu.mass, h.mass));

        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

#[test]
fn interpreted_four_muon_harness_matches_generated_code_on_synthetic_events() {
    let spec = AnalysisSpec::from_toml_str(HIGGS4MU_SPEC).expect("parse spec");
    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let plan = validate(&spec, &catalogue).expect("validate spec");

    for entry in 0..4 {
        let event = synthetic_event(entry);
        let generated = higgs4mu::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.z1_mass, row.z2_mass, row.h_mass));
        let interpreted = interpret(&plan, &event).unwrap().map(|row| {
            (
                value_f64(row.get("z1_mass").expect("z1_mass")),
                value_f64(row.get("z2_mass").expect("z2_mass")),
                value_f64(row.get("h_mass").expect("h_mass")),
            )
        });

        assert_eq!(interpreted, generated, "entry {entry}");
    }
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
        BranchSpec::new("nElectron", BranchType::U32),
        BranchSpec::new("Electron_pt", BranchType::VecF32),
        BranchSpec::new("Electron_eta", BranchType::VecF32),
        BranchSpec::new("Electron_phi", BranchType::VecF32),
        BranchSpec::new("Electron_mass", BranchType::VecF32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    vec![
        ("nMuon".to_string(), BranchColumn::U32(vec![4, 4, 4, 4])),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![46.0, 44.0, 25.0, 21.0],
                vec![30.0, 80.0, 38.0, 35.0],
                vec![50.0, 43.0, 31.0, 26.0],
                vec![20.0, 24.0, 55.0, 50.0],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.10, -0.15, 0.80, -0.70],
                vec![0.20, -0.30, 1.10, -1.00],
                vec![0.05, -0.12, 0.70, -0.65],
                vec![1.20, -1.10, 0.30, -0.35],
            ]),
        ),
        (
            "Muon_phi".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.0, 3.05, 1.2, -2.1],
                vec![2.4, -0.6, 0.8, -2.2],
                vec![0.1, 3.0, 1.4, -2.0],
                vec![1.8, -1.1, 0.1, 3.0],
            ]),
        ),
        (
            "Muon_mass".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.105; 4],
                vec![0.105; 4],
                vec![0.105; 4],
                vec![0.105; 4],
            ]),
        ),
        (
            "Muon_charge".to_string(),
            BranchColumn::VecI32(vec![
                vec![1, -1, 1, -1],
                vec![1, -1, -1, 1],
                vec![-1, 1, 1, -1],
                vec![1, -1, 1, -1],
            ]),
        ),
        ("nElectron".to_string(), BranchColumn::U32(vec![2, 2, 2, 2])),
        (
            "Electron_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![38.0, 32.0],
                vec![45.0, 30.0],
                vec![27.0, 22.0],
                vec![52.0, 41.0],
            ]),
        ),
        (
            "Electron_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.25, -0.20],
                vec![1.10, -0.90],
                vec![-1.40, 1.20],
                vec![0.40, -0.50],
            ]),
        ),
        (
            "Electron_phi".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.5, -2.8],
                vec![2.2, -0.4],
                vec![-2.0, 1.1],
                vec![0.9, -2.5],
            ]),
        ),
        (
            "Electron_mass".to_string(),
            BranchColumn::VecF32(vec![vec![0.000_511; 2]; 4]),
        ),
    ]
}

fn reco_zz_to_4l(
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
    charge: &[i32],
    z_reference_mass: f64,
) -> Option<[[usize; 2]; 2]> {
    let mut best_mass = -1_i32;
    let mut best_pair = None;

    for i1 in 0..pt.len() {
        for i2 in i1 + 1..pt.len() {
            if charge[i1] == charge[i2] {
                continue;
            }
            let this_mass = pair_vector("good_muon", [i1, i2], pt, eta, phi, mass).mass;
            if (z_reference_mass - this_mass).abs()
                < (z_reference_mass - f64::from(best_mass)).abs()
            {
                best_mass = this_mass as i32;
                best_pair = Some([i1, i2]);
            }
        }
    }

    let z1 = best_pair?;
    let mut rest = [usize::MAX; 2];
    let mut n_rest = 0;
    for index in 0..4 {
        if index != z1[0] && index != z1[1] {
            rest[n_rest] = index;
            n_rest += 1;
        }
    }
    Some([z1, rest])
}

#[derive(Debug, Clone, Copy)]
struct CandidateVector {
    mass: f64,
    energy: f64,
    px: f64,
    py: f64,
    pz: f64,
}

fn pair_vector(
    _object: &str,
    indices: [usize; 2],
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
) -> CandidateVector {
    let first = four_vector(
        pt[indices[0]],
        eta[indices[0]],
        phi[indices[0]],
        mass[indices[0]],
    );
    let second = four_vector(
        pt[indices[1]],
        eta[indices[1]],
        phi[indices[1]],
        mass[indices[1]],
    );
    combine_raw(&[first, second])
}

fn combine_vectors(items: &[CandidateVector]) -> CandidateVector {
    let mut energy = 0.0;
    let mut px = 0.0;
    let mut py = 0.0;
    let mut pz = 0.0;
    for item in items {
        energy += item.energy;
        px += item.px;
        py += item.py;
        pz += item.pz;
    }
    vector_from_components(energy, px, py, pz)
}

fn combine_raw(items: &[(f64, f64, f64, f64)]) -> CandidateVector {
    let mut energy = 0.0;
    let mut px = 0.0;
    let mut py = 0.0;
    let mut pz = 0.0;
    for (item_e, item_px, item_py, item_pz) in items {
        energy += item_e;
        px += item_px;
        py += item_py;
        pz += item_pz;
    }
    vector_from_components(energy, px, py, pz)
}

fn vector_from_components(energy: f64, px: f64, py: f64, pz: f64) -> CandidateVector {
    CandidateVector {
        mass: (energy * energy - px * px - py * py - pz * pz)
            .max(0.0)
            .sqrt(),
        energy,
        px,
        py,
        pz,
    }
}

fn four_vector(pt: f32, eta: f32, phi: f32, mass: f32) -> (f64, f64, f64, f64) {
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

fn value_f64(value: Value) -> f64 {
    match value {
        Value::F64(value) => value,
        other => panic!("unexpected value {other:?}"),
    }
}
