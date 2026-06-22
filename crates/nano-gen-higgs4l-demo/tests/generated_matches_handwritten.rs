use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_higgs4l_demo::{higgs2e2mu, higgs4e, higgs4l_all, higgs4mu};
use nano_spec::interpret::{interpret, interpret_union, Value};
use nano_spec::{validate, AnalysisSpec, Catalogue};

const NANOV9_CATALOGUE: &str = include_str!("../../../configs/branches/nanov9.yaml");
const HIGGS4MU_SPEC: &str = include_str!("../../nano-spec/examples/higgs4l.toml");
const HIGGS4L_ALL_SPEC: &str = include_str!("../../nano-spec/examples/higgs4l_all.toml");
const GENERATED_HIGGS4L_ALL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/generated_higgs4l_all.rs"));

#[test]
fn generated_full_four_muon_matches_df103_reference_on_synthetic_events() {
    for entry in 0..5 {
        let event = synthetic_event(entry);
        let generated = higgs4mu::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row_bits(row.z1_mass, row.z2_mass, row.h_mass));
        let reference = same_flavor_reference(&event, "Muon", 5.0, 2.4, "pfRelIso04_all");
        assert_eq!(generated, reference, "entry {entry}");
    }
}

#[test]
fn generated_full_four_electron_matches_df103_reference_on_synthetic_events() {
    for entry in 0..5 {
        let event = synthetic_event(entry);
        let generated = higgs4e::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row_bits(row.z1_mass, row.z2_mass, row.h_mass));
        let reference = same_flavor_reference(&event, "Electron", 7.0, 2.5, "pfRelIso03_all");
        assert_eq!(generated, reference, "entry {entry}");
    }
}

#[test]
fn generated_full_two_e_two_mu_matches_df103_reference_on_synthetic_events() {
    for entry in 0..5 {
        let event = synthetic_event(entry);
        let generated = higgs2e2mu::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row_bits(row.z1_mass, row.z2_mass, row.h_mass));
        let reference = mixed_reference(&event);
        assert_eq!(generated, reference, "entry {entry}");
    }
}

#[test]
fn either_pair_pt_accepts_either_same_flavour_pair() {
    let muon_pair_passes = synthetic_event(2);
    let electron_pair_passes = synthetic_event(3);
    let neither_pair_passes = synthetic_event(4);

    assert!(higgs2e2mu::GeneratedProducer::analyze(&muon_pair_passes)
        .unwrap()
        .is_some());
    assert!(
        higgs2e2mu::GeneratedProducer::analyze(&electron_pair_passes)
            .unwrap()
            .is_some()
    );
    assert!(higgs2e2mu::GeneratedProducer::analyze(&neither_pair_passes)
        .unwrap()
        .is_none());
}

#[test]
fn interpreted_full_four_muon_matches_generated_code_on_synthetic_events() {
    let spec = AnalysisSpec::from_toml_str(HIGGS4MU_SPEC).expect("parse spec");
    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let plan = validate(&spec, &catalogue).expect("validate spec");
    for entry in 0..5 {
        let event = synthetic_event(entry);
        let generated = higgs4mu::GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| row_bits(row.z1_mass, row.z2_mass, row.h_mass));
        let interpreted = interpret(&plan, &event)
            .unwrap_or_else(|error| panic!("entry {entry}: {error:?}"))
            .map(|row| {
                row_bits(
                    value_f64(row.get("z1_mass").expect("z1_mass")),
                    value_f64(row.get("z2_mass").expect("z2_mass")),
                    value_f64(row.get("h_mass").expect("h_mass")),
                )
            });

        assert_eq!(interpreted, generated, "entry {entry}");
    }
}

#[test]
fn generated_union_matches_per_channel_generated_outputs_on_synthetic_events() {
    for entry in 0..5 {
        let event = synthetic_event(entry);
        let union = union_rows(&event);
        let per_channel = per_channel_rows(&event);
        assert_eq!(union, per_channel, "entry {entry}");
    }
}

