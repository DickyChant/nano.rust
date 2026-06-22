//! CMS Combine datacard emission for already-filled one-dimensional histograms.
//!
//! This first slice intentionally supports one physics process per channel.
//! The API keeps channels and process metadata separate so a later multi-process
//! extension can add process columns without changing the output contract here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use nano_analysis::Hist1D;

use crate::{writer, Result, RootError};

const DATACARD_FILE: &str = "datacard.txt";
const SHAPES_FILE: &str = "shapes.root";

/// Output paths written by [`SingleProcessDatacard::write`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatacardOutput {
    pub datacard_path: PathBuf,
    pub shapes_path: PathBuf,
}

/// A flat normalization-only nuisance emitted as a Combine `lnN` row.
#[derive(Debug, Clone, PartialEq)]
pub struct FlatWeightSystematic {
    pub name: String,
    pub up: f64,
    pub down: f64,
}

impl FlatWeightSystematic {
    /// Create an asymmetric flat weight systematic.
    pub fn new(name: impl Into<String>, up: f64, down: f64) -> Self {
        Self {
            name: name.into(),
            up,
            down,
        }
    }
}

/// A two-sided per-bin shape variation for one channel and process.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeVariation<'a> {
    pub up: &'a Hist1D,
    pub down: &'a Hist1D,
}

impl<'a> ShapeVariation<'a> {
    pub fn new(up: &'a Hist1D, down: &'a Hist1D) -> Self {
        Self { up, down }
    }
}

/// One Combine channel/bin for a single process plus observed data.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel<'a> {
    name: String,
    nominal: &'a Hist1D,
    data_obs: &'a Hist1D,
    shape_variations: BTreeMap<String, ShapeVariation<'a>>,
}

impl<'a> Channel<'a> {
    /// Create a channel from the nominal process shape and observed data shape.
    pub fn new(name: impl Into<String>, nominal: &'a Hist1D, data_obs: &'a Hist1D) -> Self {
        Self {
            name: name.into(),
            nominal,
            data_obs,
            shape_variations: BTreeMap::new(),
        }
    }

    /// Attach one shape/JES-style systematic with per-bin up/down histograms.
    pub fn with_shape_systematic(
        mut self,
        name: impl Into<String>,
        up: &'a Hist1D,
        down: &'a Hist1D,
    ) -> Self {
        self.shape_variations
            .insert(name.into(), ShapeVariation::new(up, down));
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn nominal(&self) -> &Hist1D {
        self.nominal
    }

    pub fn data_obs(&self) -> &Hist1D {
        self.data_obs
    }

    pub fn shape_variations(&self) -> &BTreeMap<String, ShapeVariation<'a>> {
        &self.shape_variations
    }
}

/// A Combine datacard with one process column per channel.
#[derive(Debug, Clone, PartialEq)]
pub struct SingleProcessDatacard<'a> {
    process: String,
    process_index: i32,
    channels: Vec<Channel<'a>>,
    flat_weight_systematics: Vec<FlatWeightSystematic>,
}

impl<'a> SingleProcessDatacard<'a> {
    /// Create a single-process datacard. The default process index is `0`.
    pub fn new(process: impl Into<String>) -> Self {
        Self {
            process: process.into(),
            process_index: 0,
            channels: Vec::new(),
            flat_weight_systematics: Vec::new(),
        }
    }

    /// Set the Combine process index used in the numeric `process` row.
    pub fn with_process_index(mut self, process_index: i32) -> Self {
        self.process_index = process_index;
        self
    }

    /// Add one channel/bin. In this slice each channel has exactly one process.
    pub fn with_channel(mut self, channel: Channel<'a>) -> Self {
        self.channels.push(channel);
        self
    }

    /// Add a flat normalization-only weight systematic emitted as `lnN`.
    pub fn with_flat_weight_systematic(mut self, systematic: FlatWeightSystematic) -> Self {
        self.flat_weight_systematics.push(systematic);
        self
    }

    pub fn process(&self) -> &str {
        &self.process
    }

    pub fn channels(&self) -> &[Channel<'a>] {
        &self.channels
    }

    pub fn flat_weight_systematics(&self) -> &[FlatWeightSystematic] {
        &self.flat_weight_systematics
    }

    /// Write `datacard.txt` and `shapes.root` into `output_dir`.
    pub fn write(&self, output_dir: &Path) -> Result<DatacardOutput> {
        self.validate()?;
        fs::create_dir_all(output_dir)?;
        let datacard_path = output_dir.join(DATACARD_FILE);
        let shapes_path = output_dir.join(SHAPES_FILE);

        let shape_inputs = self.shape_inputs();
        let borrowed = shape_inputs
            .iter()
            .map(|(name, hist)| (name.as_str(), *hist))
            .collect::<Vec<_>>();
        writer::write_histograms(&shapes_path, &borrowed)?;

        let text = self.to_text(SHAPES_FILE)?;
        fs::write(&datacard_path, text)?;

        Ok(DatacardOutput {
            datacard_path,
            shapes_path,
        })
    }

