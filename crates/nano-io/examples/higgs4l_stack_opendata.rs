#[cfg(feature = "http")]
const DEFAULT_BASE_URL: &str =
    "https://eospublic.cern.ch//eos/root-eos/cms_opendata_2012_nanoaod_skimmed/";
#[cfg(feature = "http")]
const NBINS: usize = 36;
#[cfg(feature = "http")]
const MASS_MIN: f64 = 70.0;
#[cfg(feature = "http")]
const MASS_MAX: f64 = 180.0;
#[cfg(feature = "http")]
const LUMINOSITY_PB: f64 = 11580.0;
#[cfg(feature = "http")]
const SCALE_ZZ_TO_4L: f64 = 1.386;

use std::error::Error;

#[cfg(feature = "http")]
#[allow(dead_code)]
#[path = "higgs4l_opendata.rs"]
mod higgs4l_opendata;

#[cfg(feature = "http")]
fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse()?;
    ensure_plot_feature(&options.plot)?;
    if options.insecure {
        std::env::set_var("NANO_HTTP_INSECURE", "1");
    }

    let report = analyze_stack(&options.sources(), options.events)?;

    println!("luminosity_pb: {LUMINOSITY_PB}");
    println!("histogram: nbins={NBINS} range=[{MASS_MIN},{MASS_MAX}]");
    println!("samples:");
    for sample in samples() {
        println!(
            "  {},kind={},weight={:.12},channels={}",
            sample.file,
            sample.kind.label(),
            sample.weight,
            sample
                .channels
                .iter()
                .map(|channel| channel.label())
                .collect::<Vec<_>>()
                .join("+")
        );
    }
    println!("processed_samples:");
    for sample in &report.samples {
        println!(
            "  {},events_read={},bytes_fetched={},file_size={},selected={}",
            sample.file,
            sample.events_read,
            sample.bytes_fetched,
            sample.file_size,
            sample.selected
        );
    }
    print_yields(&report.histograms);

    if let Some(path) = options.plot.as_deref() {
        write_stack_plot(path, &report.histograms)?;
        println!("stack_plot: {path}");
    }

    Ok(())
}

#[cfg(not(feature = "http"))]
fn main() -> Result<(), Box<dyn Error>> {
    Err("higgs4l_stack_opendata requires the nano-io `http` feature".into())
}

#[cfg(feature = "http")]
#[derive(Debug, Clone)]
struct Options {
    base_url: String,
    local_dir: Option<String>,
    events: Option<usize>,
    insecure: bool,
    plot: Option<String>,
}

#[cfg(feature = "http")]
impl Options {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut base_url = DEFAULT_BASE_URL.to_string();
        let mut local_dir = None;
        let mut events = None;
        let mut insecure = true;
        let mut plot = None;

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--base-url" => base_url = args.next().ok_or("missing value after --base-url")?,
                "--local-dir" => {
                    local_dir = Some(args.next().ok_or("missing value after --local-dir")?)
                }
                "--events" | "-n" => {
                    events = Some(
                        args.next()
                            .ok_or("missing value after --events")?
                            .parse::<usize>()
                            .map_err(|err| format!("invalid event count: {err}"))?,
                    );
                }
                "--insecure" => insecure = true,
                "--secure" => insecure = false,
                "--plot" => plot = Some(args.next().ok_or("missing value after --plot")?),
                "-h" | "--help" => {
                    return Err(format!(
                        "usage: higgs4l_stack_opendata [--base-url url | --local-dir dir] [--events n] [--secure|--insecure] [--plot path]\ndefault base URL: {DEFAULT_BASE_URL}"
                    )
                    .into());
                }
                other => return Err(format!("unknown argument: {other}").into()),
            }
        }

        Ok(Self {
            base_url,
            local_dir,
            events,
            insecure,
            plot,
        })
    }

    fn sources(&self) -> SourceConfig {
        SourceConfig {
            base_url: self.base_url.clone(),
            local_dir: self.local_dir.clone(),
        }
    }
}

#[cfg(feature = "http")]
#[derive(Debug, Clone)]
pub struct SourceConfig {
    pub base_url: String,
    pub local_dir: Option<String>,
}

#[cfg(feature = "http")]
impl Default for SourceConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            local_dir: None,
        }
    }
}

#[cfg(feature = "http")]
impl SourceConfig {
    fn source_for(&self, file: &str) -> String {
        if let Some(dir) = self.local_dir.as_deref() {
            std::path::Path::new(dir)
                .join(file)
                .to_string_lossy()
                .into_owned()
        } else {
            format!("{}/{}", self.base_url.trim_end_matches('/'), file)
        }
    }
}

