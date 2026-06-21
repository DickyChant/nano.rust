#!/usr/bin/env bash
set -euo pipefail

# Local-only ROOT cross-check for the df103 Higgs -> ZZ -> 4 lepton analysis.
# Requires a local ROOT installation with the `root` executable on PATH.

URL="${1:-root://eospublic.cern.ch//eos/root-eos/cms_opendata_2012_nanoaod_skimmed/SMHiggsToZZTo4L.root}"
if [[ "${1:-}" == "--dump-selected" ]]; then
  DUMP_PATH="${2:?missing dump path after --dump-selected}"
  URL="${3:-root://eospublic.cern.ch//eos/root-eos/cms_opendata_2012_nanoaod_skimmed/SMHiggsToZZTo4L.root}"
elif [[ "${2:-}" == "--dump-selected" ]]; then
  DUMP_PATH="${3:?missing dump path after --dump-selected}"
else
  DUMP_PATH="${NANO_HIGGS4L_DUMP:-}"
fi
HEADER="${ROOT_HIGGS4L_HEADER:-/usr/share/doc/root/tutorials/analysis/dataframe/df103_NanoAODHiggsAnalysis_python.h}"

if ! command -v root >/dev/null 2>&1; then
  echo "error: ROOT is required for this local cross-check; install ROOT and ensure 'root' is on PATH" >&2
  exit 127
fi

if [[ ! -r "$HEADER" ]]; then
  echo "error: ROOT df103 helper header not found at $HEADER" >&2
  echo "       set ROOT_HIGGS4L_HEADER=/path/to/df103_NanoAODHiggsAnalysis_python.h" >&2
  exit 2
fi

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/nano-higgs4l-root.XXXXXX")"
macro="${tmpdir}/higgs4l_root_crosscheck.C"
trap 'rm -rf "$tmpdir"' EXIT

cat >"$macro" <<ROOT_MACRO
#include <ROOT/RDataFrame.hxx>
#include <TSystem.h>
#include <TH1D.h>

#include <cstring>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <stdexcept>

#include "$HEADER"

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

void dump_selected_channel(RNode df, const char *channel, std::ofstream &out) {
  auto runs = df.Take<unsigned int>("run");
  auto lumis = df.Take<unsigned int>("luminosityBlock");
  auto events = df.Take<unsigned long long>("event");
  auto h_masses = df.Take<float>("H_mass");
  auto z_masses = df.Take<RVecF>("Z_mass");

  const auto &run_values = *runs;
  const auto &lumi_values = *lumis;
  const auto &event_values = *events;
  const auto &h_mass_values = *h_masses;
  const auto &z_mass_values = *z_masses;
  for (size_t i = 0; i < h_mass_values.size(); ++i) {
    out << run_values[i] << "," << lumi_values[i] << "," << event_values[i] << ","
        << channel << "," << std::fixed << std::setprecision(9)
        << h_mass_values[i] << "," << z_mass_values[i][0] << "," << z_mass_values[i][1] << "\\n";
  }
}

void dump_selected(RNode four_mu, RNode four_el, RNode two_el_two_mu, const char *dump_path) {
  std::ofstream out(dump_path);
  if (!out) {
    throw std::runtime_error(std::string("could not open selected dump: ") + dump_path);
  }
  out << "run,luminosityBlock,event,channel,H_mass,Z1_mass,Z2_mass\\n";
  dump_selected_channel(four_mu, "4mu", out);
  dump_selected_channel(four_el, "4e", out);
  dump_selected_channel(two_el_two_mu, "2e2mu", out);
}

void higgs4l_root_crosscheck(const char *url, const char *dump_path = "") {
  ROOT::RDataFrame base("Events", url);
  RNode df(base);
  auto four_mu = reco_higgs_to_4mu_node(df);
  auto four_el = reco_higgs_to_4el_node(df);
  auto two_el_two_mu = reco_higgs_to_2el2mu_node(df);

  auto count_4mu = four_mu.Count();
  auto count_4e = four_el.Count();
  auto count_2e2mu = two_el_two_mu.Count();
  auto hist_4mu = four_mu.Histo1D({"h4mu", "H_mass;H_mass [GeV];Events", 11, 70.0, 180.0}, "H_mass");
  auto hist_4e = four_el.Histo1D({"h4e", "H_mass;H_mass [GeV];Events", 11, 70.0, 180.0}, "H_mass");
  auto hist_2e2mu =
      two_el_two_mu.Histo1D({"h2e2mu", "H_mass;H_mass [GeV];Events", 11, 70.0, 180.0}, "H_mass");

  TH1D total_hist(*hist_4mu);
  total_hist.SetName("h_mass");
  total_hist.Add(hist_4e.GetPtr());
  total_hist.Add(hist_2e2mu.GetPtr());

  const auto peak_bin = total_hist.GetMaximumBin();
  const auto peak_low = total_hist.GetXaxis()->GetBinLowEdge(peak_bin);
  const auto peak_high = total_hist.GetXaxis()->GetBinUpEdge(peak_bin);

  std::cout << "source: " << url << "\\n";
  std::cout << "selected_4mu: " << *count_4mu << "\\n";
  std::cout << "selected_4e: " << *count_4e << "\\n";
  std::cout << "selected_2e2mu: " << *count_2e2mu << "\\n";
  std::cout << "total_selected: " << (*count_4mu + *count_4e + *count_2e2mu) << "\\n";
  std::cout << "h_mass_histogram_gev:\\n";
  for (int index = 1; index <= total_hist.GetNbinsX(); ++index) {
    std::cout << total_hist.GetXaxis()->GetBinLowEdge(index) << "-"
              << total_hist.GetXaxis()->GetBinUpEdge(index) << " "
              << static_cast<long long>(total_hist.GetBinContent(index)) << "\\n";
  }
  std::cout << "peak_bin_gev: " << peak_low << "-" << peak_high << "\\n";
  if (std::strlen(dump_path) != 0) {
    dump_selected(four_mu, four_el, two_el_two_mu, dump_path);
    std::cout << "selected_dump: " << dump_path << "\\n";
  }
}
ROOT_MACRO

root -l -b -q "${macro}(\"${URL}\", \"${DUMP_PATH}\")"
