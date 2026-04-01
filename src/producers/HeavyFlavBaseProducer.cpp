#include "nano/producers/HeavyFlavBaseProducer.h"

#include "nano/core/Collection.h"
#include "nano/core/Helpers.h"

#include <algorithm>
#include <cmath>
#include <stdexcept>

namespace nano {

namespace {

template <typename T>
std::vector<ObjectView> filter_objects(const std::vector<ObjectView> &in, T &&predicate) {
  std::vector<ObjectView> out;
  for (const auto &obj : in) {
    if (predicate(obj)) {
      out.push_back(obj);
    }
  }
  return out;
}

std::vector<ObjectView> sort_by_pt(std::vector<ObjectView> objects) {
  std::sort(objects.begin(), objects.end(), [](const auto &a, const auto &b) { return a.pt() > b.pt(); });
  return objects;
}

}  // namespace

HeavyFlavBaseProducer::HeavyFlavBaseProducer(ProducerConfig config) : config_(std::move(config)) {}

void HeavyFlavBaseProducer::begin_file() {
  // This is the minimal output contract needed by the muon producer path.
  out_.branch("year", 2024.0f);
  out_.branch("lumiwgt", 108.96f);
  out_.branch("jetR", jet_cone_size_);
  out_.branch("passmetfilters", false);
  out_.branch("l1PreFiringWeight", 1.0f);
  out_.branch("l1PreFiringWeightUp", 1.0f);
  out_.branch("l1PreFiringWeightDown", 1.0f);
  out_.branch("nlep", std::int32_t{0});
  out_.branch("ht", 0.0f);
  out_.branch("met", 0.0f);
  out_.branch("metphi", 0.0f);
  out_.branch("jetVetoFlag", false);

  for (const auto *name : {"fj_1_is_qualified", "fj_1_pt", "fj_1_eta", "fj_1_phi", "fj_1_rawmass", "fj_1_sdmass",
                           "fj_1_sdmass_raw", "fj_1_regressed_mass", "fj_1_tau1", "fj_1_tau2", "fj_1_tau3",
                           "fj_1_tau4", "fj_1_deltaR_sj12", "fj_1_sj1_pt", "fj_1_sj1_eta", "fj_1_sj1_phi",
                           "fj_1_sj1_rawmass", "fj_1_sj1_btagdeepcsv", "fj_1_sj2_pt", "fj_1_sj2_eta", "fj_1_sj2_phi",
                           "fj_1_sj2_rawmass", "fj_1_sj2_btagdeepcsv"}) {
    out_.branch(name, 0.0f);
  }
}

std::vector<BranchSpec> HeavyFlavBaseProducer::default_schema() {
  // Keep the schema close to producer usage. Expanding this list should follow
  // actual producer dependencies rather than mirroring the whole NanoAOD file.
  return {
      {"run", BranchType::kUInt32},
      {"event", BranchType::kUInt64},
      {"genWeight", BranchType::kFloat},
      {"Flag_goodVertices", BranchType::kBool},
      {"Flag_globalSuperTightHalo2016Filter", BranchType::kBool},
      {"Flag_EcalDeadCellTriggerPrimitiveFilter", BranchType::kBool},
      {"Flag_BadPFMuonFilter", BranchType::kBool},
      {"Flag_BadPFMuonDzFilter", BranchType::kBool},
      {"Flag_eeBadScFilter", BranchType::kBool},
      {"Flag_ecalBadCalibFilter", BranchType::kBool},
      {"Flag_hfNoisyHitsFilter", BranchType::kBool},
      {"HLT_Mu50", BranchType::kBool},
      {"PuppiMET_pt", BranchType::kFloat},
      {"PuppiMET_phi", BranchType::kFloat},
      {"Muon_pt", BranchType::kVecFloat},
      {"Muon_eta", BranchType::kVecFloat},
      {"Muon_phi", BranchType::kVecFloat},
      {"Muon_mass", BranchType::kVecFloat},
      {"Muon_dxy", BranchType::kVecFloat},
      {"Muon_dz", BranchType::kVecFloat},
      {"Muon_tightId", BranchType::kVecBool},
      {"Muon_looseId", BranchType::kVecBool},
      {"Muon_miniPFRelIso_all", BranchType::kVecFloat},
      {"Electron_pt", BranchType::kVecFloat},
      {"Electron_eta", BranchType::kVecFloat},
      {"Electron_phi", BranchType::kVecFloat},
      {"Electron_mass", BranchType::kVecFloat},
      {"Electron_dxy", BranchType::kVecFloat},
      {"Electron_dz", BranchType::kVecFloat},
      {"Electron_deltaEtaSC", BranchType::kVecFloat},
      {"Electron_mvaNoIso_WP90", BranchType::kVecBool},
      {"Electron_miniPFRelIso_all", BranchType::kVecFloat},
      {"Jet_pt", BranchType::kVecFloat},
      {"Jet_eta", BranchType::kVecFloat},
      {"Jet_phi", BranchType::kVecFloat},
      {"Jet_mass", BranchType::kVecFloat},
      {"Jet_btagDeepFlavB", BranchType::kVecFloat},
      {"Jet_neHEF", BranchType::kVecFloat},
      {"Jet_neEmEF", BranchType::kVecFloat},
      {"Jet_chHEF", BranchType::kVecFloat},
      {"Jet_muEF", BranchType::kVecFloat},
      {"Jet_chEmEF", BranchType::kVecFloat},
      {"FatJet_pt", BranchType::kVecFloat},
      {"FatJet_eta", BranchType::kVecFloat},
      {"FatJet_phi", BranchType::kVecFloat},
      {"FatJet_mass", BranchType::kVecFloat},
      {"FatJet_tau1", BranchType::kVecFloat},
      {"FatJet_tau2", BranchType::kVecFloat},
      {"FatJet_tau3", BranchType::kVecFloat},
      {"FatJet_tau4", BranchType::kVecFloat},
      {"FatJet_subJetIdx1", BranchType::kVecInt16},
      {"FatJet_subJetIdx2", BranchType::kVecInt16},
      {"SubJet_pt", BranchType::kVecFloat},
      {"SubJet_eta", BranchType::kVecFloat},
      {"SubJet_phi", BranchType::kVecFloat},
      {"SubJet_mass", BranchType::kVecFloat},
      {"SubJet_btagDeepB", BranchType::kVecFloat},
  };
}

void HeavyFlavBaseProducer::select_leptons(Event &event) const {
  // looseLeptons mirrors the Python producer contract and is later used for
  // jet-lepton cleaning and event-level lepton counting.
  auto electrons = event.collection("Electron").objects();
  std::vector<ObjectView> loose_leptons;
  for (auto &el : electrons) {
    if (el.pt() > 10.0f && std::abs(el.eta()) < 2.5f && std::abs(el.get<float>("dxy")) < 0.05f &&
        std::abs(el.get<float>("dz")) < 0.2f && el.get<bool>("mvaNoIso_WP90") && el.get<float>("miniPFRelIso_all") < 0.4f) {
      loose_leptons.push_back(el);
    }
  }

  auto muons = event.collection("Muon").objects();
  for (auto &mu : muons) {
    if (mu.pt() > 10.0f && std::abs(mu.eta()) < 2.4f && std::abs(mu.get<float>("dxy")) < 0.05f &&
        std::abs(mu.get<float>("dz")) < 0.2f && mu.get<bool>("looseId") && mu.get<float>("miniPFRelIso_all") < 0.4f) {
      loose_leptons.push_back(mu);
    }
  }

  event.set("looseLeptons", sort_by_pt(std::move(loose_leptons)));
}

void HeavyFlavBaseProducer::correct_jets_and_met(Event &event) const {
  // This prototype only implements the structural part of the Python method:
  // object materialization, subjet linking, lepton cleaning, and simple MET/HT.
  auto fatjets = event.collection(fatjet_name_).objects();
  auto subjets = event.collection(subjet_name_).objects();
  auto ak4jets = event.collection("Jet").objects();

  fatjets = filter_objects(sort_by_pt(std::move(fatjets)), [](const auto &fj) {
    return fj.pt() > 200.0f && std::abs(fj.eta()) < 2.4f;
  });
  ak4jets = filter_objects(sort_by_pt(std::move(ak4jets)), [](const auto &j) {
    return j.pt() > 25.0f && std::abs(j.eta()) < 2.4f;
  });

  const auto &loose = event.get<std::vector<ObjectView>>("looseLeptons");
  auto clean_fatjets = std::vector<ObjectView>{};
  for (auto &fj : fatjets) {
    auto [idx, dr] = closest_index(fj, loose);
    if (idx < 0 || dr >= jet_cone_size_) {
      std::vector<ObjectView> linked_subjets;
      for (const auto attr : {"subJetIdx1", "subJetIdx2"}) {
        const auto sj_idx = fj.get<std::int32_t>(attr);
        if (sj_idx >= 0 && static_cast<std::size_t>(sj_idx) < subjets.size()) {
          linked_subjets.push_back(subjets[sj_idx]);
        }
      }
      // Derived jet content is attached back onto the view so later producer
      // steps can consume it like regular object state.
      fj.set("subjets", linked_subjets);
      float sdmass = 0.0f;
      if (linked_subjets.size() == 2U) {
        sdmass = (linked_subjets[0].p4() + linked_subjets[1].p4()).M();
      }
      fj.set("msoftdrop", sdmass);
      fj.set("msoftdrop_raw", sdmass);
      fj.set("is_qualified", true);
      clean_fatjets.push_back(fj);
    }
  }

  auto clean_ak4jets = std::vector<ObjectView>{};
  for (auto &j : ak4jets) {
    auto [idx, dr] = closest_index(j, loose);
    if (idx < 0 || dr >= 0.4f) {
      clean_ak4jets.push_back(j);
    }
  }

  float ht = 0.0f;
  for (const auto &jet : clean_ak4jets) {
    ht += jet.pt();
  }

  event.set("fatjets", clean_fatjets);
  event.set("ak4jets", clean_ak4jets);
  event.set("met_pt", event.scalar<float>("PuppiMET_pt"));
  event.set("met_phi", event.scalar<float>("PuppiMET_phi"));
  event.set("ht", ht);
}

void HeavyFlavBaseProducer::load_gen_history(Event &, std::vector<ObjectView> &fatjets) const {
  // Placeholder for the full Python gen-history logic. Keeping the interface in
  // place lets producers call it now without blocking later implementation.
  for (auto &fj : fatjets) {
    fj.set("dr_H", 99.0f);
    fj.set("dr_Z", 99.0f);
    fj.set("dr_W", 99.0f);
    fj.set("dr_T", 99.0f);
  }
}

void HeavyFlavBaseProducer::eval_tagger(Event &, std::vector<ObjectView> &jets) const {
  // Stub values preserve the producer flow until inference services are added.
  for (auto &jet : jets) {
    jet.set("pn_Xbb", 0.0f);
    jet.set("pn_Xcc", 0.0f);
    jet.set("pn_Xqq", 0.0f);
    jet.set("pn_QCD", 0.0f);
  }
}

void HeavyFlavBaseProducer::eval_mass_regression(Event &, std::vector<ObjectView> &jets) const {
  // Until an explicit regression model is added, use the reconstructed soft-drop mass.
  for (auto &jet : jets) {
    jet.set("regressed_mass", jet.get<float>("msoftdrop"));
  }
}

void HeavyFlavBaseProducer::fill_base_event_info(Event &event) {
  // Reset once per accepted event so missing fills fall back to declared defaults.
  out_.reset();
  out_.fill("jetR", jet_cone_size_);
  out_.fill("year", 2024.0f);
  out_.fill("lumiwgt", 108.96f);
  out_.fill("passmetfilters", event.scalar<bool>("Flag_goodVertices") && event.scalar<bool>("Flag_globalSuperTightHalo2016Filter") &&
                                  event.scalar<bool>("Flag_EcalDeadCellTriggerPrimitiveFilter") &&
                                  event.scalar<bool>("Flag_BadPFMuonFilter") && event.scalar<bool>("Flag_BadPFMuonDzFilter") &&
                                  event.scalar<bool>("Flag_eeBadScFilter") && event.scalar<bool>("Flag_ecalBadCalibFilter") &&
                                  event.scalar<bool>("Flag_hfNoisyHitsFilter"));
  out_.fill("l1PreFiringWeight", 1.0f);
  out_.fill("l1PreFiringWeightUp", 1.0f);
  out_.fill("l1PreFiringWeightDown", 1.0f);
  out_.fill("nlep", static_cast<std::int32_t>(event.get<std::vector<ObjectView>>("looseLeptons").size()));
  out_.fill("ht", event.get<float>("ht"));
  out_.fill("met", event.get<float>("met_pt"));
  out_.fill("metphi", event.get<float>("met_phi"));
  out_.fill("jetVetoFlag", false);
}

void HeavyFlavBaseProducer::fill_fatjet_info(Event &, const std::vector<ObjectView> &fatjets) {
  if (fatjets.empty()) {
    return;
  }
  const auto &fj = fatjets.front();
  // The current scope only needs the leading probe jet for the muon channel.
  out_.fill("fj_1_is_qualified", fj.get<bool>("is_qualified"));
  out_.fill("fj_1_pt", fj.pt());
  out_.fill("fj_1_eta", fj.eta());
  out_.fill("fj_1_phi", fj.phi());
  out_.fill("fj_1_rawmass", fj.mass());
  out_.fill("fj_1_sdmass", fj.get<float>("msoftdrop"));
  out_.fill("fj_1_sdmass_raw", fj.get<float>("msoftdrop_raw"));
  out_.fill("fj_1_regressed_mass", fj.get<float>("regressed_mass"));
  out_.fill("fj_1_tau1", fj.get<float>("tau1"));
  out_.fill("fj_1_tau2", fj.get<float>("tau2"));
  out_.fill("fj_1_tau3", fj.get<float>("tau3"));
  out_.fill("fj_1_tau4", fj.get<float>("tau4"));

  const auto subjets = fj.extra<std::vector<ObjectView>>("subjets");
  if (!subjets.empty()) {
    out_.fill("fj_1_sj1_pt", subjets[0].pt());
    out_.fill("fj_1_sj1_eta", subjets[0].eta());
    out_.fill("fj_1_sj1_phi", subjets[0].phi());
    out_.fill("fj_1_sj1_rawmass", subjets[0].mass());
    out_.fill("fj_1_sj1_btagdeepcsv", subjets[0].get<float>("btagDeepB"));
  }
  if (subjets.size() > 1U) {
    out_.fill("fj_1_deltaR_sj12", delta_r(subjets[0], subjets[1]));
    out_.fill("fj_1_sj2_pt", subjets[1].pt());
    out_.fill("fj_1_sj2_eta", subjets[1].eta());
    out_.fill("fj_1_sj2_phi", subjets[1].phi());
    out_.fill("fj_1_sj2_rawmass", subjets[1].mass());
    out_.fill("fj_1_sj2_btagdeepcsv", subjets[1].get<float>("btagDeepB"));
  }
}

}  // namespace nano
