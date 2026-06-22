//! Sample-table parsing and per-process histogram production.
//!
//! This layer closes the production loop between an analysis spec and the
//! multi-process datacard emitter: each sample is interpreted, MC samples are
//! scaled by `xsec*lumi/sumw`, samples sharing a process are summed, and data
//! samples are accumulated into `data_obs`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use nano_analysis::{Fb, FbInv, Hist1D, HistSet1D, Pb, PbInv};
use nano_core::Event;
use nano_spec::interpret::{
    interpret_and_fill, interpret_and_fill_systematic, InterpretedHistograms,
};
use nano_spec::ResolvedPlan;

use crate::datacard::{DatacardOutput, MultiProcessChannel, MultiProcessDatacard, Process};
use crate::{events, Result, RootError};

const NOMINAL_VARIATION: &str = "Nominal";

/// A parsed sample table with one integrated luminosity and many samples.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleTable {
    lumi: IntegratedLuminosity,
    samples: Vec<Sample>,
}

impl SampleTable {
    /// Parse and validate a TOML sample table.
    pub fn from_toml_str(input: &str) -> Result<Self> {
        let raw: RawSampleTable = toml::from_str(input)
            .map_err(|error| RootError::parse(format!("failed to parse sample TOML: {error}")))?;
        sample_table_from_raw(raw)
    }

    /// Load a TOML sample table from disk.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let input = fs::read_to_string(path)?;
        Self::from_toml_str(&input)
    }

    pub fn lumi(&self) -> IntegratedLuminosity {
        self.lumi
    }

    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    fn signal_processes(&self) -> BTreeSet<String> {
        self.samples
            .iter()
            .filter(|sample| matches!(sample.kind, SampleKind::Mc { signal: true, .. }))
            .map(|sample| sample.process.clone())
            .collect()
    }
}

/// One validated sample row.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    process: String,
    files: Vec<PathBuf>,
    kind: SampleKind,
}

