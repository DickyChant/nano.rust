use nano_analysis::Hist1D;
use nano_core::Event;

const Z_MASS_GEV: f64 = 91.1876;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceRow {
    pub dimuon_mass: f64,
    pub dimuon_pt: f64,
    pub leading_muon_pt: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceHistograms {
    pub dimuon_mass: Hist1D,
}

impl ReferenceHistograms {
    pub fn new() -> Self {
        Self {
            dimuon_mass: Hist1D::new(40, 70.0, 110.0),
        }
    }
}

impl Default for ReferenceHistograms {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ReferenceProducer;

impl ReferenceProducer {
    pub fn analyze(event: &Event) -> nano_core::Result<Option<ReferenceRow>> {
        let pt = event.vector_ref::<f32>("Muon_pt")?;
        let eta = event.vector_ref::<f32>("Muon_eta")?;
        let phi = event.vector_ref::<f32>("Muon_phi")?;
        let mass = event.vector_ref::<f32>("Muon_mass")?;
        let charge = event.vector_ref::<i32>("Muon_charge")?;

        let selected = selected_muons(pt, eta, phi, mass, charge);
        let Some(pair) = nearest_z_pair(&selected) else {
            return Ok(None);
        };
        if !(pair.mass >= 70.0 && pair.mass <= 110.0 && pair.pt > 15.0) {
            return Ok(None);
        }
        let leading_muon_pt = selected
            .iter()
            .map(|muon| muon.pt)
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);
        Ok(Some(ReferenceRow {
            dimuon_mass: pair.mass,
            dimuon_pt: pair.pt,
            leading_muon_pt,
        }))
    }

    pub fn analyze_and_fill(
        event: &Event,
        histograms: &mut ReferenceHistograms,
    ) -> nano_core::Result<Option<ReferenceRow>> {
        let row = Self::analyze(event)?;
        if let Some(row) = row {
            histograms.dimuon_mass.fill_weighted(row.dimuon_mass, 1.0);
        }
        Ok(row)
    }
}

#[derive(Debug, Clone, Copy)]
struct Muon {
    pt: f64,
    eta: f64,
    phi: f64,
    mass: f64,
    charge: i32,
}

#[derive(Debug, Clone, Copy)]
struct Pair {
    mass: f64,
    pt: f64,
}

fn selected_muons(pt: &[f32], eta: &[f32], phi: &[f32], mass: &[f32], charge: &[i32]) -> Vec<Muon> {
    pt.iter()
        .zip(eta)
        .zip(phi)
        .zip(mass)
        .zip(charge)
        .filter_map(|((((&pt, &eta), &phi), &mass), &charge)| {
            let pt = f64::from(pt);
            let eta = f64::from(eta);
            (pt > 25.0 && eta.abs() < 2.4).then_some(Muon {
                pt,
                eta,
                phi: f64::from(phi),
                mass: f64::from(mass),
                charge,
            })
        })
        .collect()
}

fn nearest_z_pair(muons: &[Muon]) -> Option<Pair> {
    let mut order = (0..muons.len()).collect::<Vec<_>>();
    order.sort_by(|&left, &right| muons[right].pt.total_cmp(&muons[left].pt));

    let mut best = None;
    let mut best_diff = None::<f64>;
    for (left_pos, &left) in order.iter().enumerate() {
        for &right in &order[left_pos + 1..] {
            let first = muons[left];
            let second = muons[right];
            if first.charge * second.charge >= 0 {
                continue;
            }
            let pair = combine(first, second);
            if !pair.mass.is_finite() || pair.mass <= 0.0 {
                continue;
            }
            let diff = (pair.mass - Z_MASS_GEV).abs();
            if best_diff.is_none_or(|best| diff < best) {
                best_diff = Some(diff);
                best = Some(pair);
            }
        }
    }
    best
}

fn combine(first: Muon, second: Muon) -> Pair {
    let (e1, px1, py1, pz1) = four_vector(first);
    let (e2, px2, py2, pz2) = four_vector(second);
    let energy = e1 + e2;
    let px = px1 + px2;
    let py = py1 + py2;
    let pz = pz1 + pz2;
    let mass = (energy * energy - px * px - py * py - pz * pz)
        .max(0.0)
        .sqrt();
    let pt = (px * px + py * py).sqrt();
    Pair { mass, pt }
}

fn four_vector(muon: Muon) -> (f64, f64, f64, f64) {
    let px = muon.pt * muon.phi.cos();
    let py = muon.pt * muon.phi.sin();
    let pz = muon.pt * muon.eta.sinh();
    let energy = (px * px + py * py + pz * pz + muon.mass * muon.mass).sqrt();
    (energy, px, py, pz)
}
