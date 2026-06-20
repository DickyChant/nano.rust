//! Channel-side correction plumbing for typed event weights.
//!
//! This module deliberately keeps physics combination policy local and readable:
//! each selected jet contributes one multiplicative factor to the accumulated
//! [`nano_analysis::EventWeight`]. Later pileup, lepton scale factors, and
//! trigger weights can slot in by adding typed input wrappers next to
//! [`JetCorrectionInput`] and multiplying their factors in the same place.

use nano_analysis::{
    passes_muon_signal_selection, Ev, EventWeight, Raw, SignalRegion, Systematic, Weighted,
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

/// Typed per-jet inputs consumed by the JME correctionlib payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JetCorrectionInput {
    pub pt: f64,
    pub eta: f64,
}

/// Errors from producer-side weight construction.
#[derive(Debug)]
pub enum WeightError {
    Core(nano_core::NanoError),
    Correction(CorrectionError),
    NonFiniteFactor {
        systematic: Systematic,
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

/// Real JME correctionlib corrections used to build event weights.
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

    /// Convert a typed systematic into a per-jet multiplicative factor.
    pub fn jet_factor(
        &self,
        systematic: Systematic,
        jet: JetCorrectionInput,
    ) -> Result<f64, WeightError> {
        let factor = match systematic {
            Systematic::Nominal => 1.0,
            Systematic::JesUp => 1.0 + self.jes_total_uncertainty(jet)?,
            Systematic::JesDown => {
                let uncertainty = self.jes_total_uncertainty(jet)?;
                if uncertainty > 1.0 {
                    return Err(WeightError::NegativeJesDown { uncertainty, jet });
                }
                1.0 - uncertainty
            }
            Systematic::JerUp => self.jer_scale_factor(jet)?,
            Systematic::JerDown => 1.0 / self.jer_scale_factor(jet)?,
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

    /// Accumulate the selected jet factors into an event-level typed weight.
    ///
    /// This first Phase-2 slice treats JME shape variations as multiplicative
    /// bookkeeping weights: for each jet, JES up/down use `1 +/- Total`, while
    /// JER up/down use the real JER scale factor and its inverse around unity.
    /// The loop is intentionally explicit so reviewers can replace this policy
    /// with full four-vector propagation later without changing the typestate
    /// interface.
    pub fn event_weight(
        &self,
        event: &Event,
        systematic: Systematic,
    ) -> Result<EventWeight, WeightError> {
        let mut weight = EventWeight::nominal();
        for jet in event.collection("Jet")?.iter() {
            let jet_input = JetCorrectionInput {
                pt: f64::from(jet.pt()?),
                eta: f64::from(jet.eta()?),
            };
            weight = weight.times(self.jet_factor(systematic, jet_input)?);
        }
        Ok(weight)
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

/// Select the current muon signal region and attach the systematic event weight.
pub fn select_muon_signal_region_with_weight<'e>(
    event: Ev<'e, Raw>,
    corrections: &JmeJetCorrections,
    systematic: Systematic,
) -> Result<Option<Weighted<'e, SignalRegion>>, WeightError> {
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