#[test]
fn interpreted_union_matches_generated_union_on_synthetic_events() {
    let spec = AnalysisSpec::from_toml_str(HIGGS4L_ALL_SPEC).expect("parse union spec");
    let catalogue =
        Catalogue::from_nanoaod_yaml_str(NANOV9_CATALOGUE, "v9").expect("parse catalogue");
    let plan = validate(&spec, &catalogue).expect("validate union spec");

    for entry in 0..5 {
        let event = synthetic_event(entry);
        let generated = union_rows(&event);
        let interpreted = interpret_union(&plan, &event)
            .unwrap_or_else(|error| panic!("entry {entry}: {error:?}"))
            .into_iter()
            .map(|row| {
                (
                    row.channel,
                    row_bits(
                        value_f64(row.row.get("z1_mass").expect("z1_mass")),
                        value_f64(row.row.get("z2_mass").expect("z2_mass")),
                        value_f64(row.row.get("h_mass").expect("h_mass")),
                    ),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(interpreted, generated, "entry {entry}");
    }
}

#[test]
fn generated_union_fills_histogram_through_weighted_terminal() {
    assert!(GENERATED_HIGGS4L_ALL.contains(".weight(weight)"));
    assert!(GENERATED_HIGGS4L_ALL
        .contains("nano_analysis::fill::<SignalRegion, nano_analysis::Nominal>"));
    assert!(GENERATED_HIGGS4L_ALL.contains("impl SystematicVisitor"));
    assert!(GENERATED_HIGGS4L_ALL.contains("systematic.visit(GenWeightVisitor)"));

    let mut histograms = higgs4l_all::GenHistograms::new();
    let mut selected = 0_usize;
    for entry in 0..5 {
        let event = synthetic_event(entry);
        let rows = higgs4l_all::GeneratedProducer::analyze_and_fill(
            &event,
            &mut histograms,
            higgs4l_all::Systematic::Nominal,
        )
        .unwrap();
        selected += rows.len();
    }

    assert_eq!(histograms.h_mass.sumw(), selected as f64);
    assert_eq!(selected, 4);
}

#[cfg(feature = "http")]
#[test]
fn generated_open_data_counts_match_root_when_enabled() {
    if std::env::var("NANO_RUN_HTTP_TESTS").as_deref() != Ok("1") {
        eprintln!("skipping networked open-data test; set NANO_RUN_HTTP_TESTS=1 to run");
        return;
    }

    let mut events = nano_io::events_url_chunked(
        "https://eospublic.cern.ch//eos/root-eos/cms_opendata_2012_nanoaod_skimmed/SMHiggsToZZTo4L.root",
        &schema(),
        4096,
    )
    .expect("open ROOT URL");
    let mut count_4mu = 0_usize;
    let mut count_4e = 0_usize;
    let mut count_2e2mu = 0_usize;
    let mut union_count_4mu = 0_usize;
    let mut union_count_4e = 0_usize;
    let mut union_count_2e2mu = 0_usize;
    let mut h_masses = Vec::new();
    let mut union_h_masses = Vec::new();

    for event in &mut events {
        let event = event.expect("read event");
        for row in higgs4l_all::GeneratedProducer::analyze(&event).expect("union") {
            match row.channel {
                "four_mu" => union_count_4mu += 1,
                "four_e" => union_count_4e += 1,
                "two_e_two_mu" => union_count_2e2mu += 1,
                other => panic!("unexpected union channel {other}"),
            }
            union_h_masses.push(f64::from(row.h_mass as f32));
        }
        if let Some(row) = higgs4mu::GeneratedProducer::analyze(&event).expect("4mu") {
            count_4mu += 1;
            h_masses.push(f64::from(row.h_mass as f32));
        }
        if let Some(row) = higgs4e::GeneratedProducer::analyze(&event).expect("4e") {
            count_4e += 1;
            h_masses.push(f64::from(row.h_mass as f32));
        }
        if let Some(row) = higgs2e2mu::GeneratedProducer::analyze(&event).expect("2e2mu") {
            count_2e2mu += 1;
            h_masses.push(f64::from(row.h_mass as f32));
        }
    }

    let peak = h_masses
        .iter()
        .filter(|mass| **mass >= 120.0 && **mass < 130.0)
        .count();

    assert_eq!(count_4mu, 9115);
    assert_eq!(count_4e, 5528);
    assert_eq!(count_2e2mu, 12065);
    assert_eq!(count_4mu + count_4e + count_2e2mu, 26708);
    assert_eq!(peak, 23370);
    assert_eq!(union_count_4mu, count_4mu);
    assert_eq!(union_count_4e, count_4e);
    assert_eq!(union_count_2e2mu, count_2e2mu);
    assert_eq!(union_h_masses, h_masses);
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
        BranchSpec::new("Muon_pfRelIso04_all", BranchType::VecF32),
        BranchSpec::new("Muon_dxy", BranchType::VecF32),
        BranchSpec::new("Muon_dz", BranchType::VecF32),
        BranchSpec::new("Muon_dxyErr", BranchType::VecF32),
        BranchSpec::new("Muon_dzErr", BranchType::VecF32),
        BranchSpec::new("nElectron", BranchType::U32),
        BranchSpec::new("Electron_pt", BranchType::VecF32),
        BranchSpec::new("Electron_eta", BranchType::VecF32),
        BranchSpec::new("Electron_phi", BranchType::VecF32),
        BranchSpec::new("Electron_mass", BranchType::VecF32),
        BranchSpec::new("Electron_charge", BranchType::VecI32),
        BranchSpec::new("Electron_pfRelIso03_all", BranchType::VecF32),
        BranchSpec::new("Electron_dxy", BranchType::VecF32),
        BranchSpec::new("Electron_dz", BranchType::VecF32),
        BranchSpec::new("Electron_dxyErr", BranchType::VecF32),
        BranchSpec::new("Electron_dzErr", BranchType::VecF32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    let mu_pt = vec![
        vec![46.0, 44.0, 25.0, 21.0],
        vec![],
        vec![45.0, 30.0],
        vec![12.0, 6.0],
        vec![19.0, 10.5],
    ];
    let mu_eta = vec![
        vec![0.10, -0.15, 0.80, -0.70],
        vec![],
        vec![0.20, -0.25],
        vec![0.30, -0.30],
        vec![0.35, -0.35],
    ];
    let mu_phi = vec![
        vec![0.0, 3.05, 1.2, -2.1],
        vec![],
        vec![0.1, 3.0],
        vec![0.2, -2.9],
        vec![0.4, -2.8],
    ];
    let mu_charge = vec![
        vec![1, -1, 1, -1],
        vec![],
        vec![1, -1],
        vec![1, -1],
        vec![1, -1],
    ];

    let el_pt = vec![
        vec![],
        vec![52.0, 47.0, 28.0, 24.0],
        vec![9.0, 8.0],
        vec![45.0, 30.0],
        vec![19.0, 10.5],
    ];
    let el_eta = vec![
        vec![],
        vec![0.25, -0.20, 0.90, -0.80],
        vec![0.40, -0.40],
        vec![0.10, -0.20],
        vec![0.15, -0.25],
    ];
    let el_phi = vec![
        vec![],
        vec![0.5, -2.8, 1.5, -1.7],
        vec![0.6, -2.6],
        vec![0.3, -2.8],
        vec![0.2, -2.9],
    ];
    let el_charge = vec![
        vec![],
        vec![1, -1, 1, -1],
        vec![1, -1],
        vec![1, -1],
        vec![1, -1],
    ];

    vec![
        ("nMuon".to_string(), BranchColumn::U32(lengths(&mu_pt))),
        ("Muon_pt".to_string(), BranchColumn::VecF32(mu_pt.clone())),
        ("Muon_eta".to_string(), BranchColumn::VecF32(mu_eta.clone())),
        ("Muon_phi".to_string(), BranchColumn::VecF32(mu_phi)),
        (
            "Muon_mass".to_string(),
            BranchColumn::VecF32(fill_like(&mu_pt, 0.105)),
        ),
        ("Muon_charge".to_string(), BranchColumn::VecI32(mu_charge)),
        (
            "Muon_pfRelIso04_all".to_string(),
            BranchColumn::VecF32(fill_like(&mu_pt, 0.1)),
        ),
        (
            "Muon_dxy".to_string(),
            BranchColumn::VecF32(fill_like(&mu_pt, 0.01)),
        ),
        (
            "Muon_dz".to_string(),
            BranchColumn::VecF32(fill_like(&mu_pt, 0.02)),
        ),
        (
            "Muon_dxyErr".to_string(),
            BranchColumn::VecF32(fill_like(&mu_pt, 0.01)),
        ),
        (
            "Muon_dzErr".to_string(),
            BranchColumn::VecF32(fill_like(&mu_pt, 0.02)),
        ),
        ("nElectron".to_string(), BranchColumn::U32(lengths(&el_pt))),
        (
            "Electron_pt".to_string(),
            BranchColumn::VecF32(el_pt.clone()),
        ),
        (
            "Electron_eta".to_string(),
            BranchColumn::VecF32(el_eta.clone()),
        ),
        ("Electron_phi".to_string(), BranchColumn::VecF32(el_phi)),
        (
            "Electron_mass".to_string(),
            BranchColumn::VecF32(fill_like(&el_pt, 0.000_511)),
        ),
        (
            "Electron_charge".to_string(),
            BranchColumn::VecI32(el_charge),
        ),
        (
            "Electron_pfRelIso03_all".to_string(),
            BranchColumn::VecF32(fill_like(&el_pt, 0.1)),
        ),
        (
            "Electron_dxy".to_string(),
            BranchColumn::VecF32(fill_like(&el_pt, 0.01)),
        ),
        (
            "Electron_dz".to_string(),
            BranchColumn::VecF32(fill_like(&el_pt, 0.02)),
        ),
        (
            "Electron_dxyErr".to_string(),
            BranchColumn::VecF32(fill_like(&el_pt, 0.01)),
        ),
        (
            "Electron_dzErr".to_string(),
            BranchColumn::VecF32(fill_like(&el_pt, 0.02)),
        ),
    ]
}

fn lengths(values: &[Vec<f32>]) -> Vec<u32> {
    values.iter().map(|items| items.len() as u32).collect()
}

fn fill_like(values: &[Vec<f32>], value: f32) -> Vec<Vec<f32>> {
    values
        .iter()
        .map(|items| vec![value; items.len()])
        .collect()
}

fn same_flavor_reference(
    event: &Event,
    source: &str,
    min_pt: f32,
    max_abs_eta: f32,
    iso_attr: &str,
) -> Option<(u32, u32, u32)> {
    let pt = vec_f32(event, source, "pt");
    let eta = vec_f32(event, source, "eta");
    let phi = vec_f32(event, source, "phi");
    let mass = vec_f32(event, source, "mass");
    let charge = vec_i32(event, source, "charge");
    let iso = vec_f32(event, source, iso_attr);
    let dxy = vec_f32(event, source, "dxy");
    let dz = vec_f32(event, source, "dz");
    let dxy_err = vec_f32(event, source, "dxyErr");
    let dz_err = vec_f32(event, source, "dzErr");

    if pt.len() < 4
        || !all_abs_lt(iso, 0.40)
        || !all_gt(pt, min_pt)
        || !all_abs_lt(eta, max_abs_eta)
        || !track_quality(dxy, dz, dxy_err, dz_err)
        || pt.len() != 4
        || count_charge(charge, 1) != 2
        || count_charge(charge, -1) != 2
    {
        return None;
    }

    let idx = reco_zz_to_4l(pt, eta, phi, mass, charge)?;
    if !filter_z_dr(&idx, eta, phi) {
        return None;
    }
    let z_masses = compute_z_masses_4l(&idx, pt, eta, phi, mass);
    if !filter_z_candidates(z_masses) {
        return None;
    }
    Some(bits(
        z_masses[0],
        z_masses[1],
        compute_higgs_mass_4l(&idx, pt, eta, phi, mass),
    ))
}

fn mixed_reference(event: &Event) -> Option<(u32, u32, u32)> {
    let mu_pt = vec_f32(event, "Muon", "pt");
    let mu_eta = vec_f32(event, "Muon", "eta");
    let mu_phi = vec_f32(event, "Muon", "phi");
    let mu_mass = vec_f32(event, "Muon", "mass");
    let mu_charge = vec_i32(event, "Muon", "charge");
    let mu_iso = vec_f32(event, "Muon", "pfRelIso04_all");
    let mu_dxy = vec_f32(event, "Muon", "dxy");
    let mu_dz = vec_f32(event, "Muon", "dz");
    let mu_dxy_err = vec_f32(event, "Muon", "dxyErr");
    let mu_dz_err = vec_f32(event, "Muon", "dzErr");

    let el_pt = vec_f32(event, "Electron", "pt");
    let el_eta = vec_f32(event, "Electron", "eta");
    let el_phi = vec_f32(event, "Electron", "phi");
    let el_mass = vec_f32(event, "Electron", "mass");
    let el_charge = vec_i32(event, "Electron", "charge");
    let el_iso = vec_f32(event, "Electron", "pfRelIso03_all");
    let el_dxy = vec_f32(event, "Electron", "dxy");
    let el_dz = vec_f32(event, "Electron", "dz");
    let el_dxy_err = vec_f32(event, "Electron", "dxyErr");
    let el_dz_err = vec_f32(event, "Electron", "dzErr");

    if el_pt.len() < 2
        || mu_pt.len() < 2
        || !all_abs_lt(el_eta, 2.5)
        || !all_abs_lt(mu_eta, 2.4)
        || !pt_cuts(mu_pt, el_pt)
        || delta_r(mu_eta[0], mu_eta[1], mu_phi[0], mu_phi[1]) < 0.02
        || delta_r(el_eta[0], el_eta[1], el_phi[0], el_phi[1]) < 0.02
        || !all_abs_lt(el_iso, 0.40)
        || !all_abs_lt(mu_iso, 0.40)
        || !track_quality(el_dxy, el_dz, el_dxy_err, el_dz_err)
        || !track_quality(mu_dxy, mu_dz, mu_dxy_err, mu_dz_err)
        || sum_charge(el_charge) != 0
        || sum_charge(mu_charge) != 0
    {
        return None;
    }

    let z_masses = compute_z_masses_2e2mu(
        el_pt, el_eta, el_phi, el_mass, mu_pt, mu_eta, mu_phi, mu_mass,
    );
    if !filter_z_candidates(z_masses) {
        return None;
    }
    Some(bits(
        z_masses[0],
        z_masses[1],
        compute_higgs_mass_2e2mu(
            el_pt, el_eta, el_phi, el_mass, mu_pt, mu_eta, mu_phi, mu_mass,
        ),
    ))
}

fn vec_f32<'a>(event: &'a Event, source: &str, attr: &str) -> &'a [f32] {
    event
        .vector_ref::<f32>(&format!("{source}_{attr}"))
        .unwrap()
}

fn vec_i32<'a>(event: &'a Event, source: &str, attr: &str) -> &'a [i32] {
    event
        .vector_ref::<i32>(&format!("{source}_{attr}"))
        .unwrap()
}