    /// Render the Combine text datacard using `shapes_file` in the `shapes` line.
    pub fn to_text(&self, shapes_file: &str) -> Result<String> {
        self.validate()?;
        if shapes_file.trim().is_empty() || shapes_file.chars().any(char::is_whitespace) {
            return Err(RootError::other(
                "Combine shapes file name must be non-empty and contain no whitespace",
            ));
        }

        let shape_systematics = self.shape_systematic_names();
        let columns = self.channels.len();
        let mut out = String::new();

        writeln!(out, "imax {} number of channels", self.channels.len())?;
        writeln!(out, "jmax 0 number of processes minus 1")?;
        writeln!(
            out,
            "kmax {} number of nuisance parameters",
            shape_systematics.len() + self.flat_weight_systematics.len()
        )?;
        writeln!(out, "------------")?;
        writeln!(
            out,
            "shapes * * {shapes_file} $CHANNEL/$PROCESS $CHANNEL/$PROCESS_$SYSTEMATIC"
        )?;
        writeln!(out, "------------")?;
        writeln!(
            out,
            "bin {}",
            join(self.channels.iter().map(|channel| channel.name()))
        )?;
        writeln!(
            out,
            "observation {}",
            join(
                self.channels
                    .iter()
                    .map(|channel| format_rate(rate(channel.data_obs())))
            )
        )?;
        writeln!(out, "------------")?;
        writeln!(
            out,
            "bin {}",
            join(self.channels.iter().map(|channel| channel.name()))
        )?;
        writeln!(out, "process {}", repeated(&self.process, columns))?;
        writeln!(
            out,
            "process {}",
            repeated(&self.process_index.to_string(), columns)
        )?;
        writeln!(
            out,
            "rate {}",
            join(
                self.channels
                    .iter()
                    .map(|channel| format_rate(rate(channel.nominal())))
            )
        )?;
        writeln!(out, "------------")?;

        for systematic in shape_systematics {
            writeln!(
                out,
                "{systematic} shape {}",
                join(self.channels.iter().map(|channel| {
                    if channel.shape_variations.contains_key(&systematic) {
                        "1"
                    } else {
                        "-"
                    }
                }))
            )?;
        }

        for systematic in &self.flat_weight_systematics {
            writeln!(
                out,
                "{} lnN {}",
                systematic.name,
                repeated(&format_lnn(systematic), columns)
            )?;
        }

        Ok(out)
    }

    fn validate(&self) -> Result<()> {
        validate_label("process", &self.process)?;
        if self.channels.is_empty() {
            return Err(RootError::other(
                "Combine datacard needs at least one channel",
            ));
        }

        let mut channel_names = BTreeSet::new();
        for channel in &self.channels {
            validate_label("channel", &channel.name)?;
            if !channel_names.insert(channel.name.as_str()) {
                return Err(RootError::other(format!(
                    "duplicate Combine channel `{}`",
                    channel.name
                )));
            }
            validate_compatible_histograms(channel.nominal, channel.data_obs, &channel.name)?;
            for (name, variation) in &channel.shape_variations {
                validate_label("shape systematic", name)?;
                validate_compatible_histograms(channel.nominal, variation.up, name)?;
                validate_compatible_histograms(channel.nominal, variation.down, name)?;
            }
        }

        let mut flat_names = BTreeSet::new();
        for systematic in &self.flat_weight_systematics {
            validate_label("flat weight systematic", &systematic.name)?;
            if !flat_names.insert(systematic.name.as_str()) {
                return Err(RootError::other(format!(
                    "duplicate flat weight systematic `{}`",
                    systematic.name
                )));
            }
            if !(systematic.up.is_finite() && systematic.down.is_finite()) {
                return Err(RootError::other(format!(
                    "flat weight systematic `{}` has non-finite up/down factor",
                    systematic.name
                )));
            }
            if systematic.up <= 0.0 || systematic.down <= 0.0 {
                return Err(RootError::other(format!(
                    "flat weight systematic `{}` must have positive up/down factors",
                    systematic.name
                )));
            }
        }

        for shape in self.shape_systematic_names() {
            if flat_names.contains(shape.as_str()) {
                return Err(RootError::other(format!(
                    "systematic `{shape}` is both shape and lnN"
                )));
            }
        }

        Ok(())
    }

    fn shape_systematic_names(&self) -> Vec<String> {
        self.channels
            .iter()
            .flat_map(|channel| channel.shape_variations.keys().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn shape_inputs(&self) -> Vec<(String, &'a Hist1D)> {
        let mut histograms = Vec::new();
        for channel in &self.channels {
            histograms.push((shape_name(&channel.name, &self.process), channel.nominal));
            histograms.push((shape_name(&channel.name, "data_obs"), channel.data_obs));
            for (systematic, variation) in &channel.shape_variations {
                histograms.push((
                    shape_name(&channel.name, &format!("{}_{systematic}Up", self.process)),
                    variation.up,
                ));
                histograms.push((
                    shape_name(&channel.name, &format!("{}_{systematic}Down", self.process)),
                    variation.down,
                ));
            }
        }
        histograms
    }
}

fn validate_label(kind: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.chars().any(char::is_whitespace)
        || value.contains('/')
        || value.contains('$')
    {
        return Err(RootError::other(format!(
            "Combine {kind} `{value}` must be non-empty and contain no whitespace, `/`, or `$`"
        )));
    }
    Ok(())
}

fn validate_compatible_histograms(reference: &Hist1D, other: &Hist1D, context: &str) -> Result<()> {
    if reference.nbins() != other.nbins()
        || reference.low() != other.low()
        || reference.high() != other.high()
    {
        return Err(RootError::other(format!(
            "histogram `{context}` has binning incompatible with the channel nominal histogram"
        )));
    }
    Ok(())
}

fn rate(hist: &Hist1D) -> f64 {
    hist.bins().iter().sum()
}

fn shape_name(channel: &str, process: &str) -> String {
    format!("{channel}/{process}")
}

fn format_lnn(systematic: &FlatWeightSystematic) -> String {
    format!(
        "{}/{}",
        format_rate(systematic.down),
        format_rate(systematic.up)
    )
}

fn format_rate(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let formatted = format!("{value:.12}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn repeated(value: &str, count: usize) -> String {
    join(std::iter::repeat_n(value, count))
}

fn join<T: AsRef<str>>(parts: impl IntoIterator<Item = T>) -> String {
    parts
        .into_iter()
        .map(|part| part.as_ref().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
