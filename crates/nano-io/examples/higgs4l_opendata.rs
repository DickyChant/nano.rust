#[cfg(feature = "http")]
pub const DEFAULT_URL: &str =
    "https://eospublic.cern.ch//eos/root-eos/cms_opendata_2012_nanoaod_skimmed/SMHiggsToZZTo4L.root";
#[cfg(feature = "http")]
const DEFAULT_CHUNK_SIZE: usize = 4096;
#[cfg(feature = "http")]
const Z_MASS: f64 = 91.2;

use std::error::Error;

#[cfg(all(feature = "http", feature = "plot"))]
#[path = "plot_hist/mod.rs"]
mod plot_hist;

#[cfg(feature = "http")]
fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse()?;
    ensure_plot_feature(&options.plot)?;
    if options.insecure {
        std::env::set_var("NANO_HTTP_INSECURE", "1");
    }

    let report = analyze_source(&options.source, options.events)?;
    println!("source: {}", options.source);
    println!("events_read: {}", report.events_read);
    println!(
        "selected_4mu: {}\nselected_4e: {}\nselected_2e2mu: {}\ntotal_selected: {}",
        report.count_4mu,
        report.count_4e,
        report.count_2e2mu,
        report.total_selected()
    );
    write_or_print_mass_histogram(&options.plot, &report.h_masses)?;
    if let Some(path) = options.dump_selected.as_deref() {
        write_selected_dump(path, &report.selected)?;
        println!("selected_dump: {}", path);
    }
    println!(
        "bytes_fetched: {} / {}",
        report.bytes_fetched, report.file_size
    );
    Ok(())
}

#[cfg(not(feature = "http"))]
fn main() -> Result<(), Box<dyn Error>> {
    Err("higgs4l_opendata requires the nano-io `http` feature".into())
}

#[cfg(feature = "http")]
#[derive(Debug, Clone)]
struct Options {
    source: String,
    events: Option<usize>,
    insecure: bool,
    dump_selected: Option<String>,
    plot: Option<String>,
}

#[cfg(feature = "http")]
impl Options {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut source = DEFAULT_URL.to_string();
        let mut events = None;
        let mut insecure = env_flag("NANO_HTTP_INSECURE");
        let mut dump_selected = None;
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
                    events = Some(value);
                }
                "--dump-selected" => {
                    dump_selected = Some(args.next().ok_or("missing value after --dump-selected")?);
                }
                "--plot" => {
                    plot = Some(args.next().ok_or("missing value after --plot")?);
                }
                "-h" | "--help" => {
                    return Err(format!(
                        "usage: higgs4l_opendata [url-or-file] [n] [--events n] [--insecure] [--dump-selected path] [--plot path]\ndefault URL: {DEFAULT_URL}"
                    )
                    .into());
                }
                other => positional.push(other.to_string()),
            }
        }

        if let Some(first) = positional.first() {
            source = first.clone();
            if let Some(second) = positional.get(1) {
                events = Some(
                    second
                        .parse::<usize>()
                        .map_err(|err| format!("invalid event count: {err}"))?,
                );
            }
            if positional.len() > 2 {
                return Err("too many positional arguments".into());
            }
        }

        Ok(Self {
            source,
            events,
            insecure,
            dump_selected,
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
        println!("h_mass_plot: {path}");
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
            title: "Higgs -> ZZ -> 4l (CMS Open Data)",
            x_label: "m(4l) [GeV]",
            y_label: "Candidates",
            bins: histogram_bins().len(),
            range: (70.0, 180.0),
            color: "#9b2c2c",
        },
    )
}

#[cfg(feature = "http")]
#[derive(Debug, Default, Clone)]
pub struct AnalysisReport {
    pub events_read: usize,
    pub count_4mu: usize,
    pub count_4e: usize,
    pub count_2e2mu: usize,
    pub h_masses: Vec<f64>,
    pub selected: Vec<SelectedCandidate>,
    pub bytes_fetched: u64,
    pub file_size: u64,
}

#[cfg(feature = "http")]
impl AnalysisReport {
    pub fn total_selected(&self) -> usize {
        self.count_4mu + self.count_4e + self.count_2e2mu
    }
}

#[cfg(feature = "http")]
#[derive(Debug, Clone)]
pub struct SelectedCandidate {
    pub run: i32,
    pub luminosity_block: u32,
    pub event: u64,
    pub channel: &'static str,
    pub h_mass: f32,
    pub z1_mass: f32,
    pub z2_mass: f32,
}

