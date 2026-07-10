/*
Channel name: wz
Physics purpose: nominal WZ VBS validation path ported from the fused
MitAnalysisRunIII skim/analyze branch.
Selections implemented in code:
- fakeable 3-lepton WZ preselection using the FAKE_MU and FAKE_EL working points
- same-flavor opposite-sign Z candidate nearest the nominal Z mass
- zero-btag signal and b-tagged control-region split
- Run-3 VBS jet phase space with derived trilepton, MET, b-tag, and VBS observables
*/

use nano_core::{BranchSchema, BranchSpec, BranchType, Event, ObjectView, Result};
use std::collections::HashSet;

const Z_MASS: f64 = 91.1876;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WzRegion {
    Signal,
    BTaggedControl,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WzConfig {
    pub region: WzRegion,
    pub met_min: f64,
    pub z_mass_window: f64,
    pub m3l_min: f64,
    pub w_lepton_pt_min: f64,
    pub vbs_mjj_min: f64,
    pub vbs_detajj_min: f64,
    pub vbs_zepvv_max: f64,
    pub jet_eta_cut: f64,
    pub vbs_jet_eta_cut: f64,
    pub btag_threshold: f64,
}

impl Default for WzConfig {
    fn default() -> Self {
        Self {
            region: WzRegion::Signal,
            met_min: 30.0,
            z_mass_window: 15.0,
            m3l_min: 100.0,
            w_lepton_pt_min: 20.0,
            vbs_mjj_min: 500.0,
            vbs_detajj_min: 2.5,
            vbs_zepvv_max: 1.0,
            jet_eta_cut: 4.7,
            vbs_jet_eta_cut: 4.7,
            btag_threshold: 0.4648,
        }
    }
}

impl WzConfig {
    pub fn btagged_control() -> Self {
        Self {
            region: WzRegion::BTaggedControl,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WzRow {
    pub event_num: u64,
    pub weight: f32,
    pub the_cat: i32,
    pub ngood_jets: u32,
    pub nbtag_goodbtag_jet_bjet: u32,
    pub n_fake_mu: u32,
    pub n_fake_el: u32,
    pub ptl1_z: f32,
    pub etal1_z: f32,
    pub phil1_z: f32,
    pub massl1_z: f32,
    pub flavorl1_z: i32,
    pub ptl2_z: f32,
    pub etal2_z: f32,
    pub phil2_z: f32,
    pub massl2_z: f32,
    pub flavorl2_z: i32,
    pub ptl_w: f32,
    pub etal_w_signed: f32,
    pub phil_w: f32,
    pub massl_w: f32,
    pub flavorl_w: i32,
    pub puppimet_pt_def: f32,
    pub puppimet_phi_def: f32,
    pub mll: f32,
    pub mll_z: f32,
    pub m3l: f32,
    pub mt_w: f32,
    pub tri_lepton_flavor: i32,
    pub vbs_mjj: f32,
    pub vbs_ptjj: f32,
    pub vbs_detajj: f32,
    pub vbs_dphijj: f32,
    pub vbs_ptj1: f32,
    pub vbs_ptj2: f32,
    pub vbs_etaj1: f32,
    pub vbs_etaj2: f32,
    pub vbs_phij1: f32,
    pub vbs_phij2: f32,
    pub vbs_massj1: f32,
    pub vbs_massj2: f32,
    pub vbs_btagj1: f32,
    pub vbs_btagj2: f32,
    pub vbs_zepvv: f32,
    pub vbs_zepmax: f32,
    pub vbs_sum_ht: f32,
    pub vbs_ptvv: f32,
    pub vbs_pttot: f32,
    pub vbs_detavvj1: f32,
    pub vbs_detavvj2: f32,
    pub vbs_ptbalance: f32,
    pub vbs_dphijjll: f32,
    pub vbs_rpt: f32,
}

pub struct WzProducer;

impl WzProducer {
    pub fn analyze(event: &Event) -> Result<Option<WzRow>> {
        Self::analyze_with_config(event, WzConfig::default())
    }

    pub fn analyze_with_config(event: &Event, config: WzConfig) -> Result<Option<WzRow>> {
        let leptons = selected_fake_leptons(event)?;
        if leptons.len() != 3 {
            return Ok(None);
        }

        let n_fake_mu = leptons
            .iter()
            .filter(|lepton| lepton.flavor == LeptonFlavor::Muon)
            .count() as u32;
        let n_fake_el = leptons.len() as u32 - n_fake_mu;
        let charge_sum: i32 = leptons.iter().map(|lepton| lepton.charge).sum();
        if charge_sum.abs() != 1 || !leptons.iter().any(|lepton| lepton.p4.pt > 25.0) {
            return Ok(None);
        }

        let met_pt = event.scalar::<f32>("PuppiMET_pt")? as f64;
        let met_phi = event.scalar::<f32>("PuppiMET_phi")? as f64;
        let trilepton = match TrileptonVars::from_leptons(&leptons, met_pt, met_phi) {
            Some(vars) => vars,
            None => return Ok(None),
        };

        if (trilepton.mll - Z_MASS).abs() >= config.z_mass_window
            || trilepton.m3l <= config.m3l_min
            || trilepton.w.p4.pt <= config.w_lepton_pt_min
            || met_pt <= config.met_min
        {
            return Ok(None);
        }
        if !passes_tau_veto(event)? {
            return Ok(None);
        }

        let jet_selection = select_jets(event, &leptons, config)?;
        match config.region {
            WzRegion::Signal if jet_selection.nbtag_goodbtag_jet_bjet != 0 => return Ok(None),
            WzRegion::BTaggedControl if jet_selection.nbtag_goodbtag_jet_bjet == 0 => {
                return Ok(None);
            }
            _ => {}
        }

        let vbs = match VbsVars::from_jets_and_leptons(
            &jet_selection.vbs_jets,
            &leptons,
            met_pt,
            met_phi,
        ) {
            Some(vars) => vars,
            None => return Ok(None),
        };

        if vbs.mjj <= config.vbs_mjj_min
            || vbs.detajj <= config.vbs_detajj_min
            || vbs.zepvv >= config.vbs_zepvv_max
        {
            return Ok(None);
        }

        Ok(Some(WzRow {
            event_num: event_number(event)?,
            weight: analysis_weight(event)?,
            the_cat: 0,
            ngood_jets: jet_selection.ngood_jets,
            nbtag_goodbtag_jet_bjet: jet_selection.nbtag_goodbtag_jet_bjet,
            n_fake_mu,
            n_fake_el,
            ptl1_z: trilepton.z1.p4.pt as f32,
            etal1_z: trilepton.z1.p4.eta as f32,
            phil1_z: trilepton.z1.p4.phi as f32,
            massl1_z: trilepton.z1.p4.mass as f32,
            flavorl1_z: trilepton.z1.flavor as i32,
            ptl2_z: trilepton.z2.p4.pt as f32,
            etal2_z: trilepton.z2.p4.eta as f32,
            phil2_z: trilepton.z2.p4.phi as f32,
            massl2_z: trilepton.z2.p4.mass as f32,
            flavorl2_z: trilepton.z2.flavor as i32,
            ptl_w: trilepton.w.p4.pt as f32,
            etal_w_signed: trilepton.w.p4.eta as f32,
            phil_w: trilepton.w.p4.phi as f32,
            massl_w: trilepton.w.p4.mass as f32,
            flavorl_w: trilepton.w.flavor as i32,
            puppimet_pt_def: met_pt as f32,
            puppimet_phi_def: met_phi as f32,
            mll: trilepton.mll as f32,
            mll_z: (trilepton.mll - Z_MASS).abs() as f32,
            m3l: trilepton.m3l as f32,
            mt_w: trilepton.mt_w as f32,
            tri_lepton_flavor: ((n_fake_mu + 3 * n_fake_el - 3) / 2) as i32,
            vbs_mjj: vbs.mjj as f32,
            vbs_ptjj: vbs.ptjj as f32,
            vbs_detajj: vbs.detajj as f32,
            vbs_dphijj: vbs.dphijj as f32,
            vbs_ptj1: vbs.j1.p4.pt as f32,
            vbs_ptj2: vbs.j2.p4.pt as f32,
            vbs_etaj1: vbs.j1.p4.eta.abs() as f32,
            vbs_etaj2: vbs.j2.p4.eta.abs() as f32,
            vbs_phij1: vbs.j1.p4.phi as f32,
            vbs_phij2: vbs.j2.p4.phi as f32,
            vbs_massj1: vbs.j1.p4.mass as f32,
            vbs_massj2: vbs.j2.p4.mass as f32,
            vbs_btagj1: vbs.j1.btag as f32,
            vbs_btagj2: vbs.j2.btag as f32,
            vbs_zepvv: vbs.zepvv as f32,
            vbs_zepmax: vbs.zepmax as f32,
            vbs_sum_ht: vbs.sum_ht as f32,
            vbs_ptvv: vbs.ptvv as f32,
            vbs_pttot: vbs.pttot as f32,
            vbs_detavvj1: vbs.detavvj1 as f32,
            vbs_detavvj2: vbs.detavvj2 as f32,
            vbs_ptbalance: vbs.ptbalance as f32,
            vbs_dphijjll: vbs.dphijjll as f32,
            vbs_rpt: vbs.rpt as f32,
        }))
    }
}

pub fn wz_schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("event", BranchType::U64),
        BranchSpec::new("PuppiMET_pt", BranchType::F32),
        BranchSpec::new("PuppiMET_phi", BranchType::F32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
        BranchSpec::new("Muon_mass", BranchType::VecF32),
        BranchSpec::new("Muon_charge", BranchType::VecI32),
        BranchSpec::new("Muon_dxy", BranchType::VecF32),
        BranchSpec::new("Muon_dz", BranchType::VecF32),
        BranchSpec::new("Muon_looseId", BranchType::VecBool),
        BranchSpec::new("Muon_mediumPromptId", BranchType::VecBool),
        BranchSpec::new("Muon_pfIsoId", BranchType::VecU8),
        BranchSpec::new("Muon_jetRelIso", BranchType::VecF32),
        BranchSpec::new("Muon_jetIdx", BranchType::VecI16),
        BranchSpec::new("Electron_pt", BranchType::VecF32),
        BranchSpec::new("Electron_eta", BranchType::VecF32),
        BranchSpec::new("Electron_phi", BranchType::VecF32),
        BranchSpec::new("Electron_mass", BranchType::VecF32),
        BranchSpec::new("Electron_charge", BranchType::VecI32),
        BranchSpec::new("Electron_dxy", BranchType::VecF32),
        BranchSpec::new("Electron_dz", BranchType::VecF32),
        BranchSpec::new("Electron_cutBased", BranchType::VecU8),
        BranchSpec::new("Electron_jetRelIso", BranchType::VecF32),
        BranchSpec::new("Electron_jetIdx", BranchType::VecI16),
        BranchSpec::new("Jet_pt", BranchType::VecF32),
        BranchSpec::new("Jet_eta", BranchType::VecF32),
        BranchSpec::new("Jet_phi", BranchType::VecF32),
        BranchSpec::new("Jet_mass", BranchType::VecF32),
        BranchSpec::new("Jet_jetId", BranchType::VecU8),
        BranchSpec::new("Jet_btagRobustParTAK4B", BranchType::VecF32).optional(),
        BranchSpec::new("Jet_btagUParTAK4B", BranchType::VecF32).optional(),
        BranchSpec::new("Tau_pt", BranchType::VecF32),
        BranchSpec::new("Tau_eta", BranchType::VecF32),
        BranchSpec::new("Tau_idDeepTau2018v2p5VSjet", BranchType::VecU8),
        BranchSpec::new("Tau_idDeepTau2018v2p5VSe", BranchType::VecU8),
        BranchSpec::new("Tau_idDeepTau2018v2p5VSmu", BranchType::VecU8),
        BranchSpec::new("weight", BranchType::F32).optional(),
    ])
    .expect("WZ workflow schema is valid")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeptonFlavor {
    Muon = 0,
    Electron = 1,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Lepton {
    p4: FourVector,
    charge: i32,
    flavor: LeptonFlavor,
    jet_idx: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Jet {
    p4: FourVector,
    btag: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FourVector {
    pt: f64,
    eta: f64,
    phi: f64,
    mass: f64,
}

impl FourVector {
    fn new(pt: f64, eta: f64, phi: f64, mass: f64) -> Self {
        Self { pt, eta, phi, mass }
    }

    fn from_met(pt: f64, phi: f64) -> Self {
        Self::new(pt, 0.0, phi, 0.0)
    }

    fn zero() -> CartesianVector {
        CartesianVector {
            px: 0.0,
            py: 0.0,
            pz: 0.0,
            energy: 0.0,
        }
    }

    fn px(self) -> f64 {
        self.pt * self.phi.cos()
    }

    fn py(self) -> f64 {
        self.pt * self.phi.sin()
    }

    fn pz(self) -> f64 {
        self.pt * self.eta.sinh()
    }

    fn energy(self) -> f64 {
        self.mass.hypot(self.pt * self.eta.cosh())
    }

    fn add(self, other: Self) -> CartesianVector {
        self.cartesian().add(other.cartesian())
    }

    fn cartesian(self) -> CartesianVector {
        CartesianVector {
            px: self.px(),
            py: self.py(),
            pz: self.pz(),
            energy: self.energy(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CartesianVector {
    px: f64,
    py: f64,
    pz: f64,
    energy: f64,
}

impl CartesianVector {
    fn add(self, other: Self) -> Self {
        Self {
            px: self.px + other.px,
            py: self.py + other.py,
            pz: self.pz + other.pz,
            energy: self.energy + other.energy,
        }
    }

    fn pt(self) -> f64 {
        self.px.hypot(self.py)
    }

    fn phi(self) -> f64 {
        self.py.atan2(self.px)
    }

    fn eta(self) -> f64 {
        let momentum = self.pt().hypot(self.pz);
        let denominator = momentum - self.pz;
        if denominator <= 0.0 {
            return 0.0;
        }
        0.5 * ((momentum + self.pz) / denominator).ln()
    }

    fn mass(self) -> f64 {
        let mass2 =
            self.energy * self.energy - self.px * self.px - self.py * self.py - self.pz * self.pz;
        mass2.max(0.0).sqrt()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct TrileptonVars {
    z1: Lepton,
    z2: Lepton,
    w: Lepton,
    mll: f64,
    m3l: f64,
    mt_w: f64,
}

impl TrileptonVars {
    fn from_leptons(leptons: &[Lepton], met_pt: f64, met_phi: f64) -> Option<Self> {
        if leptons.len() != 3 {
            return None;
        }

        let mut sorted = leptons.to_vec();
        sorted.sort_by(|left, right| right.p4.pt.total_cmp(&left.p4.pt));

        let mut best_pair = None;
        for i in 0..sorted.len() {
            for j in (i + 1)..sorted.len() {
                if sorted[i].charge == sorted[j].charge || sorted[i].flavor != sorted[j].flavor {
                    continue;
                }
                let mass = sorted[i].p4.add(sorted[j].p4).mass();
                let distance = (mass - Z_MASS).abs();
                if best_pair
                    .map(|(_, _, best_distance)| distance < best_distance)
                    .unwrap_or(true)
                {
                    best_pair = Some((i, j, distance));
                }
            }
        }

        let (z1_index, z2_index, _) = best_pair?;
        let w_index = (0..sorted.len()).find(|index| *index != z1_index && *index != z2_index)?;
        let z1 = sorted[z1_index];
        let z2 = sorted[z2_index];
        let w = sorted[w_index];
        let z = z1.p4.add(z2.p4);
        let total = sorted.iter().fold(FourVector::zero(), |sum, lepton| {
            sum.add(lepton.p4.cartesian())
        });
        let mt_w = (2.0 * w.p4.pt * met_pt * (1.0 - delta_phi(w.p4.phi, met_phi).cos())).sqrt();

        Some(Self {
            z1,
            z2,
            w,
            mll: z.mass(),
            m3l: total.mass(),
            mt_w,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
struct JetSelection {
    ngood_jets: u32,
    nbtag_goodbtag_jet_bjet: u32,
    vbs_jets: Vec<Jet>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct VbsVars {
    j1: Jet,
    j2: Jet,
    mjj: f64,
    ptjj: f64,
    detajj: f64,
    dphijj: f64,
    zepvv: f64,
    zepmax: f64,
    sum_ht: f64,
    ptvv: f64,
    pttot: f64,
    detavvj1: f64,
    detavvj2: f64,
    ptbalance: f64,
    dphijjll: f64,
    rpt: f64,
}

impl VbsVars {
    fn from_jets_and_leptons(
        jets: &[Jet],
        leptons: &[Lepton],
        met_pt: f64,
        met_phi: f64,
    ) -> Option<Self> {
        let j1 = *jets.first()?;
        let j2 = *jets.get(1)?;
        let jj = j1.p4.add(j2.p4);
        let delta_eta_jj = (j1.p4.eta - j2.p4.eta).abs();
        if delta_eta_jj == 0.0 {
            return None;
        }

        let mut lepton_sum = FourVector::zero();
        let mut vv = FourVector::from_met(met_pt, met_phi).cartesian();
        let mut total = vv.add(j1.p4.cartesian()).add(j2.p4.cartesian());
        let mut max_z = 0.0;
        let mut sum_ht = j1.p4.pt + j2.p4.pt + met_pt;
        let jet_eta_midpoint = (j1.p4.eta + j2.p4.eta) / 2.0;

        for lepton in leptons {
            let lepton_cartesian = lepton.p4.cartesian();
            lepton_sum = lepton_sum.add(lepton_cartesian);
            vv = vv.add(lepton_cartesian);
            total = total.add(lepton_cartesian);
            max_z = f64::max(
                max_z,
                (lepton.p4.eta - jet_eta_midpoint).abs() / delta_eta_jj,
            );
            sum_ht += lepton.p4.pt;
        }

        let ptjj = jj.pt();
        Some(Self {
            j1,
            j2,
            mjj: jj.mass(),
            ptjj,
            detajj: delta_eta_jj,
            dphijj: delta_phi(j1.p4.phi, j2.p4.phi),
            zepvv: (vv.eta() - jet_eta_midpoint).abs() / delta_eta_jj,
            zepmax: max_z,
            sum_ht,
            ptvv: vv.pt(),
            pttot: total.pt(),
            detavvj1: (vv.eta() - j1.p4.eta).abs(),
            detavvj2: (vv.eta() - j2.p4.eta).abs(),
            ptbalance: if ptjj > 0.0 {
                (vv.pt() - ptjj) / ptjj
            } else {
                0.0
            },
            dphijjll: delta_phi(jj.phi(), lepton_sum.phi()),
            rpt: if leptons.len() < 2 {
                1.0
            } else {
                (leptons[0].p4.pt * leptons[1].p4.pt) / (j1.p4.pt * j2.p4.pt)
            },
        })
    }
}

fn selected_fake_leptons(event: &Event) -> Result<Vec<Lepton>> {
    let mut leptons = Vec::new();
    let muon_jet_idx_type = event
        .schema()
        .find("Muon_jetIdx")
        .map(|info| info.branch_type)
        .unwrap_or(BranchType::VecI16);
    let electron_jet_idx_type = event
        .schema()
        .find("Electron_jetIdx")
        .map(|info| info.branch_type)
        .unwrap_or(BranchType::VecI16);

    for muon in event.collection("Muon")?.iter() {
        if fake_muon(muon)? {
            leptons.push(Lepton {
                p4: FourVector::new(
                    muon.pt()? as f64,
                    muon.eta()? as f64,
                    muon.phi()? as f64,
                    muon.mass()? as f64,
                ),
                charge: muon.get::<i32>("charge")?,
                flavor: LeptonFlavor::Muon,
                jet_idx: object_i32_attr(muon, "jetIdx", muon_jet_idx_type)?,
            });
        }
    }

    for electron in event.collection("Electron")?.iter() {
        if fake_electron(electron)? {
            leptons.push(Lepton {
                p4: FourVector::new(
                    electron.pt()? as f64,
                    electron.eta()? as f64,
                    electron.phi()? as f64,
                    electron.mass()? as f64,
                ),
                charge: electron.get::<i32>("charge")?,
                flavor: LeptonFlavor::Electron,
                jet_idx: object_i32_attr(electron, "jetIdx", electron_jet_idx_type)?,
            });
        }
    }

    Ok(leptons)
}

fn passes_tau_veto(event: &Event) -> Result<bool> {
    for tau in event.collection("Tau")?.iter() {
        if tau.eta()?.abs() < 2.5
            && tau.pt()? > 20.0
            && tau.get::<u8>("idDeepTau2018v2p5VSjet")? >= 6
            && tau.get::<u8>("idDeepTau2018v2p5VSe")? >= 6
            && tau.get::<u8>("idDeepTau2018v2p5VSmu")? >= 4
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn object_i32_attr(object: &ObjectView<'_>, attr: &str, branch_type: BranchType) -> Result<i32> {
    match branch_type {
        BranchType::VecI16 => Ok(object.get::<i16>(attr)? as i32),
        BranchType::VecI32 => object.get::<i32>(attr),
        _ => object.get::<i32>(attr),
    }
}

fn fake_muon(muon: &ObjectView<'_>) -> Result<bool> {
    Ok(muon.get::<f32>("dxy")?.abs() < 0.05
        && muon.get::<f32>("dz")?.abs() < 0.10
        && muon.eta()?.abs() < 2.4
        && muon.pt()? > 10.0
        && muon.get::<bool>("looseId")?
        && muon.get::<bool>("mediumPromptId")?
        && muon.get::<u8>("pfIsoId")? >= 1
        && muon.get::<f32>("jetRelIso")? < 0.5)
}

fn fake_electron(electron: &ObjectView<'_>) -> Result<bool> {
    Ok(electron.get::<f32>("dxy")?.abs() < 0.05
        && electron.get::<f32>("dz")?.abs() < 0.10
        && electron.eta()?.abs() < 2.5
        && electron.pt()? > 10.0
        && electron.get::<u8>("cutBased")? >= 2
        && electron.get::<f32>("jetRelIso")? < 0.5)
}

fn select_jets(event: &Event, leptons: &[Lepton], config: WzConfig) -> Result<JetSelection> {
    let lepton_jet_indices = leptons
        .iter()
        .filter_map(|lepton| (lepton.jet_idx >= 0).then_some(lepton.jet_idx))
        .collect::<HashSet<_>>();
    let mut clean_jets = Vec::new();

    for jet in event.collection("Jet")?.iter() {
        let pt = jet.pt()? as f64;
        if pt <= 10.0 || lepton_jet_indices.contains(&(jet.index() as i32)) {
            continue;
        }
        if event.has_physical_branch("Jet_jetId") && (jet.get::<u8>("jetId")? & (1 << 1)) == 0 {
            continue;
        }
        clean_jets.push(Jet {
            p4: FourVector::new(pt, jet.eta()? as f64, jet.phi()? as f64, jet.mass()? as f64),
            btag: jet_btag(event, jet)?,
        });
    }

    let ngood_jets = clean_jets
        .iter()
        .filter(|jet| {
            jet.p4.eta.abs() < config.jet_eta_cut
                && jet.p4.pt > 30.0
                && (jet.p4.pt > 50.0 || jet.p4.eta.abs() < 2.5 || jet.p4.eta.abs() > 3.0)
        })
        .count() as u32;
    let nbtag_goodbtag_jet_bjet = clean_jets
        .iter()
        .filter(|jet| {
            jet.p4.eta.abs() < 2.5 && jet.p4.pt > 20.0 && jet.btag > config.btag_threshold
        })
        .count() as u32;
    let mut vbs_jets = clean_jets
        .iter()
        .copied()
        .filter(|jet| jet.p4.eta.abs() < config.vbs_jet_eta_cut && jet.p4.pt > 50.0)
        .collect::<Vec<_>>();
    vbs_jets.sort_by(|left, right| right.p4.pt.total_cmp(&left.p4.pt));

    Ok(JetSelection {
        ngood_jets,
        nbtag_goodbtag_jet_bjet,
        vbs_jets,
    })
}

fn jet_btag(event: &Event, jet: &ObjectView<'_>) -> Result<f64> {
    if event.has_physical_branch("Jet_btagUParTAK4B") {
        return Ok(jet.get::<f32>("btagUParTAK4B")? as f64);
    }
    if event.has_physical_branch("Jet_btagRobustParTAK4B") {
        return Ok(jet.get::<f32>("btagRobustParTAK4B")? as f64);
    }
    Ok(0.0)
}

fn event_number(event: &Event) -> Result<u64> {
    let Some(info) = event.schema().find("event") else {
        return Ok(event.entry() as u64);
    };
    match info.branch_type {
        BranchType::U64 => event.scalar::<u64>("event"),
        BranchType::I64 => Ok(event.scalar::<i64>("event")? as u64),
        BranchType::U32 => Ok(event.scalar::<u32>("event")? as u64),
        BranchType::I32 => Ok(event.scalar::<i32>("event")? as u64),
        _ => Ok(event.entry() as u64),
    }
}

fn analysis_weight(event: &Event) -> Result<f32> {
    match (
        event.has_physical_branch("weight"),
        event.schema().find("weight").map(|info| info.branch_type),
    ) {
        (true, Some(BranchType::F32)) => event.scalar::<f32>("weight"),
        _ => Ok(1.0),
    }
}

fn delta_phi(phi1: f64, phi2: f64) -> f64 {
    let mut result = phi1 - phi2;
    while result > std::f64::consts::PI {
        result -= 2.0 * std::f64::consts::PI;
    }
    while result <= -std::f64::consts::PI {
        result += 2.0 * std::f64::consts::PI;
    }
    result.abs()
}