#[cfg(all(feature = "http", not(feature = "plot")))]
fn ensure_plot_feature(plot: &Option<String>) -> Result<(), Box<dyn Error>> {
    if plot.is_some() {
        Err("--plot requires plotting support; rebuild with --features plot".into())
    } else {
        Ok(())
    }
}

#[cfg(all(feature = "http", feature = "plot"))]
fn ensure_plot_feature(_plot: &Option<String>) -> Result<(), Box<dyn Error>> {
    Ok(())
}

#[cfg(feature = "http")]
#[derive(Debug, Clone)]
pub struct StackReport {
    pub histograms: StackHistograms,
    pub samples: Vec<SampleReport>,
}

#[cfg(feature = "http")]
#[derive(Debug, Clone)]
pub struct SampleReport {
    pub file: &'static str,
    pub events_read: usize,
    pub selected: usize,
    pub bytes_fetched: u64,
    pub file_size: u64,
}

#[cfg(feature = "http")]
#[derive(Debug, Clone)]
pub struct StackHistograms {
    pub edges: Vec<f64>,
    pub signal: Vec<f64>,
    pub background: Vec<f64>,
    pub data: Vec<f64>,
}

#[cfg(feature = "http")]
impl StackHistograms {
    pub fn total_signal(&self) -> f64 {
        self.signal.iter().sum()
    }

    pub fn total_background(&self) -> f64 {
        self.background.iter().sum()
    }

    pub fn total_data(&self) -> f64 {
        self.data.iter().sum()
    }

    pub fn total_mc(&self) -> Vec<f64> {
        self.background
            .iter()
            .zip(&self.signal)
            .map(|(background, signal)| background + signal)
            .collect()
    }

    pub fn signal_peak_bin(&self) -> Option<usize> {
        self.signal
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.total_cmp(right))
            .map(|(index, _)| index)
    }
}

#[cfg(feature = "http")]
pub fn analyze_stack(
    sources: &SourceConfig,
    limit: Option<usize>,
) -> Result<StackReport, Box<dyn Error>> {
    let mut histograms = StackHistograms {
        edges: histogram_edges(),
        signal: vec![0.0; NBINS],
        background: vec![0.0; NBINS],
        data: vec![0.0; NBINS],
    };
    let mut sample_reports = Vec::new();

    for sample in samples() {
        let source = sources.source_for(sample.file);
        let report = higgs4l_opendata::analyze_source(&source, limit)?;
        let mut selected = 0;

        for candidate in &report.selected {
            let Some(channel) = Channel::from_label(candidate.channel) else {
                continue;
            };
            if !sample.channels.contains(&channel) {
                continue;
            }
            selected += 1;
            let mass = f64::from(candidate.h_mass);
            match sample.kind {
                SampleKind::Signal => fill_weighted(&mut histograms.signal, mass, sample.weight),
                SampleKind::Background => {
                    fill_weighted(&mut histograms.background, mass, sample.weight)
                }
                SampleKind::Data => fill_weighted(&mut histograms.data, mass, sample.weight),
            }
        }

        sample_reports.push(SampleReport {
            file: sample.file,
            events_read: report.events_read,
            selected,
            bytes_fetched: report.bytes_fetched,
            file_size: report.file_size,
        });
    }

    Ok(StackReport {
        histograms,
        samples: sample_reports,
    })
}

#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy)]
struct Sample {
    file: &'static str,
    kind: SampleKind,
    channels: &'static [Channel],
    weight: f64,
}

#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleKind {
    Signal,
    Background,
    Data,
}

#[cfg(feature = "http")]
impl SampleKind {
    fn label(self) -> &'static str {
        match self {
            Self::Signal => "signal",
            Self::Background => "background",
            Self::Data => "data",
        }
    }
}

#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Channel {
    FourMu,
    FourEl,
    TwoElTwoMu,
}

#[cfg(feature = "http")]
impl Channel {
    fn label(self) -> &'static str {
        match self {
            Self::FourMu => "4mu",
            Self::FourEl => "4e",
            Self::TwoElTwoMu => "2e2mu",
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "4mu" => Some(Self::FourMu),
            "4e" => Some(Self::FourEl),
            "2e2mu" => Some(Self::TwoElTwoMu),
            _ => None,
        }
    }
}