#[cfg(feature = "http")]
pub fn higgs4l_schema() -> nano_core::BranchSchema {
    use nano_core::{BranchSchema, BranchSpec, BranchType};

    BranchSchema::new([
        BranchSpec::new("run", BranchType::I32),
        BranchSpec::new("luminosityBlock", BranchType::U32),
        BranchSpec::new("event", BranchType::U64),
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
        BranchSpec::new("Muon_phi", BranchType::VecF32),
        BranchSpec::new("Muon_mass", BranchType::VecF32),
        BranchSpec::new("Muon_charge", BranchType::VecI32),
        BranchSpec::new("Muon_pfRelIso04_all", BranchType::VecF32),
        BranchSpec::new("Muon_dxy", BranchType::VecF32),
        BranchSpec::new("Muon_dz", BranchType::VecF32),
        BranchSpec::new("Muon_dxyErr", BranchType::VecF32),
        BranchSpec::new("Muon_dzErr", BranchType::VecF32),
        BranchSpec::new("nElectron", BranchType::U32),
        BranchSpec::new("Electron_pt", BranchType::VecF32),
        BranchSpec::new("Electron_eta", BranchType::VecF32),
        BranchSpec::new("Electron_phi", BranchType::VecF32),
        BranchSpec::new("Electron_mass", BranchType::VecF32),
        BranchSpec::new("Electron_charge", BranchType::VecI32),
        BranchSpec::new("Electron_pfRelIso03_all", BranchType::VecF32),
        BranchSpec::new("Electron_dxy", BranchType::VecF32),
        BranchSpec::new("Electron_dz", BranchType::VecF32),
        BranchSpec::new("Electron_dxyErr", BranchType::VecF32),
        BranchSpec::new("Electron_dzErr", BranchType::VecF32),
    ])
    .expect("valid Higgs four-lepton branch schema")
}

#[cfg(feature = "http")]
pub fn analyze_source(
    source: &str,
    limit: Option<usize>,
) -> Result<AnalysisReport, Box<dyn Error>> {
    let schema = higgs4l_schema();
    if is_http_url(source) {
        let mut events = nano_io::events_url_chunked(source, &schema, DEFAULT_CHUNK_SIZE)?;
        let file_size = events.file_size();
        let mut report = analyze_events(&mut events, limit)?;
        report.bytes_fetched = events.bytes_fetched();
        report.file_size = file_size;
        Ok(report)
    } else {
        let path = std::path::Path::new(source);
        let file_size = std::fs::metadata(path)?.len();
        let mut events = nano_io::events_chunked(path, &schema, DEFAULT_CHUNK_SIZE)?;
        let mut report = analyze_events(&mut events, limit)?;
        report.file_size = file_size;
        Ok(report)
    }
}

#[cfg(feature = "http")]
pub fn analyze_events<I>(
    events: &mut I,
    limit: Option<usize>,
) -> Result<AnalysisReport, Box<dyn Error>>
where
    I: Iterator<Item = nano_io::Result<nano_core::Event>>,
{
    let mut report = AnalysisReport::default();
    let max_events = limit.unwrap_or(usize::MAX);

    for event in events.take(max_events) {
        let event = event?;
        report.events_read += 1;
        let id = EventId::from_event(&event)?;

        if let Some(candidate) = reco_higgs_to_4mu(&event, id)? {
            report.count_4mu += 1;
            report.h_masses.push(f64::from(candidate.h_mass));
            report.selected.push(candidate);
        }
        if let Some(candidate) = reco_higgs_to_4el(&event, id)? {
            report.count_4e += 1;
            report.h_masses.push(f64::from(candidate.h_mass));
            report.selected.push(candidate);
        }
        if let Some(candidate) = reco_higgs_to_2el2mu(&event, id)? {
            report.count_2e2mu += 1;
            report.h_masses.push(f64::from(candidate.h_mass));
            report.selected.push(candidate);
        }
    }

    Ok(report)
}

#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy)]
struct EventId {
    run: i32,
    luminosity_block: u32,
    event: u64,
}

#[cfg(feature = "http")]
impl EventId {
    fn from_event(event: &nano_core::Event) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            run: event.scalar::<i32>("run")?,
            luminosity_block: event.scalar::<u32>("luminosityBlock")?,
            event: event.scalar::<u64>("event")?,
        })
    }
}