impl Sample {
    pub fn process(&self) -> &str {
        &self.process
    }

    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }

    pub fn is_data(&self) -> bool {
        matches!(self.kind, SampleKind::Data)
    }

    pub fn is_signal(&self) -> bool {
        matches!(self.kind, SampleKind::Mc { signal: true, .. })
    }

    /// Per-event normalization for this sample.
    ///
    /// MC uses the same formula as `higgs4l_stack_opendata.rs`:
    /// `luminosity_pb * xsec_pb / sumw`. Data samples return `1.0`.
    pub fn normalization_factor(&self, lumi: IntegratedLuminosity) -> Result<f64> {
        match self.kind {
            SampleKind::Data => Ok(1.0),
            SampleKind::Mc { xsec, sumw, .. } => {
                mc_normalization_factor_pb(xsec.to_pb(), lumi.to_pb_inv(), sumw)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SampleKind {
    Data,
    Mc {
        signal: bool,
        xsec: CrossSection,
        sumw: f64,
    },
}

/// A cross-section parsed from the sample table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CrossSection {
    Fb(Fb),
    Pb(Pb),
}

impl CrossSection {
    pub fn to_pb(self) -> Pb {
        match self {
            Self::Fb(value) => value.to_pb(),
            Self::Pb(value) => value,
        }
    }

    pub fn to_fb(self) -> Fb {
        match self {
            Self::Fb(value) => value,
            Self::Pb(value) => value.to_fb(),
        }
    }
}

/// An integrated luminosity parsed from the sample table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IntegratedLuminosity {
    FbInv(FbInv),
    PbInv(PbInv),
}

impl IntegratedLuminosity {
    pub fn to_pb_inv(self) -> PbInv {
        match self {
            Self::FbInv(value) => value.to_pb_inv(),
            Self::PbInv(value) => value,
        }
    }

    pub fn to_fb_inv(self) -> FbInv {
        match self {
            Self::FbInv(value) => value,
            Self::PbInv(value) => value.to_fb_inv(),
        }
    }
}

/// MC normalization in pb/pb^-1 units: `xsec * lumi / sumw`.
pub fn mc_normalization_factor_pb(xsec: Pb, lumi: PbInv, sumw: f64) -> Result<f64> {
    validate_sumw(sumw, "normalization")?;
    if !xsec.0.is_finite() || xsec.0 < 0.0 {
        return Err(RootError::other(
            "normalization cross-section must be finite and non-negative",
        ));
    }
    if !lumi.0.is_finite() || lumi.0 <= 0.0 {
        return Err(RootError::other(
            "normalization luminosity must be finite and positive",
        ));
    }
    Ok((xsec * lumi) / sumw)
}

/// MC normalization in fb/fb^-1 units: `xsec * lumi / sumw`.
pub fn mc_normalization_factor_fb(xsec: Fb, lumi: FbInv, sumw: f64) -> Result<f64> {
    validate_sumw(sumw, "normalization")?;
    if !xsec.0.is_finite() || xsec.0 < 0.0 {
        return Err(RootError::other(
            "normalization cross-section must be finite and non-negative",
        ));
    }
    if !lumi.0.is_finite() || lumi.0 <= 0.0 {
        return Err(RootError::other(
            "normalization luminosity must be finite and positive",
        ));
    }
    Ok((xsec * lumi) / sumw)
}

/// Run a validated analysis plan over all ROOT files in a sample table.
pub fn run_interpreted_samples(
    table: &SampleTable,
    plan: &ResolvedPlan,
) -> Result<NormalizedProcessHistograms> {
    run_interpreted_samples_with_events(table, plan, |path| events(path, &plan.read_branches))
}

/// Run a validated analysis plan using a caller-supplied event source.
///
/// Tests can supply synthetic events while production callers use
/// [`run_interpreted_samples`] to stream ROOT files.
pub fn run_interpreted_samples_with_events<F, I>(
    table: &SampleTable,
    plan: &ResolvedPlan,
    mut events_for_file: F,
) -> Result<NormalizedProcessHistograms>
where
    F: FnMut(&Path) -> Result<I>,
    I: IntoIterator<Item = Result<Event>>,
{
    let mut output = NormalizedProcessHistograms::new(table.signal_processes());

    for (sample_index, sample) in table.samples.iter().enumerate() {
        let factor = sample.normalization_factor(table.lumi)?;
        let mut histograms = InterpretedHistograms::new(plan);
        let systematic_variations = systematic_variations(&histograms);
        let mut events_read = 0_usize;
        let mut selected = 0_usize;

        for file in &sample.files {
            for event in events_for_file(file)? {
                let event = event?;
                events_read += 1;
                if plan.spec.has_shape_correction() {
                    for systematic in &systematic_variations {
                        let row = interpret_and_fill_systematic(
                            plan,
                            &event,
                            &mut histograms,
                            systematic,
                        )
                        .map_err(|error| RootError::other(error.to_string()))?;
                        if systematic == NOMINAL_VARIATION && row.is_some() {
                            selected += 1;
                        }
                    }
                } else if interpret_and_fill(plan, &event, &mut histograms)
                    .map_err(|error| RootError::other(error.to_string()))?
                    .is_some()
                {
                    selected += 1;
                }
            }
        }

        if !sample.is_data() {
            histograms.scale(factor);
        }
        output.accumulate_sample(sample, &histograms);
        output.sample_reports.push(SampleRunReport {
            sample_index,
            process: sample.process.clone(),
            data: sample.is_data(),
            signal: sample.is_signal(),
            normalization_factor: factor,
            events_read,
            selected,
        });
    }

    Ok(output)
}

/// Per-process, normalized histogram output from a sample-table run.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedProcessHistograms {
    signal_processes: BTreeSet<String>,
    processes: BTreeMap<String, BTreeMap<String, HistSet1D<String>>>,
    data_obs: BTreeMap<String, HistSet1D<String>>,
    sample_reports: Vec<SampleRunReport>,
}

impl NormalizedProcessHistograms {
    fn new(signal_processes: BTreeSet<String>) -> Self {
        Self {
            signal_processes,
            processes: BTreeMap::new(),
            data_obs: BTreeMap::new(),
            sample_reports: Vec::new(),
        }
    }

    pub fn processes(&self) -> &BTreeMap<String, BTreeMap<String, HistSet1D<String>>> {
        &self.processes
    }

    pub fn data_obs(&self) -> &BTreeMap<String, HistSet1D<String>> {
        &self.data_obs
    }

    pub fn sample_reports(&self) -> &[SampleRunReport] {
        &self.sample_reports
    }

    /// Build a Combine datacard from the normalized nominal histograms.
    pub fn to_datacard(&self) -> Result<MultiProcessDatacard<'_>> {
        if self.signal_processes.len() != 1 {
            return Err(RootError::other(format!(
                "multi-process datacard needs exactly one signal process, found {}",
                self.signal_processes.len()
            )));
        }
        let signal_process = self
            .signal_processes
            .iter()
            .next()
            .expect("checked exactly one signal process");

