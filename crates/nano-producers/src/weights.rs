//! Channel-side JME correction plumbing for typed event selections.
//!
//! JME JES/JER variations are shape variations: they move reconstructed jet
//! four-vectors, so jet selections and jet-derived observables must be
//! recomputed from the varied jets. They are deliberately not encoded as an
//! [`nano_analysis::EventWeight`]. Later pileup, lepton scale factors, and
//! trigger weights can still be carried as separate normalization weights.

use nano_analysis::{
    passes_muon_signal_selection, Ev, EventWeight, Nominal, Raw, SignalRegion, Weighted,
};
use nano_core::Event;
use nano_corrections::{Correction, CorrectionError, CorrectionSet, Value};
use std::error::Error as StdError;
use std::fmt;
use std::path::Path;

use crate::MuonSkimRow;

pub const RUN2_2016POSTVFP_AK4PFPUPPI_JES_TOTAL: &str = "Summer19UL16_V7_MC_Total_AK4PFPuppi";
pub const RUN2_2016POSTVFP_AK4PFPUPPI_JER_SCALE_FACTOR: &str =
    "Summer20UL16_JRV3_MC_ScaleFactor_AK4PFPuppi";

/// Closed JME shape-variation axis used by the hand-written producer module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JetSystematic {
    Nominal,
    JesUp,
    JesDown,
    JerUp,
    JerDown,
}

impl JetSystematic {
    pub const ALL: [Self; 5] = [
        Self::Nominal,
        Self::JesUp,
        Self::JesDown,
        Self::JerUp,
        Self::JerDown,
    ];

    pub fn all() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter()
    }
}

/// Typed per-jet inputs consumed by the JME correctionlib payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JetCorrectionInput {
    pub pt: f64,
    pub eta: f64,
}

/// A raw NanoAOD jet after applying one JME shape variation.
///
/// The correction payload is evaluated with the raw jet `pt` and `eta`.
/// The resulting scale is applied to the jet four-vector by scaling `pt` and
/// `mass` while keeping `eta` and `phi` fixed. With this parameterization the
/// Cartesian components and energy scale by the same factor, so downstream
/// cuts and observables see a real shape variation rather than a bookkeeping
/// event weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VariedJet {
    pub index: usize,
    pub raw_pt: f64,
    pub pt: f64,
    pub eta: f64,
    pub phi: f64,
    pub raw_mass: f64,
    pub mass: f64,
    pub scale: f64,
}

impl VariedJet {
    pub fn px(self) -> f64 {
        self.pt * self.phi.cos()
    }

    pub fn py(self) -> f64 {
        self.pt * self.phi.sin()
    }

    pub fn pz(self) -> f64 {
        self.pt * self.eta.sinh()
    }

    pub fn energy(self) -> f64 {
        self.mass.hypot(self.pt * self.eta.cosh())
    }
}

/// Jet selection evaluated after JME variation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VariedJetSelection {
    pub min_pt: f64,
    pub max_abs_eta: f64,
}

impl VariedJetSelection {
    pub fn new(min_pt: f64, max_abs_eta: f64) -> Self {
        Self {
            min_pt,
            max_abs_eta,
        }
    }

    pub fn accepts(self, jet: &VariedJet) -> bool {
        jet.pt > self.min_pt && jet.eta.abs() < self.max_abs_eta
    }
}

impl Default for VariedJetSelection {
    fn default() -> Self {
        Self {
            min_pt: 30.0,
            max_abs_eta: 2.4,
        }
    }
}

/// Muon signal-region event with jets and jet observables recomputed under one
/// JME shape variation.
#[derive(Clone)]
pub struct VariedMuonSignalRegion<'e> {
    ev: Ev<'e, SignalRegion>,
    systematic: JetSystematic,
    jets: Vec<VariedJet>,
    selected_jet_indices: Vec<usize>,
    normalization_weight: EventWeight,
}

impl<'e> VariedMuonSignalRegion<'e> {
    pub fn ev(&self) -> Ev<'e, SignalRegion> {
        self.ev
    }

    pub fn event(&self) -> &'e Event {
        self.ev.event()
    }

    pub fn systematic(&self) -> JetSystematic {
        self.systematic
    }

    pub fn jets(&self) -> &[VariedJet] {
        &self.jets
    }

    pub fn selected_jets(&self) -> impl Iterator<Item = &VariedJet> {
        self.selected_jet_indices
            .iter()
            .map(|&index| &self.jets[index])
    }

    pub fn selected_jet_indices(&self) -> &[usize] {
        &self.selected_jet_indices
    }

    pub fn n_selected_jets(&self) -> usize {
        self.selected_jet_indices.len()
    }

    pub fn lead_selected_jet_pt(&self) -> Option<f64> {
        self.selected_jets()
            .map(|jet| jet.pt)
            .max_by(f64::total_cmp)
    }

    pub fn normalization_weight(&self) -> EventWeight {
        self.normalization_weight
    }

    pub fn weighted(&self) -> Weighted<'e, SignalRegion, Nominal> {
        self.ev.weight(self.normalization_weight)
    }
}

