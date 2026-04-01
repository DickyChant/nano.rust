#include "nano/producers/HeavyFlavMuonSampleProducer.h"

#include "nano/core/Collection.h"
#include "nano/core/Helpers.h"

#include <algorithm>
#include <cmath>

namespace nano {

HeavyFlavMuonSampleProducer::HeavyFlavMuonSampleProducer(ProducerConfig config)
    : HeavyFlavBaseProducer([&config] {
        config.channel = "muon";
        return config;
      }()) {}

void HeavyFlavMuonSampleProducer::begin_file() {
  HeavyFlavBaseProducer::begin_file();
  out_.branch("passMuTrig", false);
  out_.branch("muon_pt", 0.0f);
  out_.branch("muon_eta", 0.0f);
  out_.branch("muon_miniIso", 0.0f);
  out_.branch("leptonicW_pt", 0.0f);
}

bool HeavyFlavMuonSampleProducer::analyze(Event &event) {
  // This follows the Python MuonSampleProducer selection order closely so the
  // later full port can stay behaviorally aligned.
  auto muons = event.collection("Muon").objects();
  std::vector<ObjectView> selected_muons;
  for (auto &mu : muons) {
    if (mu.pt() > 55.0f && std::abs(mu.eta()) < 2.4f && std::abs(mu.get<float>("dxy")) < 0.2f &&
        std::abs(mu.get<float>("dz")) < 0.5f && mu.get<bool>("tightId") && mu.get<float>("miniPFRelIso_all") < 0.10f) {
      selected_muons.push_back(mu);
    }
  }
  if (selected_muons.size() != 1U) {
    return false;
  }
  event.set("muons", selected_muons);

  select_leptons(event);
  correct_jets_and_met(event);

  if (event.get<float>("met_pt") < 50.0f) {
    return false;
  }

  auto mu = selected_muons.front();
  event.set("mu", mu);
  const auto leptonic_w = polar_p4(mu) + met_p4(event.get<float>("met_pt"), event.get<float>("met_phi"));
  event.set("leptonicW", leptonic_w);
  if (leptonic_w.Pt() < 100.0f) {
    return false;
  }

  const auto ak4jets = event.get<std::vector<ObjectView>>("ak4jets");
  std::vector<ObjectView> bjets;
  for (auto &jet : ak4jets) {
    if (jet.get<float>("btagDeepFlavB") > deepjet_wp_m_ && std::abs(delta_phi(jet, mu)) < 2.0f) {
      bjets.push_back(jet);
    }
  }
  if (bjets.empty()) {
    return false;
  }
  event.set("bjets", bjets);

  const auto fatjets = event.get<std::vector<ObjectView>>("fatjets");
  std::vector<ObjectView> probe_jets;
  for (auto &fj : fatjets) {
    if (std::abs(delta_phi(fj, mu)) > 2.0f) {
      probe_jets.push_back(fj);
    }
  }
  if (probe_jets.empty()) {
    return false;
  }
  // The muon channel keeps only the leading probe jet in the current workflow.
  probe_jets.erase(probe_jets.begin() + 1, probe_jets.end());

  load_gen_history(event, probe_jets);
  eval_tagger(event, probe_jets);
  eval_mass_regression(event, probe_jets);
  fill_base_event_info(event);
  fill_fatjet_info(event, probe_jets);

  out_.fill("passMuTrig", event.scalar<bool>("HLT_Mu50"));
  out_.fill("muon_pt", mu.pt());
  out_.fill("muon_eta", mu.eta());
  out_.fill("muon_miniIso", mu.get<float>("miniPFRelIso_all"));
  out_.fill("leptonicW_pt", static_cast<float>(leptonic_w.Pt()));
  return true;
}

}  // namespace nano