#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy)]
struct CandidateMasses {
    h_mass: f32,
    z_masses: [f32; 2],
}

#[cfg(feature = "http")]
impl CandidateMasses {
    fn selected(self, id: EventId, channel: &'static str) -> SelectedCandidate {
        SelectedCandidate {
            run: id.run,
            luminosity_block: id.luminosity_block,
            event: id.event,
            channel,
            h_mass: self.h_mass,
            z1_mass: self.z_masses[0],
            z2_mass: self.z_masses[1],
        }
    }
}

#[cfg(feature = "http")]
fn reco_higgs_to_4mu(
    event: &nano_core::Event,
    id: EventId,
) -> Result<Option<SelectedCandidate>, Box<dyn Error>> {
    let muons = MuonBranches::from_event(event)?;

    // df103 selection_4mu: event-level cuts on all muons, then exactly two
    // positive and two negative muons.
    if muons.n < 4
        || !all_abs_lt(muons.iso, 0.40)
        || !all_gt(muons.pt, 5.0)
        || !all_abs_lt(muons.eta, 2.4)
        || !track_quality(muons.dxy, muons.dz, muons.dxy_err, muons.dz_err)
        || muons.n != 4
        || count_charge(muons.charge, 1) != 2
        || count_charge(muons.charge, -1) != 2
    {
        return Ok(None);
    }

    // df103 reco_zz_to_4l + filter_z_dr + filter_z_candidates.
    let Some(z_idx) = reco_zz_to_4l(muons.pt, muons.eta, muons.phi, muons.mass, muons.charge)
    else {
        return Ok(None);
    };
    if !filter_z_dr(&z_idx, muons.eta, muons.phi) {
        return Ok(None);
    }
    let z_masses = compute_z_masses_4l(&z_idx, muons.pt, muons.eta, muons.phi, muons.mass);
    if !filter_z_candidates(z_masses) {
        return Ok(None);
    }

    Ok(Some(
        CandidateMasses {
            h_mass: compute_higgs_mass_4l(&z_idx, muons.pt, muons.eta, muons.phi, muons.mass),
            z_masses,
        }
        .selected(id, "4mu"),
    ))
}

#[cfg(feature = "http")]
fn reco_higgs_to_4el(
    event: &nano_core::Event,
    id: EventId,
) -> Result<Option<SelectedCandidate>, Box<dyn Error>> {
    let electrons = ElectronBranches::from_event(event)?;

    // df103 selection_4el.
    if electrons.n < 4
        || !all_abs_lt(electrons.iso, 0.40)
        || !all_gt(electrons.pt, 7.0)
        || !all_abs_lt(electrons.eta, 2.5)
        || !track_quality(
            electrons.dxy,
            electrons.dz,
            electrons.dxy_err,
            electrons.dz_err,
        )
        || electrons.n != 4
        || count_charge(electrons.charge, 1) != 2
        || count_charge(electrons.charge, -1) != 2
    {
        return Ok(None);
    }

    // Same C++ helper as the 4mu channel, applied to electron branches.
    let Some(z_idx) = reco_zz_to_4l(
        electrons.pt,
        electrons.eta,
        electrons.phi,
        electrons.mass,
        electrons.charge,
    ) else {
        return Ok(None);
    };
    if !filter_z_dr(&z_idx, electrons.eta, electrons.phi) {
        return Ok(None);
    }
    let z_masses = compute_z_masses_4l(
        &z_idx,
        electrons.pt,
        electrons.eta,
        electrons.phi,
        electrons.mass,
    );
    if !filter_z_candidates(z_masses) {
        return Ok(None);
    }

    Ok(Some(
        CandidateMasses {
            h_mass: compute_higgs_mass_4l(
                &z_idx,
                electrons.pt,
                electrons.eta,
                electrons.phi,
                electrons.mass,
            ),
            z_masses,
        }
        .selected(id, "4e"),
    ))
}