/// Errors from producer-side weight construction.
#[derive(Debug)]
pub enum WeightError {
    Core(nano_core::NanoError),
    Correction(CorrectionError),
    NonFiniteFactor {
        systematic: JetSystematic,
        jet: JetCorrectionInput,
        factor: f64,
    },
    NegativeJesDown {
        uncertainty: f64,
        jet: JetCorrectionInput,
    },
}

impl fmt::Display for WeightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => write!(f, "{error}"),
            Self::Correction(error) => write!(f, "{error}"),
            Self::NonFiniteFactor {
                systematic,
                jet,
                factor,
            } => write!(
                f,
                "non-finite JME factor {factor} for {systematic:?} and jet {jet:?}"
            ),
            Self::NegativeJesDown { uncertainty, jet } => write!(
                f,
                "JES down factor would be negative for uncertainty {uncertainty} and jet {jet:?}"
            ),
        }
    }
}

impl StdError for WeightError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Core(error) => Some(error),
            Self::Correction(error) => Some(error),
            _ => None,
        }
    }
}

impl From<nano_core::NanoError> for WeightError {
    fn from(value: nano_core::NanoError) -> Self {
        Self::Core(value)
    }
}

impl From<CorrectionError> for WeightError {
    fn from(value: CorrectionError) -> Self {
        Self::Correction(value)
    }
}

/// Real JME correctionlib corrections used to vary reconstructed jets.
#[derive(Debug, Clone)]
pub struct JmeJetCorrections {
    jes_total: Correction,
    jer_scale_factor: Correction,
}

impl JmeJetCorrections {
    /// Load the real Run2 2016postVFP AK4PFPuppi JME corrections from a
    /// correctionlib JSON or JSON.GZ payload.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, WeightError> {
        Self::from_correction_set(&CorrectionSet::from_path(path)?)
    }

    /// Select the concrete JME corrections from a loaded correction set.
    pub fn from_correction_set(set: &CorrectionSet) -> Result<Self, WeightError> {
        Ok(Self {
            jes_total: set
                .correction(RUN2_2016POSTVFP_AK4PFPUPPI_JES_TOTAL)?
                .clone(),
            jer_scale_factor: set
                .correction(RUN2_2016POSTVFP_AK4PFPUPPI_JER_SCALE_FACTOR)?
                .clone(),
        })
    }

    /// Evaluate the payload's total JES uncertainty for one jet.
    pub fn jes_total_uncertainty(&self, jet: JetCorrectionInput) -> Result<f64, WeightError> {
        self.evaluate_jet_correction(&self.jes_total, jet)
    }

    /// Evaluate the payload's JER scale factor for one jet.
    pub fn jer_scale_factor(&self, jet: JetCorrectionInput) -> Result<f64, WeightError> {
        self.evaluate_jet_correction(&self.jer_scale_factor, jet)
    }

    /// Convert a typed JME systematic into a per-jet four-vector scale.
    ///
    /// Physics convention used here:
    /// - nominal leaves the raw NanoAOD jet unchanged;
    /// - JES Total up/down scales by `1 +/- total_uncertainty`;
    /// - JER up/down uses the available real payload's JER scale factor as a
    ///   deterministic smearing envelope, `SF` and `1 / SF`, around unity.
    ///
    /// This value scales the jet four-vector. It must not be multiplied into an
    /// event normalization weight.
    pub fn jet_scale(
        &self,
        systematic: JetSystematic,
        jet: JetCorrectionInput,
    ) -> Result<f64, WeightError> {
        let factor = match systematic {
            JetSystematic::Nominal => 1.0,
            JetSystematic::JesUp => 1.0 + self.jes_total_uncertainty(jet)?,
            JetSystematic::JesDown => {
                let uncertainty = self.jes_total_uncertainty(jet)?;
                if uncertainty > 1.0 {
                    return Err(WeightError::NegativeJesDown { uncertainty, jet });
                }
                1.0 - uncertainty
            }
            JetSystematic::JerUp => self.jer_scale_factor(jet)?,
            JetSystematic::JerDown => 1.0 / self.jer_scale_factor(jet)?,
        };

        if !factor.is_finite() {
            return Err(WeightError::NonFiniteFactor {
                systematic,
                jet,
                factor,
            });
        }
        Ok(factor)
    }

    /// Compatibility alias for older callers that asked for a per-jet factor.
    ///
    /// The returned value is now explicitly a four-vector scale, not a weight.
    pub fn jet_factor(
        &self,
        systematic: JetSystematic,
        jet: JetCorrectionInput,
    ) -> Result<f64, WeightError> {
        self.jet_scale(systematic, jet)
    }

    /// Build the varied jet collection for one systematic.
    ///
    /// The raw `Jet_pt`, `Jet_eta`, `Jet_phi`, and `Jet_mass` branches are read
    /// once, the correctionlib payload is evaluated per jet, and the varied
    /// four-vector is materialized as typed [`VariedJet`] values. Downstream
    /// selections should consume this collection for JME variations.
    pub fn varied_jets(
        &self,
        event: &Event,
        systematic: JetSystematic,
    ) -> Result<Vec<VariedJet>, WeightError> {
        let mut jets = Vec::new();
        for jet in event.collection("Jet")?.iter() {
            let raw_pt = f64::from(jet.pt()?);
            let eta = f64::from(jet.eta()?);
            let phi = f64::from(jet.phi()?);
            let raw_mass = f64::from(jet.mass()?);
            let input = JetCorrectionInput { pt: raw_pt, eta };
            let scale = self.jet_scale(systematic, input)?;

            jets.push(VariedJet {
                index: jet.index(),
                raw_pt,
                pt: raw_pt * scale,
                eta,
                phi,
                raw_mass,
                mass: raw_mass * scale,
                scale,
            });
        }
        Ok(jets)
    }

    /// JME corrections are shape-only in this module.
    ///
    /// Keep normalization weights separate from the jet four-vector variation
    /// path so histograms can be filled with pileup/lepton/trigger weights
    /// without accidentally treating JES/JER as scalar factors.
    pub fn normalization_weight(&self, _event: &Event) -> Result<EventWeight, WeightError> {
        Ok(EventWeight::nominal())
    }

    /// Compatibility shim for the old placeholder API.
    ///
    /// JES/JER no longer contribute to this scalar. Use [`Self::varied_jets`]
    /// and recompute selections/observables under the requested systematic.
    pub fn event_weight(
        &self,
        event: &Event,
        _systematic: JetSystematic,
    ) -> Result<EventWeight, WeightError> {
        self.normalization_weight(event)
    }

    fn evaluate_jet_correction(
        &self,
        correction: &Correction,
        jet: JetCorrectionInput,
    ) -> Result<f64, WeightError> {
        let mut values = Vec::with_capacity(correction.inputs.len());
        for variable in &correction.inputs {
            values.push(match variable.name.as_str() {
                "JetPt" | "pt" => Value::Real(jet.pt),
                "JetEta" | "eta" => Value::Real(jet.eta),
                other => {
                    return Err(CorrectionError::Unsupported(format!(
                        "JME jet correction does not know how to map input `{other}`"
                    ))
                    .into())
                }
            });
        }
        Ok(correction.evaluate(&values)?)
    }
}