fn row_bits(z1: f64, z2: f64, h: f64) -> (u32, u32, u32) {
    bits(z1 as f32, z2 as f32, h as f32)
}

fn union_rows(event: &Event) -> Vec<(String, (u32, u32, u32))> {
    higgs4l_all::GeneratedProducer::analyze(event)
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.channel.to_string(),
                row_bits(row.z1_mass, row.z2_mass, row.h_mass),
            )
        })
        .collect()
}

fn per_channel_rows(event: &Event) -> Vec<(String, (u32, u32, u32))> {
    let mut rows = Vec::new();
    if let Some(row) = higgs4mu::GeneratedProducer::analyze(event).unwrap() {
        rows.push((
            "four_mu".to_string(),
            row_bits(row.z1_mass, row.z2_mass, row.h_mass),
        ));
    }
    if let Some(row) = higgs4e::GeneratedProducer::analyze(event).unwrap() {
        rows.push((
            "four_e".to_string(),
            row_bits(row.z1_mass, row.z2_mass, row.h_mass),
        ));
    }
    if let Some(row) = higgs2e2mu::GeneratedProducer::analyze(event).unwrap() {
        rows.push((
            "two_e_two_mu".to_string(),
            row_bits(row.z1_mass, row.z2_mass, row.h_mass),
        ));
    }
    rows
}