#[cfg(feature = "http")]
fn reco_higgs_to_2el2mu(
    event: &nano_core::Event,
    id: EventId,
) -> Result<Option<SelectedCandidate>, Box<dyn Error>> {
    let muons = MuonBranches::from_event(event)?;
    let electrons = ElectronBranches::from_event(event)?;

    // df103 selection_2el2mu. This channel intentionally keeps ROOT's
    // semantics: cuts and charge sums apply to all leptons in the event, while
    // the C++ mass helpers use the first two electrons and first two muons.
    if electrons.n < 2
        || muons.n < 2
        || !all_abs_lt(electrons.eta, 2.5)
        || !all_abs_lt(muons.eta, 2.4)
        || !pt_cuts(muons.pt, electrons.pt)
        || !dr_cuts(muons.eta, muons.phi, electrons.eta, electrons.phi)
        || !all_abs_lt(electrons.iso, 0.40)
        || !all_abs_lt(muons.iso, 0.40)
        || !track_quality(
            electrons.dxy,
            electrons.dz,
            electrons.dxy_err,
            electrons.dz_err,
        )
        || !track_quality(muons.dxy, muons.dz, muons.dxy_err, muons.dz_err)
        || sum_charge(electrons.charge) != 0
        || sum_charge(muons.charge) != 0
    {
        return Ok(None);
    }

    let z_masses = compute_z_masses_2el2mu(
        electrons.pt,
        electrons.eta,
        electrons.phi,
        electrons.mass,
        muons.pt,
        muons.eta,
        muons.phi,
        muons.mass,
    );
    if !filter_z_candidates(z_masses) {
        return Ok(None);
    }

    Ok(Some(
        CandidateMasses {
            h_mass: compute_higgs_mass_2el2mu(
                electrons.pt,
                electrons.eta,
                electrons.phi,
                electrons.mass,
                muons.pt,
                muons.eta,
                muons.phi,
                muons.mass,
            ),
            z_masses,
        }
        .selected(id, "2e2mu"),
    ))
}

#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy)]
struct MuonBranches<'a> {
    n: usize,
    pt: &'a [f32],
    eta: &'a [f32],
    phi: &'a [f32],
    mass: &'a [f32],
    charge: &'a [i32],
    iso: &'a [f32],
    dxy: &'a [f32],
    dz: &'a [f32],
    dxy_err: &'a [f32],
    dz_err: &'a [f32],
}

#[cfg(feature = "http")]
impl<'a> MuonBranches<'a> {
    fn from_event(event: &'a nano_core::Event) -> Result<Self, Box<dyn Error>> {
        let branches = Self {
            n: event.scalar::<u32>("nMuon")? as usize,
            pt: event.vector_ref::<f32>("Muon_pt")?,
            eta: event.vector_ref::<f32>("Muon_eta")?,
            phi: event.vector_ref::<f32>("Muon_phi")?,
            mass: event.vector_ref::<f32>("Muon_mass")?,
            charge: event.vector_ref::<i32>("Muon_charge")?,
            iso: event.vector_ref::<f32>("Muon_pfRelIso04_all")?,
            dxy: event.vector_ref::<f32>("Muon_dxy")?,
            dz: event.vector_ref::<f32>("Muon_dz")?,
            dxy_err: event.vector_ref::<f32>("Muon_dxyErr")?,
            dz_err: event.vector_ref::<f32>("Muon_dzErr")?,
        };
        branches.validate("Muon")?;
        Ok(branches)
    }

    fn validate(&self, label: &str) -> Result<(), Box<dyn Error>> {
        validate_lengths(
            label,
            self.n,
            &[
                self.pt.len(),
                self.eta.len(),
                self.phi.len(),
                self.mass.len(),
                self.charge.len(),
                self.iso.len(),
                self.dxy.len(),
                self.dz.len(),
                self.dxy_err.len(),
                self.dz_err.len(),
            ],
        )
    }
}

#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy)]
struct ElectronBranches<'a> {
    n: usize,
    pt: &'a [f32],
    eta: &'a [f32],
    phi: &'a [f32],
    mass: &'a [f32],
    charge: &'a [i32],
    iso: &'a [f32],
    dxy: &'a [f32],
    dz: &'a [f32],
    dxy_err: &'a [f32],
    dz_err: &'a [f32],
}

