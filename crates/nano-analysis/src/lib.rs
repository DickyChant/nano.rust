//! Compile-time analysis lifecycle wrappers over the dynamic `nano_core::Event`.
//!
//! `nano-core` remains the open, runtime-typed data-access layer. This crate
//! adds a small typestate layer for analysis structure: raw events must pass
//! preselection before region selection, region-selected events must be
//! weighted before histogram filling, and region-specific fill APIs can demand
//! the exact region token they need.

use std::marker::PhantomData;

use nano_core::Event;

/// Zero-sized marker for an event before analysis preselection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Raw;

/// Zero-sized marker for an event after baseline preselection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Baseline;

/// Marker trait for selected analysis regions.
pub trait Region {
    /// Stable region name for labels, diagnostics, and generated code.
    const NAME: &'static str;
}

/// Demonstration signal-region marker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SignalRegion;

impl Region for SignalRegion {
    const NAME: &'static str = "signal";
}

/// Thin typestate wrapper over a borrowed dynamic [`Event`].
///
/// The `S` parameter is a zero-sized stage or region marker. The wrapper holds
/// only `&Event` plus `PhantomData`; it does not allocate per event.
pub struct Ev<'e, S> {
    inner: &'e Event,
    _stage: PhantomData<S>,
}

impl<'e, S> Clone for Ev<'e, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'e, S> Copy for Ev<'e, S> {}

impl<'e> Ev<'e, Raw> {
    /// Begin a typed analysis lifecycle from a dynamic event.
    pub fn new(event: &'e Event) -> Self {
        Self {
            inner: event,
            _stage: PhantomData,
        }
    }

    /// Advance `Raw -> Baseline`, or veto by returning `None`.
    pub fn preselect(self, predicate: impl Fn(&Event) -> bool) -> Option<Ev<'e, Baseline>> {
        predicate(self.inner).then_some(Ev {
            inner: self.inner,
            _stage: PhantomData,
        })
    }
}

impl<'e> Ev<'e, Baseline> {
    /// Advance `Baseline -> R`, or veto by returning `None`.
    pub fn select<R: Region>(self, predicate: impl Fn(&Event) -> bool) -> Option<Ev<'e, R>> {
        predicate(self.inner).then_some(Ev {
            inner: self.inner,
            _stage: PhantomData,
        })
    }
}

impl<'e, S> Ev<'e, S> {
    /// Access the underlying dynamic event for branch reads and object access.
    pub fn event(&self) -> &'e Event {
        self.inner
    }
}

impl<'e, R: Region> Ev<'e, R> {
    /// Attach an accumulated event weight, producing the token required by
    /// histogram filling.
    pub fn weight(self, weight: EventWeight) -> Weighted<'e, R> {
        Weighted { ev: self, weight }
    }
}

/// Accumulated multiplicative event weight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EventWeight {
    value: f64,
}

impl EventWeight {
    /// Unit nominal event weight.
    pub fn nominal() -> Self {
        Self { value: 1.0 }
    }

    /// Multiply by one additional correction factor.
    pub fn times(mut self, factor: f64) -> Self {
        self.value *= factor;
        self
    }

    /// Numeric value of the accumulated weight.
    pub fn value(self) -> f64 {
        self.value
    }
}

/// Region-selected event after weights have been applied.
pub struct Weighted<'e, R: Region> {
    ev: Ev<'e, R>,
    weight: EventWeight,
}

impl<'e, R: Region> Clone for Weighted<'e, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'e, R: Region> Copy for Weighted<'e, R> {}

impl<'e, R: Region> Weighted<'e, R> {
    /// Region-selected event token carried by this weighted event.
    pub fn ev(&self) -> Ev<'e, R> {
        self.ev
    }

    /// Underlying dynamic event.
    pub fn event(&self) -> &'e Event {
        self.ev.event()
    }

    /// Accumulated event weight.
    pub fn weight(&self) -> EventWeight {
        self.weight
    }

    /// Region name associated with this weighted token.
    pub fn region_name(&self) -> &'static str {
        R::NAME
    }
}

/// Minimal fixed-width one-dimensional histogram.
#[derive(Debug, Clone, PartialEq)]
pub struct Hist1D {
    low: f64,
    high: f64,
    bins: Vec<f64>,
    underflow: f64,
    overflow: f64,
}

impl Hist1D {
    /// Create a fixed-bin histogram over `[low, high)`.
    ///
    /// Panics if `bins == 0`, bounds are not finite, or `high <= low`.
    pub fn new(bins: usize, low: f64, high: f64) -> Self {
        assert!(bins > 0, "histogram must have at least one bin");
        assert!(
            low.is_finite() && high.is_finite() && high > low,
            "histogram bounds must be finite and ordered"
        );

        Self {
            low,
            high,
            bins: vec![0.0; bins],
            underflow: 0.0,
            overflow: 0.0,
        }
    }

