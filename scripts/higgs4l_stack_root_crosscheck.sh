#!/usr/bin/env bash
set -euo pipefail

# Local ROOT cross-check for the full df103 Higgs discovery stack.
# Emits the same 36-bin weighted signal/background/data table as
# crates/nano-io/examples/higgs4l_stack_opendata.rs.

BASE_URL="root://eospublic.cern.ch//eos/root-eos/cms_opendata_2012_nanoaod_skimmed/"
LOCAL_DIR=""
HEADER="${ROOT_HIGGS4L_HEADER:-/usr/share/doc/root/tutorials/analysis/dataframe/df103_NanoAODHiggsAnalysis_python.h}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base-url)
      BASE_URL="${2:?missing value after --base-url}"
      shift 2
      ;;
    --local-dir)
      LOCAL_DIR="${2:?missing value after --local-dir}"
      shift 2
      ;;
    -h|--help)
      cat <<'USAGE'
usage: scripts/higgs4l_stack_root_crosscheck.sh [--base-url url | --local-dir dir]

Defaults to ROOT's xrootd EOS path. Set ROOT_HIGGS4L_HEADER to override the
df103 helper header path.
USAGE
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if ! command -v root >/dev/null 2>&1; then
  echo "error: ROOT is required for this local cross-check; install ROOT and ensure 'root' is on PATH" >&2
  exit 127
fi

if [[ ! -r "$HEADER" ]]; then
  echo "error: ROOT df103 helper header not found at $HEADER" >&2
  echo "       set ROOT_HIGGS4L_HEADER=/path/to/df103_NanoAODHiggsAnalysis_python.h" >&2
  exit 2
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/nano-higgs4l-stack-root.XXXXXX")"
macro="${tmpdir}/higgs4l_stack_root_crosscheck.C"
trap 'rm -rf "$tmpdir"' EXIT

cat >"$macro" <<ROOT_MACRO
#include <ROOT/RDataFrame.hxx>
#include <TH1D.h>

#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>

#include "$HEADER"

constexpr int NBINS = 36;
constexpr double MASS_MIN = 70.0;
constexpr double MASS_MAX = 180.0;
constexpr double LUMINOSITY = 11580.0;
constexpr double SCALE_ZZ_TO_4L = 1.386;

std::string trim_right_slashes(std::string value) {
  while (!value.empty() && value.back() == '/') {
    value.pop_back();
  }
  return value;
}

std::string source_path(const std::string &base_url, const std::string &local_dir, const char *file_name) {
  if (!local_dir.empty()) {
    return trim_right_slashes(local_dir) + "/" + file_name;
  }
  return trim_right_slashes(base_url) + "/" + file_name;
}

RNode filter_z_candidates_node(RNode df) {
  return df.Filter("Z_mass[0] > 40 && Z_mass[0] < 120", "Mass of first Z candidate in [40, 120]")
           .Filter("Z_mass[1] > 12 && Z_mass[1] < 120", "Mass of second Z candidate in [12, 120]");
}

RNode selection_4mu_node(RNode df) {
  return df.Filter("nMuon>=4", "At least four muons")
           .Filter("All(abs(Muon_pfRelIso04_all)<0.40)", "Require good isolation")
           .Filter("All(Muon_pt>5) && All(abs(Muon_eta)<2.4)", "Good muon kinematics")
           .Define("Muon_ip3d", "sqrt(Muon_dxy*Muon_dxy + Muon_dz*Muon_dz)")
           .Define("Muon_sip3d", "Muon_ip3d/sqrt(Muon_dxyErr*Muon_dxyErr + Muon_dzErr*Muon_dzErr)")
           .Filter("All(Muon_sip3d<4) && All(abs(Muon_dxy)<0.5) && All(abs(Muon_dz)<1.0)",
                   "Track close to primary vertex with small uncertainty")
           .Filter("nMuon==4 && Sum(Muon_charge==1)==2 && Sum(Muon_charge==-1)==2",
                   "Two positive and two negative muons");
}

RNode selection_4el_node(RNode df) {
  return df.Filter("nElectron>=4", "At least four electrons")
           .Filter("All(abs(Electron_pfRelIso03_all)<0.40)", "Require good isolation")
           .Filter("All(Electron_pt>7) && All(abs(Electron_eta)<2.5)", "Good Electron kinematics")
           .Define("Electron_ip3d", "sqrt(Electron_dxy*Electron_dxy + Electron_dz*Electron_dz)")
           .Define("Electron_sip3d",
                   "Electron_ip3d/sqrt(Electron_dxyErr*Electron_dxyErr + Electron_dzErr*Electron_dzErr)")
           .Filter("All(Electron_sip3d<4) && All(abs(Electron_dxy)<0.5) && All(abs(Electron_dz)<1.0)",
                   "Track close to primary vertex with small uncertainty")
           .Filter("nElectron==4 && Sum(Electron_charge==1)==2 && Sum(Electron_charge==-1)==2",
                   "Two positive and two negative electrons");
}