#[cfg(feature = "http")]
impl<'a> ElectronBranches<'a> {
    fn from_event(event: &'a nano_core::Event) -> Result<Self, Box<dyn Error>> {
        let branches = Self {
            n: event.scalar::<u32>("nElectron")? as usize,
            pt: event.vector_ref::<f32>("Electron_pt")?,
            eta: event.vector_ref::<f32>("Electron_eta")?,
            phi: event.vector_ref::<f32>("Electron_phi")?,
            mass: event.vector_ref::<f32>("Electron_mass")?,
            charge: event.vector_ref::<i32>("Electron_charge")?,
            iso: event.vector_ref::<f32>("Electron_pfRelIso03_all")?,
            dxy: event.vector_ref::<f32>("Electron_dxy")?,
            dz: event.vector_ref::<f32>("Electron_dz")?,
            dxy_err: event.vector_ref::<f32>("Electron_dxyErr")?,
            dz_err: event.vector_ref::<f32>("Electron_dzErr")?,
        };
        branches.validate("Electron")?;
        Ok(branches)
    }

    fn validate(&self, label: &str) -> Result<(), Box<dyn Error>> {
        validate_lengths(
            label,
            self.n,
            &[
                self.pt.len(),
                self.eta.len(),
                self.phi.len(),
                self.mass.len(),
                self.charge.len(),
                self.iso.len(),
                self.dxy.len(),
                self.dz.len(),
                self.dxy_err.len(),
                self.dz_err.len(),
            ],
        )
    }
}

#[cfg(feature = "http")]
fn validate_lengths(label: &str, n: usize, lengths: &[usize]) -> Result<(), Box<dyn Error>> {
    if lengths.iter().all(|len| *len == n) {
        Ok(())
    } else {
        Err(format!("{label} branch length mismatch: n={n}, lengths={lengths:?}").into())
    }
}

#[cfg(feature = "http")]
fn reco_zz_to_4l(
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
    charge: &[i32],
) -> Option<[[usize; 2]; 2]> {
    // Match ROOT df103 exactly: the C++ helper says `auto best_mass = -1`,
    // so the stored best mass is an int and every accepted candidate mass is
    // truncated before the next comparison. The candidate mass itself remains
    // the double precision ROOT::Math::PtEtaPhiMVector mass.
    let mut best_mass = -1_i32;
    let mut best_pair = None;

    for i1 in 0..pt.len() {
        for i2 in i1 + 1..pt.len() {
            if charge[i1] == charge[i2] {
                continue;
            }
            let this_mass = invariant_mass(&[
                Lepton::new(pt[i1], eta[i1], phi[i1], mass[i1]),
                Lepton::new(pt[i2], eta[i2], phi[i2], mass[i2]),
            ]);
            if (Z_MASS - this_mass).abs() < (Z_MASS - f64::from(best_mass)).abs() {
                best_mass = this_mass as i32;
                best_pair = Some([i1, i2]);
            }
        }
    }

    let z1 = best_pair?;
    let mut rest = [usize::MAX; 2];
    let mut n_rest = 0;
    for index in 0..4 {
        if index != z1[0] && index != z1[1] {
            rest[n_rest] = index;
            n_rest += 1;
        }
    }

    Some([z1, rest])
}

#[cfg(feature = "http")]
fn compute_z_masses_4l(
    idx: &[[usize; 2]; 2],
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
) -> [f32; 2] {
    let mut z_masses = [0.0; 2];
    for (slot, pair) in idx.iter().enumerate() {
        z_masses[slot] = invariant_mass(&[
            Lepton::new(pt[pair[0]], eta[pair[0]], phi[pair[0]], mass[pair[0]]),
            Lepton::new(pt[pair[1]], eta[pair[1]], phi[pair[1]], mass[pair[1]]),
        ]) as f32;
    }

    if (f64::from(z_masses[0]) - Z_MASS).abs() < (f64::from(z_masses[1]) - Z_MASS).abs() {
        z_masses
    } else {
        [z_masses[1], z_masses[0]]
    }
}

#[cfg(feature = "http")]
fn compute_higgs_mass_4l(
    idx: &[[usize; 2]; 2],
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    mass: &[f32],
) -> f32 {
    invariant_mass(&[
        Lepton::new(
            pt[idx[0][0]],
            eta[idx[0][0]],
            phi[idx[0][0]],
            mass[idx[0][0]],
        ),
        Lepton::new(
            pt[idx[0][1]],
            eta[idx[0][1]],
            phi[idx[0][1]],
            mass[idx[0][1]],
        ),
        Lepton::new(
            pt[idx[1][0]],
            eta[idx[1][0]],
            phi[idx[1][0]],
            mass[idx[1][0]],
        ),
        Lepton::new(
            pt[idx[1][1]],
            eta[idx[1][1]],
            phi[idx[1][1]],
            mass[idx[1][1]],
        ),
    ]) as f32
}