#[cfg(feature = "http")]
const ALL_CHANNELS: &[Channel] = &[Channel::FourMu, Channel::FourEl, Channel::TwoElTwoMu];
#[cfg(feature = "http")]
const FOUR_MU: &[Channel] = &[Channel::FourMu];
#[cfg(feature = "http")]
const FOUR_EL: &[Channel] = &[Channel::FourEl];
#[cfg(feature = "http")]
const TWO_EL_TWO_MU: &[Channel] = &[Channel::TwoElTwoMu];
#[cfg(feature = "http")]
const DOUBLE_MU_DATA_CHANNELS: &[Channel] = &[Channel::FourMu, Channel::TwoElTwoMu];

#[cfg(feature = "http")]
fn samples() -> Vec<Sample> {
    let weight_signal = LUMINOSITY_PB * 0.0065 / 299_973.0;
    vec![
        Sample {
            file: "SMHiggsToZZTo4L.root",
            kind: SampleKind::Signal,
            channels: ALL_CHANNELS,
            weight: weight_signal,
        },
        Sample {
            file: "ZZTo4mu.root",
            kind: SampleKind::Background,
            channels: FOUR_MU,
            weight: LUMINOSITY_PB * 0.077 * SCALE_ZZ_TO_4L / 1_499_064.0,
        },
        Sample {
            file: "ZZTo4e.root",
            kind: SampleKind::Background,
            channels: FOUR_EL,
            weight: LUMINOSITY_PB * 0.077 * SCALE_ZZ_TO_4L / 1_499_093.0,
        },
        Sample {
            file: "ZZTo2e2mu.root",
            kind: SampleKind::Background,
            channels: TWO_EL_TWO_MU,
            weight: LUMINOSITY_PB * 0.18 * SCALE_ZZ_TO_4L / 1_497_445.0,
        },
        Sample {
            file: "Run2012B_DoubleMuParked.root",
            kind: SampleKind::Data,
            channels: DOUBLE_MU_DATA_CHANNELS,
            weight: 1.0,
        },
        Sample {
            file: "Run2012C_DoubleMuParked.root",
            kind: SampleKind::Data,
            channels: DOUBLE_MU_DATA_CHANNELS,
            weight: 1.0,
        },
        Sample {
            file: "Run2012B_DoubleElectron.root",
            kind: SampleKind::Data,
            channels: FOUR_EL,
            weight: 1.0,
        },
        Sample {
            file: "Run2012C_DoubleElectron.root",
            kind: SampleKind::Data,
            channels: FOUR_EL,
            weight: 1.0,
        },
    ]
}

#[cfg(feature = "http")]
fn histogram_edges() -> Vec<f64> {
    let width = (MASS_MAX - MASS_MIN) / NBINS as f64;
    (0..=NBINS)
        .map(|index| MASS_MIN + index as f64 * width)
        .collect()
}

#[cfg(feature = "http")]
fn fill_weighted(histogram: &mut [f64], value: f64, weight: f64) {
    if !(MASS_MIN..MASS_MAX).contains(&value) {
        return;
    }
    let bin = ((value - MASS_MIN) / (MASS_MAX - MASS_MIN) * NBINS as f64) as usize;
    if let Some(slot) = histogram.get_mut(bin) {
        *slot += weight;
    }
}

#[cfg(feature = "http")]
fn print_yields(histograms: &StackHistograms) {
    println!("bin,low,high,signal,background,data");
    for index in 0..NBINS {
        println!(
            "{},{:.9},{:.9},{:.12},{:.12},{:.0}",
            index + 1,
            histograms.edges[index],
            histograms.edges[index + 1],
            histograms.signal[index],
            histograms.background[index],
            histograms.data[index]
        );
    }
    println!(
        "totals,signal={:.12},background={:.12},data={:.0},mc={:.12}",
        histograms.total_signal(),
        histograms.total_background(),
        histograms.total_data(),
        histograms.total_signal() + histograms.total_background()
    );
}