        let mut datacard = MultiProcessDatacard::new();
        for (channel, data_set) in &self.data_obs {
            let data_nominal = nominal_histogram(data_set, channel)?;
            let mut combine_channel = MultiProcessChannel::new(channel, data_nominal);
            let mut background_index = 1_i32;

            for (process_name, histograms_by_channel) in &self.processes {
                let Some(set) = histograms_by_channel.get(channel) else {
                    continue;
                };
                let index = if process_name == signal_process {
                    0
                } else {
                    let index = background_index;
                    background_index += 1;
                    index
                };
                let mut process =
                    Process::new(process_name, index, nominal_histogram(set, channel)?);
                for systematic in paired_shape_systematics(set) {
                    let up = set.get(format!("{systematic}Up"));
                    let down = set.get(format!("{systematic}Down"));
                    process = process.with_shape_systematic(systematic, up, down);
                }
                combine_channel = combine_channel.with_process(process);
            }

            datacard = datacard.with_channel(combine_channel);
        }

        Ok(datacard)
    }

    /// Write `datacard.txt` and `shapes.root` for the normalized output.
    pub fn write_datacard(&self, output_dir: &Path) -> Result<DatacardOutput> {
        self.to_datacard()?.write(output_dir)
    }

    fn accumulate_sample(&mut self, sample: &Sample, histograms: &InterpretedHistograms) {
        let target = if sample.is_data() {
            &mut self.data_obs
        } else {
            self.processes.entry(sample.process.clone()).or_default()
        };
        for (name, set) in histograms.iter() {
            target
                .entry(name.clone())
                .and_modify(|existing| existing.add(set))
                .or_insert_with(|| set.clone());
        }
    }
}

/// Summary for one processed sample row.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleRunReport {
    pub sample_index: usize,
    pub process: String,
    pub data: bool,
    pub signal: bool,
    pub normalization_factor: f64,
    pub events_read: usize,
    pub selected: usize,
}

#[derive(Debug, serde::Deserialize)]
struct RawSampleTable {
    lumi: String,
    #[serde(default)]
    sample: Vec<RawSample>,
}

#[derive(Debug, serde::Deserialize)]
struct RawSample {
    process: String,
    #[serde(default)]
    signal: bool,
    #[serde(default)]
    data: bool,
    files: Vec<PathBuf>,
    xsec: Option<String>,
    sumw: Option<f64>,
}

fn sample_table_from_raw(raw: RawSampleTable) -> Result<SampleTable> {
    let lumi = parse_luminosity(&raw.lumi, "lumi")?;
    if raw.sample.is_empty() {
        return Err(RootError::other(
            "sample table must contain at least one [[sample]] row",
        ));
    }
    let samples = raw
        .sample
        .into_iter()
        .enumerate()
        .map(|(index, sample)| sample_from_raw(index, sample))
        .collect::<Result<Vec<_>>>()?;

    if !samples
        .iter()
        .any(|sample| matches!(sample.kind, SampleKind::Mc { signal: true, .. }))
    {
        return Err(RootError::other(
            "sample table must contain at least one MC sample with signal=true",
        ));
    }

    Ok(SampleTable { lumi, samples })
}

fn sample_from_raw(index: usize, raw: RawSample) -> Result<Sample> {
    validate_label("process", &raw.process)?;
    if raw.files.is_empty() {
        return Err(RootError::other(format!(
            "sample {index} process `{}` must list at least one file",
            raw.process
        )));
    }

    let kind = if raw.data || raw.xsec.is_none() {
        if raw.data && raw.xsec.is_some() {
            return Err(RootError::other(format!(
                "data sample `{}` must not set xsec",
                raw.process
            )));
        }
        if raw.signal {
            return Err(RootError::other(format!(
                "data sample `{}` cannot set signal=true",
                raw.process
            )));
        }
        if raw.sumw.is_some() {
            return Err(RootError::other(format!(
                "data sample `{}` must not set sumw",
                raw.process
            )));
        }
        SampleKind::Data
    } else {
        let xsec = parse_cross_section(raw.xsec.as_deref().expect("checked xsec"), "xsec")?;
        let sumw = raw.sumw.ok_or_else(|| {
            RootError::other(format!(
                "MC sample `{}` must set sumw with xsec",
                raw.process
            ))
        })?;
        validate_sumw(sumw, &format!("sample `{}`", raw.process))?;
        SampleKind::Mc {
            signal: raw.signal,
            xsec,
            sumw,
        }
    };

    Ok(Sample {
        process: raw.process,
        files: raw.files,
        kind,
    })
}