#[cfg(feature = "http")]
// Parallel electron/muon kinematic arrays mirror the source NanoAOD branches.
#[allow(clippy::too_many_arguments)]
fn compute_z_masses_2el2mu(
    el_pt: &[f32],
    el_eta: &[f32],
    el_phi: &[f32],
    el_mass: &[f32],
    mu_pt: &[f32],
    mu_eta: &[f32],
    mu_phi: &[f32],
    mu_mass: &[f32],
) -> [f32; 2] {
    let mu_z = invariant_mass(&[
        Lepton::new(mu_pt[0], mu_eta[0], mu_phi[0], mu_mass[0]),
        Lepton::new(mu_pt[1], mu_eta[1], mu_phi[1], mu_mass[1]),
    ]);
    let el_z = invariant_mass(&[
        Lepton::new(el_pt[0], el_eta[0], el_phi[0], el_mass[0]),
        Lepton::new(el_pt[1], el_eta[1], el_phi[1], el_mass[1]),
    ]);

    if (mu_z - Z_MASS).abs() < (el_z - Z_MASS).abs() {
        [mu_z as f32, el_z as f32]
    } else {
        [el_z as f32, mu_z as f32]
    }
}

#[cfg(feature = "http")]
// Parallel electron/muon kinematic arrays mirror the source NanoAOD branches.
#[allow(clippy::too_many_arguments)]
fn compute_higgs_mass_2el2mu(
    el_pt: &[f32],
    el_eta: &[f32],
    el_phi: &[f32],
    el_mass: &[f32],
    mu_pt: &[f32],
    mu_eta: &[f32],
    mu_phi: &[f32],
    mu_mass: &[f32],
) -> f32 {
    invariant_mass(&[
        Lepton::new(mu_pt[0], mu_eta[0], mu_phi[0], mu_mass[0]),
        Lepton::new(mu_pt[1], mu_eta[1], mu_phi[1], mu_mass[1]),
        Lepton::new(el_pt[0], el_eta[0], el_phi[0], el_mass[0]),
        Lepton::new(el_pt[1], el_eta[1], el_phi[1], el_mass[1]),
    ]) as f32
}

#[cfg(feature = "http")]
fn filter_z_dr(idx: &[[usize; 2]; 2], eta: &[f32], phi: &[f32]) -> bool {
    idx.iter()
        .all(|pair| delta_r(eta[pair[0]], eta[pair[1]], phi[pair[0]], phi[pair[1]]) >= 0.02)
}

#[cfg(feature = "http")]
fn filter_z_candidates(z_masses: [f32; 2]) -> bool {
    z_masses[0] > 40.0 && z_masses[0] < 120.0 && z_masses[1] > 12.0 && z_masses[1] < 120.0
}

#[cfg(feature = "http")]
fn pt_cuts(mu_pt: &[f32], el_pt: &[f32]) -> bool {
    let mut mu_sorted = mu_pt.to_vec();
    mu_sorted.sort_by(|left, right| right.total_cmp(left));
    if mu_sorted[0] > 20.0 && mu_sorted[1] > 10.0 {
        return true;
    }

    let mut el_sorted = el_pt.to_vec();
    el_sorted.sort_by(|left, right| right.total_cmp(left));
    el_sorted[0] > 20.0 && el_sorted[1] > 10.0
}

#[cfg(feature = "http")]
fn dr_cuts(mu_eta: &[f32], mu_phi: &[f32], el_eta: &[f32], el_phi: &[f32]) -> bool {
    let mu_dr = delta_r(mu_eta[0], mu_eta[1], mu_phi[0], mu_phi[1]);
    let el_dr = delta_r(el_eta[0], el_eta[1], el_phi[0], el_phi[1]);
    mu_dr >= 0.02 && el_dr >= 0.02
}

#[cfg(feature = "http")]
fn track_quality(dxy: &[f32], dz: &[f32], dxy_err: &[f32], dz_err: &[f32]) -> bool {
    dxy.iter()
        .zip(dz)
        .zip(dxy_err)
        .zip(dz_err)
        .all(|(((dxy, dz), dxy_err), dz_err)| {
            let ip3d = (dxy * dxy + dz * dz).sqrt();
            let err3d = (dxy_err * dxy_err + dz_err * dz_err).sqrt();
            let sip3d = ip3d / err3d;
            sip3d < 4.0 && dxy.abs() < 0.5 && dz.abs() < 1.0
        })
}