    /// Bin contents, excluding underflow and overflow.
    pub fn bins(&self) -> &[f64] {
        &self.bins
    }

    pub fn underflow(&self) -> f64 {
        self.underflow
    }

    pub fn overflow(&self) -> f64 {
        self.overflow
    }

    pub fn sumw(&self) -> f64 {
        self.underflow + self.overflow + self.bins.iter().sum::<f64>()
    }

    fn fill_weighted(&mut self, value: f64, weight: f64) {
        if value < self.low {
            self.underflow += weight;
        } else if value >= self.high {
            self.overflow += weight;
        } else {
            let width = self.high - self.low;
            let bin = ((value - self.low) / width * self.bins.len() as f64) as usize;
            self.bins[bin] += weight;
        }
    }
}

/// Fill a histogram with a weighted event from region `R`.
///
/// The type signature is the precondition: callers cannot pass a raw,
/// baseline, selected-but-unweighted, or differently-regioned event.
///
/// ```compile_fail
/// use nano_analysis::{fill, Ev, Hist1D, SignalRegion};
/// use nano_core::{BranchSchema, BranchSpec, Event};
///
/// let schema = BranchSchema::new(Vec::<BranchSpec>::new()).unwrap();
/// let event = Event::from_columns(
///     schema,
///     Vec::<(String, nano_core::BranchColumn)>::new(),
///     0,
/// )
/// .unwrap();
/// let raw = Ev::new(&event);
/// let mut hist = Hist1D::new(1, 0.0, 1.0);
/// fill::<SignalRegion>(&mut hist, &raw, 0.5);
/// ```
///
/// ```compile_fail
/// use nano_analysis::{fill, Ev, EventWeight, Hist1D, Region, SignalRegion};
/// use nano_core::{BranchSchema, BranchSpec, Event};
///
/// struct ControlRegion;
/// impl Region for ControlRegion {
///     const NAME: &'static str = "control";
/// }
///
/// let schema = BranchSchema::new(Vec::<BranchSpec>::new()).unwrap();
/// let event = Event::from_columns(
///     schema,
///     Vec::<(String, nano_core::BranchColumn)>::new(),
///     0,
/// )
/// .unwrap();
/// let control = Ev::new(&event)
///     .preselect(|_| true)
///     .unwrap()
///     .select::<ControlRegion>(|_| true)
///     .unwrap()
///     .weight(EventWeight::nominal());
/// let mut hist = Hist1D::new(1, 0.0, 1.0);
/// fill::<SignalRegion>(&mut hist, &control, 0.5);
/// ```
pub fn fill<R: Region>(hist: &mut Hist1D, event: &Weighted<R>, value: f64) {
    hist.fill_weighted(value, event.weight.value());
}

/// Energy unit newtype.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct GeV(pub f64);

/// Integrated luminosity unit newtype.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Fb(pub f64);

/// Cross-section unit newtype.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Pb(pub f64);

/// Exhaustive list of systematic variations handled by this first slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Systematic {
    Nominal,
    JesUp,
    JesDown,
    JerUp,
    JerDown,
}

impl Systematic {
    pub const ALL: [Self; 5] = [
        Self::Nominal,
        Self::JesUp,
        Self::JesDown,
        Self::JerUp,
        Self::JerDown,
    ];

    /// Iterate over every systematic variation.
    pub fn all() -> impl Iterator<Item = Self> {
        Self::ALL.into_iter()
    }
}

/// Whether the dynamic event passes the existing muon producer's signal cut.
///
/// The cut matches `nano_producers::MuonProducer`: at least one muon with
/// `pt > 30` and `abs(eta) < 2.4`.
pub fn passes_muon_signal_selection(event: &Event) -> nano_core::Result<bool> {
    let muons = event.collection("Muon")?;
    for muon in muons.iter() {
        let pt = muon.pt()?;
        let eta = muon.eta()?;
        if pt > 30.0 && eta.abs() < 2.4 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Typed re-expression of the current muon selection.
///
/// There is no separate baseline cut in the existing muon producer, so this
/// demonstration uses an identity preselection before the signal-region cut.
/// Dynamic branch-read failures are treated as vetoes because the requested
/// demonstration returns `Option`.
pub fn select_muon_signal_region(event: Ev<'_, Raw>) -> Option<Weighted<'_, SignalRegion>> {
    Some(
        event
            .preselect(|_| true)?
            .select::<SignalRegion>(|event| passes_muon_signal_selection(event).unwrap_or(false))?
            .weight(EventWeight::nominal()),
    )
}
