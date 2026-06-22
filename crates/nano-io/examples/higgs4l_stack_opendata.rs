use std::error::Error;

#[cfg(feature = "http")]
#[allow(dead_code)]
#[path = "higgs4l_opendata.rs"]
mod higgs4l_opendata;

#[cfg(feature = "http")]
fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse()?;
    let config = higgs4l_opendata::load_higgs_config(options.config.as_deref())?;
    ensure_plot_feature(&options.plot)?;
    if options.insecure {
        std::env::set_var("NANO_HTTP_INSECURE", "1");
    }

    let report = analyze_stack_with_config(&options.sources(&config), options.events, &config)?;

    println!("luminosity_pb: {}", config.luminosity);
    println!(
        "histogram: nbins={} range=[{},{}]",
        config.histogram.bins, config.histogram.range[0], config.histogram.range[1]
    );
    println!("samples:");
    for sample in samples(&config)? {
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
        write_stack_plot(
            path,
            &report.histograms,
            &config.histogram,
            config.luminosity,
        )?;
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
    base_url: Option<String>,
    local_dir: Option<String>,
    events: Option<usize>,
    insecure: bool,
    plot: Option<String>,
    config: Option<String>,
}

#[cfg(feature = "http")]
impl Options {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut base_url = None;
        let mut local_dir = None;
        let mut events = None;
        let mut insecure = true;
        let mut plot = None;
        let mut config = None;

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--base-url" => base_url = Some(args.next().ok_or("missing value after --base-url")?),
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
                "--config" => config = Some(args.next().ok_or("missing value after --config")?),
                "-h" | "--help" => {
                    return Err(
                        "usage: higgs4l_stack_opendata [--base-url url | --local-dir dir] [--events n] [--secure|--insecure] [--plot path] [--config path]"
                            .into(),
                    )
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
            config,
        })
    }

    fn sources(&self, config: &higgs4l_opendata::HiggsConfig) -> SourceConfig {
        SourceConfig {
            base_url: self
                .base_url
                .clone()
                .unwrap_or_else(|| config.source.skimmed_base_url.clone()),
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
        let config = higgs4l_opendata::default_higgs_config().expect("default Higgs config parses");
        Self {
            base_url: config.source.skimmed_base_url,
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
    pub file: String,
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
    let config = higgs4l_opendata::default_higgs_config()?;
    analyze_stack_with_config(sources, limit, &config)
}

#[cfg(feature = "http")]
pub fn analyze_stack_with_config(
    sources: &SourceConfig,
    limit: Option<usize>,
    config: &higgs4l_opendata::HiggsConfig,
) -> Result<StackReport, Box<dyn Error>> {
    let mut histograms = StackHistograms {
        edges: histogram_edges_from(&config.histogram),
        signal: vec![0.0; config.histogram.bins],
        background: vec![0.0; config.histogram.bins],
        data: vec![0.0; config.histogram.bins],
    };
    let mut sample_reports = Vec::new();

    for sample in samples(config)? {
        let source = sources.source_for(&sample.file);
        let report = higgs4l_opendata::analyze_source_with_config(&source, limit, config)?;
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
                SampleKind::Signal => fill_weighted(
                    &mut histograms.signal,
                    mass,
                    sample.weight,
                    &config.histogram,
                ),
                SampleKind::Background => fill_weighted(
                    &mut histograms.background,
                    mass,
                    sample.weight,
                    &config.histogram,
                ),
                SampleKind::Data => {
                    fill_weighted(&mut histograms.data, mass, sample.weight, &config.histogram)
                }
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
#[derive(Debug, Clone)]
struct Sample {
    file: String,
    kind: SampleKind,
    channels: Vec<Channel>,
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

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "signal" => Some(Self::Signal),
            "background" => Some(Self::Background),
            "data" => Some(Self::Data),
            _ => None,
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
fn samples(config: &higgs4l_opendata::HiggsConfig) -> Result<Vec<Sample>, Box<dyn Error>> {
    config
        .sample
        .iter()
        .map(|sample| {
            let kind = SampleKind::from_label(&sample.role)
                .ok_or_else(|| format!("unknown sample role: {}", sample.role))?;
            let channels = sample
                .channels
                .iter()
                .map(|channel| {
                    Channel::from_label(channel)
                        .ok_or_else(|| format!("unknown sample channel: {channel}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Sample {
                file: sample.file.clone(),
                kind,
                channels,
                weight: sample_weight(config.luminosity, sample)?,
            })
        })
        .collect()
}

#[cfg(feature = "http")]
fn sample_weight(
    luminosity_pb: f64,
    sample: &higgs4l_opendata::SampleConfig,
) -> Result<f64, Box<dyn Error>> {
    use nano_analysis::{Pb, PbInv};
    use nano_io::samples::mc_normalization_factor_pb;

    match (sample.xsec, sample.nevt) {
        (Some(xsec), Some(nevt)) => {
            let mut weight = mc_normalization_factor_pb(Pb(xsec), PbInv(luminosity_pb), nevt)?;
            if sample.scale != 1.0 {
                weight *= sample.scale;
            }
            Ok(weight)
        }
        (None, None) => Ok(sample.scale),
        _ => Err(format!("sample {} must set both xsec and nevt", sample.name).into()),
    }
}

#[cfg(feature = "http")]
#[cfg(test)]
fn histogram_edges() -> Vec<f64> {
    let config = higgs4l_opendata::default_higgs_config().expect("default Higgs config parses");
    histogram_edges_from(&config.histogram)
}

#[cfg(feature = "http")]
fn histogram_edges_from(histogram: &higgs4l_opendata::HistogramConfig) -> Vec<f64> {
    let width = (histogram.range[1] - histogram.range[0]) / histogram.bins as f64;
    (0..=histogram.bins)
        .map(|index| histogram.range[0] + index as f64 * width)
        .collect()
}

#[cfg(feature = "http")]
fn fill_weighted(
    histogram: &mut [f64],
    value: f64,
    weight: f64,
    config: &higgs4l_opendata::HistogramConfig,
) {
    let mass_min = config.range[0];
    let mass_max = config.range[1];
    if !(mass_min..mass_max).contains(&value) {
        return;
    }
    let bin = ((value - mass_min) / (mass_max - mass_min) * config.bins as f64) as usize;
    if let Some(slot) = histogram.get_mut(bin) {
        *slot += weight;
    }
}

#[cfg(feature = "http")]
fn print_yields(histograms: &StackHistograms) {
    println!("bin,low,high,signal,background,data");
    for index in 0..histograms.signal.len() {
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
fn write_stack_plot(
    path: &str,
    histograms: &StackHistograms,
    histogram: &higgs4l_opendata::HistogramConfig,
    luminosity_pb: f64,
) -> Result<(), Box<dyn Error>> {
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
                .with_x_err(std::iter::repeat(half_width).take(histogram.bins))
                .with_y_err(data_yerr)
                .with_color("black")
                .with_size(4.0)
                .with_legend("Data"),
        ),
    ];
    let title = format!(
        "CMS Open Data, sqrt(s)=8 TeV, L={:.1} fb^-1",
        luminosity_pb / 1000.0
    );
    let layout = Layout::auto_from_plots(&plots)
        .with_title(title)
        .with_x_label("m(4l) [GeV]")
        .with_y_label("N_Events")
        .with_width(900.0)
        .with_height(700.0)
        .with_x_axis_min(histogram.range[0])
        .with_x_axis_max(histogram.range[1])
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
fn write_stack_plot(
    _path: &str,
    _histograms: &StackHistograms,
    _histogram: &higgs4l_opendata::HistogramConfig,
    _luminosity_pb: f64,
) -> Result<(), Box<dyn Error>> {
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
        let config = higgs4l_opendata::default_higgs_config().expect("default config parses");
        let edges = histogram_edges();
        assert_eq!(edges.len(), config.histogram.bins + 1);
        assert_eq!(edges[0], config.histogram.range[0]);
        assert_eq!(edges[config.histogram.bins], config.histogram.range[1]);
    }

    #[test]
    fn sample_wiring_matches_df103() {
        let config = higgs4l_opendata::default_higgs_config().expect("default config parses");
        let sample_table = samples(&config).expect("samples parse");
        assert_eq!(sample_table.len(), 8);
        assert_eq!(sample_table[0].file, "SMHiggsToZZTo4L.root");
        assert_eq!(
            sample_table[0].channels,
            vec![Channel::FourMu, Channel::FourEl, Channel::TwoElTwoMu]
        );
        assert_eq!(
            sample_table[4].channels,
            vec![Channel::FourMu, Channel::TwoElTwoMu]
        );
        assert_eq!(
            sample_table[5].channels,
            vec![Channel::FourMu, Channel::TwoElTwoMu]
        );
        assert_eq!(sample_table[6].channels, vec![Channel::FourEl]);
        assert_eq!(sample_table[7].channels, vec![Channel::FourEl]);
    }

    #[cfg(feature = "plot")]
    #[test]
    fn renders_stack_plot_svg_from_precomputed_bins() {
        let path = std::env::temp_dir().join(format!(
            "nano-higgs4l-stack-plot-test-{}.svg",
            std::process::id()
        ));
        let config = higgs4l_opendata::default_higgs_config().expect("default config parses");
        let histograms = StackHistograms {
            edges: histogram_edges(),
            signal: (0..config.histogram.bins)
                .map(|index| if index == 18 { 3.0 } else { 0.2 })
                .collect(),
            background: (0..config.histogram.bins)
                .map(|index| 0.5 + index as f64 * 0.02)
                .collect(),
            data: (0..config.histogram.bins)
                .map(|index| if index == 18 { 5.0 } else { (index % 3) as f64 })
                .collect(),
        };

        write_stack_plot(
            path.to_str().expect("utf-8 temp path"),
            &histograms,
            &config.histogram,
            config.luminosity,
        )
        .expect("stack SVG renders");
        let svg = std::fs::read_to_string(&path).expect("stack SVG can be read");
        let _ = std::fs::remove_file(&path);

        assert!(svg.contains("<svg"));
        assert!(svg.contains("CMS Open Data"));
    }
}
