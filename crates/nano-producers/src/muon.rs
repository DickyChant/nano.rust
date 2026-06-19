/*
Channel name: muon
Physics purpose: minimal single-muon Phase-1 plumbing channel for NanoAOD skims.
Why this phase space is useful: selects events with at least one central,
moderately hard reconstructed muon so the typed event model, object grouping,
selection veto, and skim branch writing can be validated end to end.
Selections implemented in code:
- build the Muon collection from Muon_* jagged branches
- keep muons with Muon_pt > 30 GeV
- keep muons with abs(Muon_eta) < 2.4
- veto events with zero selected muons
*/

use nano_core::{Event, Result};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MuonSkimRow {
    pub n_good_muon: u32,
    pub lead_muon_pt: f32,
}

pub struct MuonProducer;

impl MuonProducer {
    pub fn analyze(event: &Event) -> Result<Option<MuonSkimRow>> {
        let muons = event.collection("Muon")?;
        let mut n_good_muon = 0_u32;
        let mut lead_muon_pt = None;

        for muon in muons.iter() {
            let pt = muon.pt()?;
            let eta = muon.eta()?;
            if pt > 30.0 && eta.abs() < 2.4 {
                n_good_muon += 1;
                lead_muon_pt = Some(lead_muon_pt.map_or(pt, |lead: f32| lead.max(pt)));
            }
        }

        Ok(lead_muon_pt.map(|lead_muon_pt| MuonSkimRow {
            n_good_muon,
            lead_muon_pt,
        }))
    }
}