RNode selection_2el2mu_node(RNode df) {
  return df.Filter("nElectron>=2 && nMuon>=2", "At least two electrons and two muons")
           .Filter("All(abs(Electron_eta)<2.5) && All(abs(Muon_eta)<2.4)", "Eta cuts")
           .Filter("pt_cuts(Muon_pt, Electron_pt)", "Pt cuts")
           .Filter("dr_cuts(Muon_eta, Muon_phi, Electron_eta, Electron_phi)", "Dr cuts")
           .Filter("All(abs(Electron_pfRelIso03_all)<0.40) && All(abs(Muon_pfRelIso04_all)<0.40)",
                   "Require good isolation")
           .Define("Electron_ip3d_el", "sqrt(Electron_dxy*Electron_dxy + Electron_dz*Electron_dz)")
           .Define("Electron_sip3d_el",
                   "Electron_ip3d_el/sqrt(Electron_dxyErr*Electron_dxyErr + Electron_dzErr*Electron_dzErr)")
           .Filter("All(Electron_sip3d_el<4) && All(abs(Electron_dxy)<0.5) && All(abs(Electron_dz)<1.0)",
                   "Electron track close to primary vertex with small uncertainty")
           .Define("Muon_ip3d_mu", "sqrt(Muon_dxy*Muon_dxy + Muon_dz*Muon_dz)")
           .Define("Muon_sip3d_mu",
                   "Muon_ip3d_mu/sqrt(Muon_dxyErr*Muon_dxyErr + Muon_dzErr*Muon_dzErr)")
           .Filter("All(Muon_sip3d_mu<4) && All(abs(Muon_dxy)<0.5) && All(abs(Muon_dz)<1.0)",
                   "Muon track close to primary vertex with small uncertainty")
           .Filter("Sum(Electron_charge)==0 && Sum(Muon_charge)==0",
                   "Two opposite charged electron and muon pairs");
}

RNode reco_higgs_to_4mu_node(RNode df) {
  auto df_z_mass =
      selection_4mu_node(df)
          .Define("Z_idx", "reco_zz_to_4l(Muon_pt, Muon_eta, Muon_phi, Muon_mass, Muon_charge)")
          .Filter("filter_z_dr(Z_idx, Muon_eta, Muon_phi)", "Delta R separation of muons building Z system")
          .Define("Z_mass", "compute_z_masses_4l(Z_idx, Muon_pt, Muon_eta, Muon_phi, Muon_mass)");
  return filter_z_candidates_node(df_z_mass)
      .Define("H_mass", "compute_higgs_mass_4l(Z_idx, Muon_pt, Muon_eta, Muon_phi, Muon_mass)");
}

RNode reco_higgs_to_4el_node(RNode df) {
  auto df_z_mass =
      selection_4el_node(df)
          .Define("Z_idx", "reco_zz_to_4l(Electron_pt, Electron_eta, Electron_phi, Electron_mass, Electron_charge)")
          .Filter("filter_z_dr(Z_idx, Electron_eta, Electron_phi)",
                  "Delta R separation of electrons building Z system")
          .Define("Z_mass",
                  "compute_z_masses_4l(Z_idx, Electron_pt, Electron_eta, Electron_phi, Electron_mass)");
  return filter_z_candidates_node(df_z_mass)
      .Define("H_mass", "compute_higgs_mass_4l(Z_idx, Electron_pt, Electron_eta, Electron_phi, Electron_mass)");
}

RNode reco_higgs_to_2el2mu_node(RNode df) {
  auto df_z_mass =
      selection_2el2mu_node(df).Define(
          "Z_mass",
          "compute_z_masses_2el2mu(Electron_pt, Electron_eta, Electron_phi, Electron_mass, "
          "Muon_pt, Muon_eta, Muon_phi, Muon_mass)");
  return filter_z_candidates_node(df_z_mass)
      .Define("H_mass",
              "compute_higgs_mass_2el2mu(Electron_pt, Electron_eta, Electron_phi, Electron_mass, "
              "Muon_pt, Muon_eta, Muon_phi, Muon_mass)");
}

ROOT::RDF::RResultPtr<TH1D> weighted_hist(RNode df, const char *name, double weight) {
  std::ostringstream weight_expr;
  weight_expr << std::setprecision(17) << weight;
  return df.Define("weight", weight_expr.str())
      .Histo1D(ROOT::RDF::TH1DModel(name, "", NBINS, MASS_MIN, MASS_MAX), "H_mass", "weight");
}