#[cfg(all(feature = "http", feature = "plot"))]
fn write_stack_plot(path: &str, histograms: &StackHistograms) -> Result<(), Box<dyn Error>> {
    use kuva::backend::svg::SvgBackend;
    use kuva::plot::scatter::ScatterPlot;
    use kuva::plot::Histogram;
    use kuva::render::layout::Layout;
    use kuva::render::plots::Plot;
    use kuva::render::render::render_multiple;
    #[cfg(feature = "plot-png")]
    use kuva::PngBackend;

    let centers: Vec<f64> = histograms
        .edges
        .windows(2)
        .map(|edge| 0.5 * (edge[0] + edge[1]))
        .collect();
    let half_width = 0.5 * (histograms.edges[1] - histograms.edges[0]);
    let data_yerr = histograms.data.iter().map(|count| count.sqrt());

    let total_mc = histograms.total_mc();
    let plots = vec![
        Plot::Histogram(
            Histogram::from_bins(histograms.edges.clone(), total_mc)
                .with_color("#c43c3926")
                .with_legend("m_H = 125 GeV + ZZ"),
        ),
        Plot::Histogram(
            Histogram::from_bins(histograms.edges.clone(), histograms.background.clone())
                .with_color("#7db9d6")
                .with_legend("ZZ"),
        ),
        Plot::Scatter(
            ScatterPlot::new()
                .with_data(centers.into_iter().zip(histograms.data.iter().copied()))
                .with_x_err(std::iter::repeat(half_width).take(NBINS))
                .with_y_err(data_yerr)
                .with_color("black")
                .with_size(4.0)
                .with_legend("Data"),
        ),
    ];
    let layout = Layout::auto_from_plots(&plots)
        .with_title("CMS Open Data, sqrt(s)=8 TeV, L=11.6 fb^-1")
        .with_x_label("m(4l) [GeV]")
        .with_y_label("N_Events")
        .with_width(900.0)
        .with_height(700.0)
        .with_x_axis_min(MASS_MIN)
        .with_x_axis_max(MASS_MAX)
        .with_y_axis_min(0.0);
    let scene = render_multiple(plots, layout);

    match output_kind(path) {
        PlotOutput::Svg => {
            let svg = SvgBackend.render_scene(&scene);
            std::fs::write(path, svg)?;
        }
        PlotOutput::Png => {
            #[cfg(feature = "plot-png")]
            {
                let png = PngBackend::new()
                    .render_scene(&scene)
                    .map_err(|err| format!("failed to render PNG plot: {err}"))?;
                std::fs::write(path, png)?;
            }
            #[cfg(not(feature = "plot-png"))]
            {
                return Err("PNG plot output requires the nano-io `plot-png` feature; rebuild with --features \"http plot plot-png\"".into());
            }
        }
    }
    Ok(())
}

#[cfg(all(feature = "http", not(feature = "plot")))]
fn write_stack_plot(_path: &str, _histograms: &StackHistograms) -> Result<(), Box<dyn Error>> {
    Err("--plot requires plotting support; rebuild with --features plot".into())
}

#[cfg(all(feature = "http", feature = "plot"))]
#[derive(Debug, Clone, Copy)]
enum PlotOutput {
    Svg,
    Png,
}

#[cfg(all(feature = "http", feature = "plot"))]
fn output_kind(path: &str) -> PlotOutput {
    match std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some(extension) if extension.eq_ignore_ascii_case("png") => PlotOutput::Png,
        _ => PlotOutput::Svg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_edges_match_df103() {
        let edges = histogram_edges();
        assert_eq!(edges.len(), NBINS + 1);
        assert_eq!(edges[0], MASS_MIN);
        assert_eq!(edges[NBINS], MASS_MAX);
    }

    #[test]
    fn sample_wiring_matches_df103() {
        let sample_table = samples();
        assert_eq!(sample_table.len(), 8);
        assert_eq!(sample_table[0].file, "SMHiggsToZZTo4L.root");
        assert_eq!(sample_table[0].channels, ALL_CHANNELS);
        assert_eq!(sample_table[4].channels, DOUBLE_MU_DATA_CHANNELS);
        assert_eq!(sample_table[5].channels, DOUBLE_MU_DATA_CHANNELS);
        assert_eq!(sample_table[6].channels, FOUR_EL);
        assert_eq!(sample_table[7].channels, FOUR_EL);
    }

    #[cfg(feature = "plot")]
    #[test]
    fn renders_stack_plot_svg_from_precomputed_bins() {
        let path = std::env::temp_dir().join(format!(
            "nano-higgs4l-stack-plot-test-{}.svg",
            std::process::id()
        ));
        let histograms = StackHistograms {
            edges: histogram_edges(),
            signal: (0..NBINS)
                .map(|index| if index == 18 { 3.0 } else { 0.2 })
                .collect(),
            background: (0..NBINS).map(|index| 0.5 + index as f64 * 0.02).collect(),
            data: (0..NBINS)
                .map(|index| if index == 18 { 5.0 } else { (index % 3) as f64 })
                .collect(),
        };

        write_stack_plot(path.to_str().expect("utf-8 temp path"), &histograms)
            .expect("stack SVG renders");
        let svg = std::fs::read_to_string(&path).expect("stack SVG can be read");
        let _ = std::fs::remove_file(&path);

        assert!(svg.contains("<svg"));
        assert!(svg.contains("CMS Open Data"));
    }
}
