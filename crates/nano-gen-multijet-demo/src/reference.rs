use nano_analysis::Hist1D;
use nano_core::Event;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceRow {
    pub ht: f64,
    pub n_selected_jets: u32,
    pub leading_jet_pt: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceHistograms {
    pub ht: Hist1D,
}

impl ReferenceHistograms {
    pub fn new() -> Self {
        Self {
            ht: Hist1D::new(50, 500.0, 3000.0),
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
        let pt = event.vector_ref::<f32>("Jet_pt")?;
        let eta = event.vector_ref::<f32>("Jet_eta")?;

        let selected = selected_jets(pt, eta);
        let n_selected_jets = selected.len() as u32;
        if n_selected_jets < 4 {
            return Ok(None);
        }

        let ht = selected.iter().map(|jet| jet.pt).sum::<f64>();
        if ht <= 500.0 {
            return Ok(None);
        }

        let leading_jet_pt = selected
            .iter()
            .map(|jet| jet.pt)
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);
        if leading_jet_pt <= 100.0 {
            return Ok(None);
        }

        Ok(Some(ReferenceRow {
            ht,
            n_selected_jets,
            leading_jet_pt,
        }))
    }

    pub fn analyze_and_fill(
        event: &Event,
        histograms: &mut ReferenceHistograms,
    ) -> nano_core::Result<Option<ReferenceRow>> {
        let row = Self::analyze(event)?;
        if let Some(row) = row {
            histograms.ht.fill_weighted(row.ht, 1.0);
        }
        Ok(row)
    }
}

#[derive(Debug, Clone, Copy)]
struct Jet {
    pt: f64,
}

fn selected_jets(pt: &[f32], eta: &[f32]) -> Vec<Jet> {
    pt.iter()
        .zip(eta)
        .filter_map(|(&pt, &eta)| {
            let pt = f64::from(pt);
            let eta = f64::from(eta);
            (pt > 30.0 && eta.abs() < 2.5).then_some(Jet { pt })
        })
        .collect()
}