void print_table(TH1D &signal, TH1D &background, TH1D &data) {
  std::cout << "luminosity_pb: " << LUMINOSITY << "\\n";
  std::cout << "histogram: nbins=" << NBINS << " range=[" << MASS_MIN << "," << MASS_MAX << "]\\n";
  std::cout << "sample_weights:\\n";
  std::cout << std::fixed << std::setprecision(12);
  std::cout << "  SMHiggsToZZTo4L.root=" << (LUMINOSITY * 0.0065 / 299973.0) << "\\n";
  std::cout << "  ZZTo4mu.root=" << (LUMINOSITY * 0.077 * SCALE_ZZ_TO_4L / 1499064.0) << "\\n";
  std::cout << "  ZZTo4e.root=" << (LUMINOSITY * 0.077 * SCALE_ZZ_TO_4L / 1499093.0) << "\\n";
  std::cout << "  ZZTo2e2mu.root=" << (LUMINOSITY * 0.18 * SCALE_ZZ_TO_4L / 1497445.0) << "\\n";
  std::cout << "bin,low,high,signal,background,data\\n";
  for (int index = 1; index <= signal.GetNbinsX(); ++index) {
    std::cout << index << ","
              << std::setprecision(9) << signal.GetXaxis()->GetBinLowEdge(index) << ","
              << signal.GetXaxis()->GetBinUpEdge(index) << ","
              << std::setprecision(12) << signal.GetBinContent(index) << ","
              << background.GetBinContent(index) << ","
              << std::setprecision(0) << data.GetBinContent(index) << "\\n";
  }
  std::cout << std::setprecision(12)
            << "totals,signal=" << signal.Integral()
            << ",background=" << background.Integral()
            << ",data=" << std::setprecision(0) << data.Integral()
            << std::setprecision(12) << ",mc=" << (signal.Integral() + background.Integral()) << "\\n";
}

void higgs4l_stack_root_crosscheck(const char *base_url, const char *local_dir) {
  const std::string base(base_url);
  const std::string local(local_dir);

  ROOT::EnableImplicitMT();

  ROOT::RDataFrame df_sig("Events", source_path(base, local, "SMHiggsToZZTo4L.root"));
  ROOT::RDataFrame df_bkg_4mu("Events", source_path(base, local, "ZZTo4mu.root"));
  ROOT::RDataFrame df_bkg_4el("Events", source_path(base, local, "ZZTo4e.root"));
  ROOT::RDataFrame df_bkg_2el2mu("Events", source_path(base, local, "ZZTo2e2mu.root"));
  ROOT::RDataFrame df_data_doublemu("Events", std::vector<std::string>{
      source_path(base, local, "Run2012B_DoubleMuParked.root"),
      source_path(base, local, "Run2012C_DoubleMuParked.root")});
  ROOT::RDataFrame df_data_doubleel("Events", std::vector<std::string>{
      source_path(base, local, "Run2012B_DoubleElectron.root"),
      source_path(base, local, "Run2012C_DoubleElectron.root")});

  const auto weight_sig = LUMINOSITY * 0.0065 / 299973.0;
  const auto weight_bkg_4mu = LUMINOSITY * 0.077 * SCALE_ZZ_TO_4L / 1499064.0;
  const auto weight_bkg_4el = LUMINOSITY * 0.077 * SCALE_ZZ_TO_4L / 1499093.0;
  const auto weight_bkg_2el2mu = LUMINOSITY * 0.18 * SCALE_ZZ_TO_4L / 1497445.0;

  auto h_sig_4mu = weighted_hist(reco_higgs_to_4mu_node(RNode(df_sig)), "h_sig_4mu", weight_sig);
  auto h_sig_4el = weighted_hist(reco_higgs_to_4el_node(RNode(df_sig)), "h_sig_4el", weight_sig);
  auto h_sig_2el2mu = weighted_hist(reco_higgs_to_2el2mu_node(RNode(df_sig)), "h_sig_2el2mu", weight_sig);

  auto h_bkg_4mu = weighted_hist(reco_higgs_to_4mu_node(RNode(df_bkg_4mu)), "h_bkg_4mu", weight_bkg_4mu);
  auto h_bkg_4el = weighted_hist(reco_higgs_to_4el_node(RNode(df_bkg_4el)), "h_bkg_4el", weight_bkg_4el);
  auto h_bkg_2el2mu =
      weighted_hist(reco_higgs_to_2el2mu_node(RNode(df_bkg_2el2mu)), "h_bkg_2el2mu", weight_bkg_2el2mu);

  auto h_data_4mu = weighted_hist(reco_higgs_to_4mu_node(RNode(df_data_doublemu)), "h_data_4mu", 1.0);
  auto h_data_4el = weighted_hist(reco_higgs_to_4el_node(RNode(df_data_doubleel)), "h_data_4el", 1.0);
  auto h_data_2el2mu = weighted_hist(reco_higgs_to_2el2mu_node(RNode(df_data_doublemu)), "h_data_2el2mu", 1.0);

  TH1D signal(*h_sig_4mu.GetPtr());
  signal.SetName("signal");
  signal.Add(h_sig_4el.GetPtr());
  signal.Add(h_sig_2el2mu.GetPtr());

  TH1D background(*h_bkg_4mu.GetPtr());
  background.SetName("background");
  background.Add(h_bkg_4el.GetPtr());
  background.Add(h_bkg_2el2mu.GetPtr());

  TH1D data(*h_data_4mu.GetPtr());
  data.SetName("data");
  data.Add(h_data_4el.GetPtr());
  data.Add(h_data_2el2mu.GetPtr());

  print_table(signal, background, data);
}
ROOT_MACRO

root -l -b -q "${macro}(\"${BASE_URL}\", \"${LOCAL_DIR}\")"