#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy)]
struct Lepton {
    pt: f64,
    eta: f64,
    phi: f64,
    mass: f64,
}

#[cfg(feature = "http")]
impl Lepton {
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
fn invariant_mass(leptons: &[Lepton]) -> f64 {
    let (mut energy, mut px, mut py, mut pz) = (0.0, 0.0, 0.0, 0.0);
    for lepton in leptons {
        let (e, x, y, z) = four_vector(*lepton);
        energy += e;
        px += x;
        py += y;
        pz += z;
    }
    (energy * energy - px * px - py * py - pz * pz)
        .max(0.0)
        .sqrt()
}

#[cfg(feature = "http")]
fn four_vector(lepton: Lepton) -> (f64, f64, f64, f64) {
    let px = lepton.pt * lepton.phi.cos();
    let py = lepton.pt * lepton.phi.sin();
    let pz = lepton.pt * lepton.eta.sinh();
    let energy = (px * px + py * py + pz * pz + lepton.mass * lepton.mass).sqrt();
    (energy, px, py, pz)
}

#[cfg(feature = "http")]
fn delta_r(eta1: f32, eta2: f32, phi1: f32, phi2: f32) -> f32 {
    let deta = eta1 - eta2;
    let dphi = delta_phi(phi1, phi2);
    (deta * deta + dphi * dphi).sqrt()
}

#[cfg(feature = "http")]
fn delta_phi(phi1: f32, phi2: f32) -> f32 {
    let c = f64::from(std::f32::consts::PI);
    let mut dphi = f64::from(phi2 - phi1) % (2.0 * c);
    if dphi < -c {
        dphi += 2.0 * c;
    }
    if dphi > c {
        dphi -= 2.0 * c;
    }
    dphi as f32
}

#[cfg(feature = "http")]
fn all_abs_lt(values: &[f32], threshold: f32) -> bool {
    values.iter().all(|value| value.abs() < threshold)
}

#[cfg(feature = "http")]
fn all_gt(values: &[f32], threshold: f32) -> bool {
    values.iter().all(|value| *value > threshold)
}

#[cfg(feature = "http")]
fn count_charge(charges: &[i32], target: i32) -> usize {
    charges.iter().filter(|charge| **charge == target).count()
}

#[cfg(feature = "http")]
fn sum_charge(charges: &[i32]) -> i32 {
    charges.iter().sum()
}

#[cfg(feature = "http")]
pub fn histogram_bins() -> Vec<(f64, f64)> {
    (0..11)
        .map(|index| {
            let low = 70.0 + f64::from(index) * 10.0;
            (low, low + 10.0)
        })
        .collect()
}

#[cfg(feature = "http")]
pub fn histogram_counts(values: &[f64], bins: &[(f64, f64)]) -> Vec<usize> {
    bins.iter()
        .map(|(low, high)| count_range(values, *low, *high))
        .collect()
}

#[cfg(feature = "http")]
fn print_histogram(values: &[f64]) {
    let bins = histogram_bins();
    let counts = histogram_counts(values, &bins);
    let max_count = counts.iter().copied().max().unwrap_or(0).max(1);
    println!("h_mass_histogram_gev:");
    for ((low, high), count) in bins.iter().zip(counts) {
        let width = (count * 40).div_ceil(max_count);
        println!("{low:>5.0}-{high:<5.0} {count:>5} {}", "#".repeat(width));
    }
}

#[cfg(feature = "http")]
fn write_selected_dump(path: &str, selected: &[SelectedCandidate]) -> Result<(), Box<dyn Error>> {
    use std::io::Write;

    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    writeln!(
        writer,
        "run,luminosityBlock,event,channel,H_mass,Z1_mass,Z2_mass"
    )?;
    for candidate in selected {
        writeln!(
            writer,
            "{},{},{},{},{:.9},{:.9},{:.9}",
            candidate.run,
            candidate.luminosity_block,
            candidate.event,
            candidate.channel,
            candidate.h_mass,
            candidate.z1_mass,
            candidate.z2_mass
        )?;
    }
    Ok(())
}

#[cfg(feature = "http")]
fn count_range(values: &[f64], low: f64, high: f64) -> usize {
    values
        .iter()
        .filter(|value| **value >= low && **value < high)
        .count()
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