fn bits(z1: f32, z2: f32, h: f32) -> (u32, u32, u32) {
    (z1.to_bits(), z2.to_bits(), h.to_bits())
}

fn reco_zz_to_4l(
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
    charge: &[i32],
) -> Option<[[usize; 2]; 2]> {
    let mut best_mass = -1_i32;
    let mut best_pair = None;
    for i1 in 0..pt.len() {
        for i2 in i1 + 1..pt.len() {
            if charge[i1] == charge[i2] {
                continue;
            }
            let this_mass = invariant_mass(&[
                Lepton::new(pt[i1], eta[i1], phi[i1], mass[i1]),
                Lepton::new(pt[i2], eta[i2], phi[i2], mass[i2]),
            ]);
            if (91.2 - this_mass).abs() < (91.2 - f64::from(best_mass)).abs() {
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

fn compute_z_masses_4l(
    idx: &[[usize; 2]; 2],
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
) -> [f32; 2] {
    let z0 = invariant_mass(&[
        Lepton::new(
            pt[idx[0][0]],
            eta[idx[0][0]],
            phi[idx[0][0]],
            mass[idx[0][0]],
        ),
        Lepton::new(
            pt[idx[0][1]],
            eta[idx[0][1]],
            phi[idx[0][1]],
            mass[idx[0][1]],
        ),
    ]) as f32;
    let z1 = invariant_mass(&[
        Lepton::new(
            pt[idx[1][0]],
            eta[idx[1][0]],
            phi[idx[1][0]],
            mass[idx[1][0]],
        ),
        Lepton::new(
            pt[idx[1][1]],
            eta[idx[1][1]],
            phi[idx[1][1]],
            mass[idx[1][1]],
        ),
    ]) as f32;
    if (f64::from(z0) - 91.2).abs() < (f64::from(z1) - 91.2).abs() {
        [z0, z1]
    } else {
        [z1, z0]
    }
}

fn compute_higgs_mass_4l(
    idx: &[[usize; 2]; 2],
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
) -> f32 {
    invariant_mass(&[
        Lepton::new(
            pt[idx[0][0]],
            eta[idx[0][0]],
            phi[idx[0][0]],
            mass[idx[0][0]],
        ),
        Lepton::new(
            pt[idx[0][1]],
            eta[idx[0][1]],
            phi[idx[0][1]],
            mass[idx[0][1]],
        ),
        Lepton::new(
            pt[idx[1][0]],
            eta[idx[1][0]],
            phi[idx[1][0]],
            mass[idx[1][0]],
        ),
        Lepton::new(
            pt[idx[1][1]],
            eta[idx[1][1]],
            phi[idx[1][1]],
            mass[idx[1][1]],
        ),
    ]) as f32
}

#[allow(clippy::too_many_arguments)]
fn compute_z_masses_2e2mu(
    el_pt: &[f32],
    el_eta: &[f32],
    el_phi: &[f32],
    el_mass: &[f32],
    mu_pt: &[f32],
    mu_eta: &[f32],
    mu_phi: &[f32],
    mu_mass: &[f32],
) -> [f32; 2] {
    let mu_z = invariant_mass(&[
        Lepton::new(mu_pt[0], mu_eta[0], mu_phi[0], mu_mass[0]),
        Lepton::new(mu_pt[1], mu_eta[1], mu_phi[1], mu_mass[1]),
    ]);
    let el_z = invariant_mass(&[
        Lepton::new(el_pt[0], el_eta[0], el_phi[0], el_mass[0]),
        Lepton::new(el_pt[1], el_eta[1], el_phi[1], el_mass[1]),
    ]);
    if (mu_z - 91.2).abs() < (el_z - 91.2).abs() {
        [mu_z as f32, el_z as f32]
    } else {
        [el_z as f32, mu_z as f32]
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_higgs_mass_2e2mu(
    el_pt: &[f32],
    el_eta: &[f32],
    el_phi: &[f32],
    el_mass: &[f32],
    mu_pt: &[f32],
    mu_eta: &[f32],
    mu_phi: &[f32],
    mu_mass: &[f32],
) -> f32 {
    invariant_mass(&[
        Lepton::new(mu_pt[0], mu_eta[0], mu_phi[0], mu_mass[0]),
        Lepton::new(mu_pt[1], mu_eta[1], mu_phi[1], mu_mass[1]),
        Lepton::new(el_pt[0], el_eta[0], el_phi[0], el_mass[0]),
        Lepton::new(el_pt[1], el_eta[1], el_phi[1], el_mass[1]),
    ]) as f32
}

fn filter_z_dr(idx: &[[usize; 2]; 2], eta: &[f32], phi: &[f32]) -> bool {
    idx.iter()
        .all(|pair| delta_r(eta[pair[0]], eta[pair[1]], phi[pair[0]], phi[pair[1]]) >= 0.02)
}

fn filter_z_candidates(z_masses: [f32; 2]) -> bool {
    z_masses[0] > 40.0 && z_masses[0] < 120.0 && z_masses[1] > 12.0 && z_masses[1] < 120.0
}

fn pt_cuts(mu_pt: &[f32], el_pt: &[f32]) -> bool {
    let mut mu_sorted = mu_pt.to_vec();
    mu_sorted.sort_by(|left, right| right.total_cmp(left));
    if mu_sorted[0] > 20.0 && mu_sorted[1] > 10.0 {
        return true;
    }
    let mut el_sorted = el_pt.to_vec();
    el_sorted.sort_by(|left, right| right.total_cmp(left));
    el_sorted[0] > 20.0 && el_sorted[1] > 10.0
}

fn track_quality(dxy: &[f32], dz: &[f32], dxy_err: &[f32], dz_err: &[f32]) -> bool {
    dxy.iter()
        .zip(dz)
        .zip(dxy_err)
        .zip(dz_err)
        .all(|(((dxy, dz), dxy_err), dz_err)| {
            let ip3d = (dxy * dxy + dz * dz).sqrt();
            let err3d = (dxy_err * dxy_err + dz_err * dz_err).sqrt();
            let sip3d = ip3d / err3d;
            sip3d < 4.0 && dxy.abs() < 0.5 && dz.abs() < 1.0
        })
}

fn all_abs_lt(values: &[f32], threshold: f32) -> bool {
    values.iter().all(|value| value.abs() < threshold)
}

fn all_gt(values: &[f32], threshold: f32) -> bool {
    values.iter().all(|value| *value > threshold)
}

fn count_charge(charges: &[i32], target: i32) -> usize {
    charges.iter().filter(|charge| **charge == target).count()
}

fn sum_charge(charges: &[i32]) -> i32 {
    charges.iter().sum()
}

#[derive(Debug, Clone, Copy)]
struct Lepton {
    pt: f64,
    eta: f64,
    phi: f64,
    mass: f64,
}

impl Lepton {
    fn new(pt: f32, eta: f32, phi: f32, mass: f32) -> Self {
        Self {
            pt: f64::from(pt),
            eta: f64::from(eta),
            phi: f64::from(phi),
            mass: f64::from(mass),
        }
    }
}

fn invariant_mass(leptons: &[Lepton]) -> f64 {
    let (mut energy, mut px, mut py, mut pz) = (0.0, 0.0, 0.0, 0.0);
    for lepton in leptons {
        let (e, x, y, z) = four_vector(*lepton);
        energy += e;
        px += x;
        py += y;
        pz += z;
    }
    (energy * energy - px * px - py * py - pz * pz)
        .max(0.0)
        .sqrt()
}

fn four_vector(lepton: Lepton) -> (f64, f64, f64, f64) {
    let px = lepton.pt * lepton.phi.cos();
    let py = lepton.pt * lepton.phi.sin();
    let pz = lepton.pt * lepton.eta.sinh();
    let energy = (px * px + py * py + pz * pz + lepton.mass * lepton.mass).sqrt();
    (energy, px, py, pz)
}

fn delta_r(eta1: f32, eta2: f32, phi1: f32, phi2: f32) -> f32 {
    let deta = eta1 - eta2;
    let dphi = delta_phi(phi1, phi2);
    (deta * deta + dphi * dphi).sqrt()
}

fn delta_phi(phi1: f32, phi2: f32) -> f32 {
    let c = f64::from(std::f32::consts::PI);
    let mut dphi = f64::from(phi2 - phi1) % (2.0 * c);
    if dphi < -c {
        dphi += 2.0 * c;
    }
    if dphi > c {
        dphi -= 2.0 * c;
    }
    dphi as f32
}

fn value_f64(value: Value) -> f64 {
    match value {
        Value::F64(value) => value,
        other => panic!("unexpected value {other:?}"),
    }
}