/// Select the current muon signal region and recompute jets under a JME shape
/// variation.
pub fn select_muon_signal_region_with_varied_jets<'e>(
    event: Ev<'e, Raw>,
    corrections: &JmeJetCorrections,
    systematic: JetSystematic,
    jet_selection: VariedJetSelection,
) -> Result<Option<VariedMuonSignalRegion<'e>>, WeightError> {
    let Some(selected) = event.preselect(|_| true).and_then(|event| {
        event.select::<SignalRegion>(|event| passes_muon_signal_selection(event).unwrap_or(false))
    }) else {
        return Ok(None);
    };

    let jets = corrections.varied_jets(selected.event(), systematic)?;
    let selected_jet_indices = jets
        .iter()
        .enumerate()
        .filter_map(|(index, jet)| jet_selection.accepts(jet).then_some(index))
        .collect();
    let normalization_weight = corrections.normalization_weight(selected.event())?;

    Ok(Some(VariedMuonSignalRegion {
        ev: selected,
        systematic,
        jets,
        selected_jet_indices,
        normalization_weight,
    }))
}

/// Select the current muon signal region and attach only the normalization
/// event weight.
///
/// This is kept for callers that want a [`Weighted`] token for histogram
/// filling. JME shape systematics are intentionally not represented in this
/// scalar weight; use [`select_muon_signal_region_with_varied_jets`] for JES/JER
/// propagation.
pub fn select_muon_signal_region_with_weight<'e>(
    event: Ev<'e, Raw>,
    corrections: &JmeJetCorrections,
    systematic: JetSystematic,
) -> Result<Option<Weighted<'e, SignalRegion, Nominal>>, WeightError> {
    let Some(selected) = event.preselect(|_| true).and_then(|event| {
        event.select::<SignalRegion>(|event| passes_muon_signal_selection(event).unwrap_or(false))
    }) else {
        return Ok(None);
    };

    let weight = corrections.event_weight(selected.event(), systematic)?;
    Ok(Some(selected.weight(weight)))
}

/// Additive row type for outputs that want to persist the computed weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeightedMuonSkimRow {
    pub n_good_muon: u32,
    pub lead_muon_pt: f32,
    pub event_weight: f64,
}

impl WeightedMuonSkimRow {
    pub fn from_row(row: MuonSkimRow, weight: EventWeight) -> Self {
        Self {
            n_good_muon: row.n_good_muon,
            lead_muon_pt: row.lead_muon_pt,
            event_weight: weight.value(),
        }
    }
}
