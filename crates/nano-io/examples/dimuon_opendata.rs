#[cfg(feature = "http")]
const DEFAULT_URL: &str = "https://eospublic.cern.ch//eos/opendata/cms/derived-data/AOD2NanoAODOutreachTool/Run2012BC_DoubleMuParked_Muons.root";
#[cfg(feature = "http")]
const DEFAULT_EVENTS: usize = 1_000;

use std::error::Error;

#[cfg(all(feature = "http", feature = "plot"))]
#[path = "plot_hist/mod.rs"]
mod plot_hist;

#[cfg(feature = "http")]
fn main() -> Result<(), Box<dyn Error>> {
    use nano_core::{BranchSchema, BranchSpec, BranchType};
    use nano_io::events_url_chunked;

    let options = Options::parse()?;
    ensure_plot_feature(&options.plot)?;
    if options.insecure {
        std::env::set_var("NANO_HTTP_INSECURE", "1");
    }

    let schema = BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
        BranchSpec::new("Muon_mass", BranchType::VecF32),
        BranchSpec::new("Muon_charge", BranchType::VecI32),
    ])?;
    let mut events = events_url_chunked(&options.url, &schema, options.events.max(1))?;
    let file_size = events.file_size();

    let mut masses = Vec::new();
    let mut first_masses = Vec::new();
    let mut rows = 0_usize;
    for event in events.by_ref().take(options.events) {
        let event = event?;
        rows += 1;
        let n_muon = event.scalar::<u32>("nMuon")? as usize;
        let pt = event.vector_ref::<f32>("Muon_pt")?;
        let eta = event.vector_ref::<f32>("Muon_eta")?;
        let phi = event.vector_ref::<f32>("Muon_phi")?;
        let mass = event.vector_ref::<f32>("Muon_mass")?;
        let charge = event.vector_ref::<i32>("Muon_charge")?;
        if [pt.len(), eta.len(), phi.len(), mass.len(), charge.len()]
            .iter()
            .any(|len| *len != n_muon)
        {
            return Err(format!("Muon branch length mismatch in event {rows}").into());
        }

        if rows <= 5 {
            let pts = pt
                .iter()
                .map(|p| format!("{p:.4}"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("entry {} nMuon={} Muon_pt=[{}]", rows - 1, n_muon, pts);
        }

        if let Some(value) = leading_opposite_charge_mass(pt, eta, phi, mass, charge) {
            masses.push(value);
            if first_masses.len() < 8 {
                first_masses.push(value);
            }
        }
    }

    println!("source: {}", options.url);
    println!("events_read: {rows}");
    println!("opposite_charge_pairs: {}", masses.len());
    println!(
        "first_masses_gev: {}",
        first_masses
            .iter()
            .map(|mass| format!("{mass:.3}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("low_mass_2_12_gev: {}", count_range(&masses, 2.0, 12.0));
    println!("z_window_60_120_gev: {}", count_range(&masses, 60.0, 120.0));
    write_or_print_mass_histogram(&options.plot, &masses)?;
    println!("bytes_fetched: {} / {}", events.bytes_fetched(), file_size);

    Ok(())
}

#[cfg(not(feature = "http"))]
fn main() -> Result<(), Box<dyn Error>> {
    Err("dimuon_opendata requires the nano-io `http` feature".into())
}

#[cfg(feature = "http")]
#[derive(Debug, Clone)]
struct Options {
    url: String,
    events: usize,
    insecure: bool,
    plot: Option<String>,
}

#[cfg(feature = "http")]
impl Options {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut url = DEFAULT_URL.to_string();
        let mut events = DEFAULT_EVENTS;
        let mut insecure = env_flag("NANO_HTTP_INSECURE");
        let mut plot = None;
        let mut positional = Vec::new();

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--insecure" => insecure = true,
                "-n" | "--events" => {
                    let value = args
                        .next()
                        .ok_or("missing value after --events")?
                        .parse::<usize>()
                        .map_err(|err| format!("invalid event count: {err}"))?;
                    events = value;
                }
                "--plot" => {
                    plot = Some(args.next().ok_or("missing value after --plot")?);
                }
                "-h" | "--help" => {
                    return Err(format!(
                        "usage: dimuon_opendata [url] [n] [--events n] [--insecure] [--plot path]\ndefault URL: {DEFAULT_URL}"
                    )
                    .into());
                }
                other => positional.push(other.to_string()),
            }
        }

        if let Some(first) = positional.first() {
            if is_http_url(first) {
                url = first.clone();
                if let Some(second) = positional.get(1) {
                    events = second
                        .parse::<usize>()
                        .map_err(|err| format!("invalid event count: {err}"))?;
                }
                if positional.len() > 2 {
                    return Err("too many positional arguments".into());
                }
            } else {
                events = first
                    .parse::<usize>()
                    .map_err(|err| format!("invalid event count: {err}"))?;
                if positional.len() > 1 {
                    return Err("too many positional arguments".into());
                }
            }
        }

        Ok(Self {
            url,
            events,
            insecure,
            plot,
        })
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

#[cfg(all(feature = "http", feature = "plot"))]
fn write_or_print_mass_histogram(
    plot: &Option<String>,
    masses: &[f64],
) -> Result<(), Box<dyn Error>> {
    if let Some(path) = plot.as_deref() {
        write_mass_plot(path, masses)?;
        println!("mass_plot: {path}");
    } else {
        print_histogram(masses);
    }
    Ok(())
}

#[cfg(all(feature = "http", not(feature = "plot")))]
fn write_or_print_mass_histogram(
    plot: &Option<String>,
    masses: &[f64],
) -> Result<(), Box<dyn Error>> {
    let _ = plot;
    print_histogram(masses);
    Ok(())
}

#[cfg(all(feature = "http", feature = "plot"))]
fn write_mass_plot(path: &str, masses: &[f64]) -> Result<(), Box<dyn Error>> {
    plot_hist::write_histogram(
        path,
        masses,
        plot_hist::HistogramSpec {
            title: "Dimuon invariant mass (CMS Open Data)",
            x_label: "m(mu+mu-) [GeV]",
            y_label: "Candidates",
            bins: 10,
            range: (0.0, 200.0),
            color: "#2b6cb0",
        },
    )
}

#[cfg(feature = "http")]
fn leading_opposite_charge_mass(
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
    charge: &[i32],
) -> Option<f64> {
    let mut order = (0..pt.len()).collect::<Vec<_>>();
    order.sort_by(|&left, &right| pt[right].total_cmp(&pt[left]));

    for (left_pos, &left) in order.iter().enumerate() {
        for &right in &order[left_pos + 1..] {
            if charge[left] * charge[right] >= 0 {
                continue;
            }
            let value = dimuon_mass(
                Muon::new(pt[left], eta[left], phi[left], mass[left]),
                Muon::new(pt[right], eta[right], phi[right], mass[right]),
            );
            if value.is_finite() && value > 0.0 {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy)]
struct Muon {
    pt: f64,
    eta: f64,
    phi: f64,
    mass: f64,
}

#[cfg(feature = "http")]
impl Muon {
    fn new(pt: f32, eta: f32, phi: f32, mass: f32) -> Self {
        Self {
            pt: f64::from(pt),
            eta: f64::from(eta),
            phi: f64::from(phi),
            mass: f64::from(mass),
        }
    }
}

#[cfg(feature = "http")]
fn dimuon_mass(first: Muon, second: Muon) -> f64 {
    let (e1, px1, py1, pz1) = four_vector(first);
    let (e2, px2, py2, pz2) = four_vector(second);
    let energy = e1 + e2;
    let px = px1 + px2;
    let py = py1 + py2;
    let pz = pz1 + pz2;
    (energy * energy - px * px - py * py - pz * pz)
        .max(0.0)
        .sqrt()
}

#[cfg(feature = "http")]
fn four_vector(muon: Muon) -> (f64, f64, f64, f64) {
    let px = muon.pt * muon.phi.cos();
    let py = muon.pt * muon.phi.sin();
    let pz = muon.pt * muon.eta.sinh();
    let energy = (px * px + py * py + pz * pz + muon.mass * muon.mass).sqrt();
    (energy, px, py, pz)
}

#[cfg(feature = "http")]
fn count_range(values: &[f64], low: f64, high: f64) -> usize {
    values
        .iter()
        .filter(|value| **value >= low && **value < high)
        .count()
}

#[cfg(feature = "http")]
fn print_histogram(values: &[f64]) {
    let bins = [
        (0.0, 20.0),
        (20.0, 40.0),
        (40.0, 60.0),
        (60.0, 80.0),
        (80.0, 100.0),
        (100.0, 120.0),
        (120.0, 200.0),
    ];
    let counts = bins
        .iter()
        .map(|(low, high)| count_range(values, *low, *high))
        .collect::<Vec<_>>();
    let max_count = counts.iter().copied().max().unwrap_or(0).max(1);
    println!("mass_histogram_gev:");
    for ((low, high), count) in bins.iter().zip(counts) {
        let width = (count * 32).div_ceil(max_count);
        println!("{low:>5.0}-{high:<5.0} {count:>5} {}", "#".repeat(width));
    }
}

#[cfg(feature = "http")]
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

#[cfg(feature = "http")]
fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}