fn parse_cross_section(input: &str, field: &str) -> Result<CrossSection> {
    let (value, unit) = parse_quantity(input, field)?;
    if !value.is_finite() || value < 0.0 {
        return Err(RootError::other(format!(
            "{field} must be finite and non-negative"
        )));
    }
    match unit {
        "Pb" | "pb" => Ok(CrossSection::Pb(Pb(value))),
        "Fb" | "fb" => Ok(CrossSection::Fb(Fb(value))),
        "PbInv" | "pb^-1" | "pb-1" | "1/pb" | "FbInv" | "fb^-1" | "fb-1" | "1/fb" => {
            Err(RootError::other(format!(
                "{field} unit `{unit}` is not a cross-section; expected Pb or Fb"
            )))
        }
        _ => Err(RootError::other(format!(
            "{field} unit `{unit}` is unsupported; expected Pb or Fb"
        ))),
    }
}

fn parse_luminosity(input: &str, field: &str) -> Result<IntegratedLuminosity> {
    let (value, unit) = parse_quantity(input, field)?;
    if !value.is_finite() || value <= 0.0 {
        return Err(RootError::other(format!(
            "{field} must be finite and positive"
        )));
    }
    match unit {
        "FbInv" | "fb^-1" | "fb-1" | "1/fb" => Ok(IntegratedLuminosity::FbInv(FbInv(value))),
        "PbInv" | "pb^-1" | "pb-1" | "1/pb" => Ok(IntegratedLuminosity::PbInv(PbInv(value))),
        "Pb" | "pb" | "Fb" | "fb" => Err(RootError::other(format!(
            "{field} unit `{unit}` is not an integrated luminosity; expected FbInv or PbInv"
        ))),
        _ => Err(RootError::other(format!(
            "{field} unit `{unit}` is unsupported; expected FbInv or PbInv"
        ))),
    }
}

fn parse_quantity<'a>(input: &'a str, field: &str) -> Result<(f64, &'a str)> {
    let mut parts = input.split_whitespace();
    let value = parts
        .next()
        .ok_or_else(|| RootError::other(format!("{field} is missing a numeric value")))?
        .parse::<f64>()?;
    let unit = parts
        .next()
        .ok_or_else(|| RootError::other(format!("{field} is missing a unit")))?;
    if let Some(extra) = parts.next() {
        return Err(RootError::other(format!(
            "unexpected token `{extra}` in {field} quantity `{input}`"
        )));
    }
    Ok((value, unit))
}

fn validate_sumw(sumw: f64, context: &str) -> Result<()> {
    if !sumw.is_finite() || sumw <= 0.0 {
        return Err(RootError::other(format!(
            "{context} sumw must be finite and > 0"
        )));
    }
    Ok(())
}

fn validate_label(kind: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.chars().any(char::is_whitespace)
        || value.contains('/')
        || value.contains('$')
    {
        return Err(RootError::other(format!(
            "{kind} `{value}` must be non-empty and contain no whitespace, `/`, or `$`"
        )));
    }
    Ok(())
}

fn nominal_histogram<'a>(set: &'a HistSet1D<String>, context: &str) -> Result<&'a Hist1D> {
    set.iter()
        .find_map(|(systematic, hist)| (systematic == NOMINAL_VARIATION).then_some(hist))
        .ok_or_else(|| RootError::other(format!("histogram `{context}` is missing Nominal")))
}

fn paired_shape_systematics(set: &HistSet1D<String>) -> Vec<String> {
    let variations = set
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<BTreeSet<_>>();
    variations
        .iter()
        .filter_map(|name| name.strip_suffix("Up"))
        .filter(|base| variations.contains(format!("{base}Down").as_str()))
        .map(ToString::to_string)
        .collect()
}

fn systematic_variations(histograms: &InterpretedHistograms) -> Vec<String> {
    histograms
        .iter()
        .next()
        .map(|(_, set)| {
            set.iter()
                .map(|(systematic, _)| systematic.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![NOMINAL_VARIATION.to_string()])
}
