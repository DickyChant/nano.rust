#include "nano/producers/HeavyFlavBaseProducer.h"

#include "nano/core/Collection.h"
#include "nano/core/Helpers.h"
#include "nano/helpers/FatjetGenMatching.h"
#include "nano/helpers/JetMETCorrector.h"
#include "nano/helpers/PuWeightProducer.h"
#include "nano/helpers/TopPtWeightProducer.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string_view>

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

bool pass_v15_jet_id(const ObjectView &jet, bool tight_lep_veto) {
  // Jet ID definition: https://twiki.cern.ch/twiki/bin/viewauth/CMS/JetID13p6TeV#nanoAOD_Flags
  const auto abs_eta = std::abs(jet.eta());
  bool pass_tight = false;
  if (abs_eta <= 2.6f) {
    pass_tight = safe_object_float(jet, "neHEF", 1.0f) < 0.99f && safe_object_float(jet, "neEmEF", 1.0f) < 0.9f &&
                 safe_object_int(jet, "chMultiplicity", 0) + safe_object_int(jet, "neMultiplicity", 0) > 1 &&
                 safe_object_float(jet, "chHEF", 0.0f) > 0.01f && safe_object_int(jet, "chMultiplicity", 0) > 0;
  } else if (abs_eta <= 2.7f) {
    pass_tight = safe_object_float(jet, "neHEF", 1.0f) < 0.9f && safe_object_float(jet, "neEmEF", 1.0f) < 0.99f;
  } else if (abs_eta <= 3.0f) {
    pass_tight = safe_object_float(jet, "neHEF", 1.0f) < 0.99f;
  } else {
    pass_tight = safe_object_int(jet, "neMultiplicity", 0) >= 2 && safe_object_float(jet, "neEmEF", 1.0f) < 0.4f;
  }

  if (!tight_lep_veto || abs_eta > 2.7f) {
    return pass_tight;
  }
  return pass_tight && safe_object_float(jet, "muEF", 0.0f) < 0.8f && safe_object_float(jet, "chEmEF", 0.0f) < 0.8f;
}

std::vector<ObjectView> subjets_for(const ObjectView &fatjet, Event &event, std::string_view subjet_name) {
  const auto subjets = event.collection(subjet_name).objects();
  std::vector<ObjectView> out;
  const auto idx1 = safe_object_int(fatjet, "subJetIdx1", -1);
  const auto idx2 = safe_object_int(fatjet, "subJetIdx2", -1);
  if (idx1 >= 0 && static_cast<std::size_t>(idx1) < subjets.size()) {
    out.push_back(subjets[static_cast<std::size_t>(idx1)]);
  }
  if (idx2 >= 0 && static_cast<std::size_t>(idx2) < subjets.size()) {
    out.push_back(subjets[static_cast<std::size_t>(idx2)]);
  }
  return out;
}

}  // namespace

HeavyFlavBaseProducer::HeavyFlavBaseProducer(ProducerConfig config) : config_(std::move(config)) {
  jme_corrector_ = std::make_unique<JetMETCorrector>(config_);
  pu_weight_producer_ = std::make_unique<PuWeightProducer>(config_);
  top_pt_weight_producer_ = std::make_unique<TopPtWeightProducer>();
  fatjet_gen_matching_ = std::make_unique<FatjetGenMatching>();
}

HeavyFlavBaseProducer::~HeavyFlavBaseProducer() = default;

void HeavyFlavBaseProducer::begin_file() {
  out_.branch("run", std::uint32_t{0});
  out_.branch("luminosityBlock", std::uint32_t{0});
  out_.branch("event", std::uint64_t{0});
  out_.branch("year", 0.0f);
  out_.branch("lumiwgt", 0.0f);
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
  out_.branch("genWeight", 1.0f);
  out_.branch("LHEScaleWeight", std::vector<float>{});
  pu_weight_producer_->begin_file(out_);
  top_pt_weight_producer_->begin_file(out_);

  out_.branch("fj_1_is_qualified", false);
  for (const auto *name : {"fj_1_pt", "fj_1_eta", "fj_1_phi", "fj_1_mass", "fj_1_sdmass",
                           "fj_1_tau1", "fj_1_tau2", "fj_1_tau3",
                           "fj_1_tau4", "fj_1_deltaR_sj12", "fj_1_sj1_pt", "fj_1_sj1_eta", "fj_1_sj1_phi",
                           "fj_1_sj1_mass", "fj_1_sj1_btagdeepcsv", "fj_1_sj2_pt", "fj_1_sj2_eta", "fj_1_sj2_phi",
                           "fj_1_sj2_mass", "fj_1_sj2_btagdeepcsv"}) {
    out_.branch(name, 0.0f);
  }
  for (const auto &tagger : config_.tagger_names) {
    out_.branch("fj_1_" + tagger, -99.0f);
  }

  for (const auto *name : {"fj_1_genfj_nbhadrons",   "fj_1_genfj_nchadrons",   "fj_1_genfj_partonflavour",
                           "fj_1_nbhadrons",         "fj_1_nchadrons",         "fj_1_partonflavour",
                           "fj_1_sj1_nbhadrons",     "fj_1_sj1_nchadrons",     "fj_1_sj1_partonflavour",
                           "fj_1_sj2_nbhadrons",     "fj_1_sj2_nchadrons",     "fj_1_sj2_partonflavour",
                           "fj_1_H_decay",           "fj_1_Z_decay",           "fj_1_W_decay",
                           "fj_1_T_Wq_max_pdgId",    "fj_1_T_Wq_min_pdgId"}) {
    out_.branch(name, std::int32_t{0});
  }

  for (const auto *name : {"fj_1_dr_H",       "fj_1_dr_H_daus",  "fj_1_H_pt",         "fj_1_dr_Z",      "fj_1_dr_Z_daus",
                           "fj_1_Z_pt",       "fj_1_dr_W",       "fj_1_dr_W_daus",    "fj_1_W_pt",      "fj_1_dr_T",
                           "fj_1_dr_T_b",     "fj_1_dr_T_Wq_max","fj_1_dr_T_Wq_min",  "fj_1_T_pt"}) {
    out_.branch(name, 0.0f);
  }
}

std::vector<BranchSpec> HeavyFlavBaseProducer::default_schema(const ProducerConfig &config) {
  std::vector<BranchSpec> specs{
      {"run", BranchType::kUInt32},
      {"luminosityBlock", BranchType::kUInt32},
      {"event", BranchType::kUInt64},
      {"genWeight", BranchType::kFloat, true},
      {"Pileup_nTrueInt", BranchType::kFloat, true},
      {"Flag_goodVertices", BranchType::kBool},
      {"Flag_globalSuperTightHalo2016Filter", BranchType::kBool},
      {"Flag_EcalDeadCellTriggerPrimitiveFilter", BranchType::kBool},
      {"Flag_BadPFMuonFilter", BranchType::kBool},
      {"Flag_BadPFMuonDzFilter", BranchType::kBool},
      {"Flag_eeBadScFilter", BranchType::kBool},
      {"Flag_ecalBadCalibFilter", BranchType::kBool, true},
      {"Flag_hfNoisyHitsFilter", BranchType::kBool, true},
      {"Flag_HBHENoiseFilter", BranchType::kBool, true},
      {"Flag_HBHENoiseIsoFilter", BranchType::kBool, true},
      {"PuppiMET_pt", BranchType::kFloat},
      {"PuppiMET_phi", BranchType::kFloat},
      {"RawPuppiMET_pt", BranchType::kFloat},
      {"RawPuppiMET_phi", BranchType::kFloat},
      {"MET_MetUnclustEnUpDeltaX", BranchType::kFloat, true},
      {"MET_MetUnclustEnUpDeltaY", BranchType::kFloat, true},
      {"Rho_fixedGridRhoFastjetAll", BranchType::kFloat},
      {"nLHEScaleWeight", BranchType::kInt32, true},
      {"LHEScaleWeight", BranchType::kVecFloat, true},
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
      {"Jet_rawFactor", BranchType::kVecFloat},
      {"Jet_area", BranchType::kVecFloat},
      {"Jet_muonSubtrFactor", BranchType::kVecFloat},
      {"Jet_btagDeepFlavB", BranchType::kVecFloat, true},
      {"Jet_btagPNetB", BranchType::kVecFloat, true},
      {"Jet_btagUParTAK4B", BranchType::kVecFloat, true},
      {"Jet_neHEF", BranchType::kVecFloat},
      {"Jet_neEmEF", BranchType::kVecFloat},
      {"Jet_chHEF", BranchType::kVecFloat},
      {"Jet_muEF", BranchType::kVecFloat},
      {"Jet_chEmEF", BranchType::kVecFloat},
      {"Jet_chMultiplicity", BranchType::kVecUInt8},
      {"Jet_neMultiplicity", BranchType::kVecUInt8},
      {"Jet_jetId", BranchType::kVecUInt8, true},
      {"Jet_partonFlavour", BranchType::kVecInt16, true},
      {"Jet_genJetIdx", BranchType::kVecInt16, true},
      {"CorrT1METJet_rawPt", BranchType::kVecFloat},
      {"CorrT1METJet_eta", BranchType::kVecFloat},
      {"CorrT1METJet_phi", BranchType::kVecFloat},
      {"CorrT1METJet_area", BranchType::kVecFloat},
      {"CorrT1METJet_muonSubtrFactor", BranchType::kVecFloat},
      {"FatJet_pt", BranchType::kVecFloat},
      {"FatJet_eta", BranchType::kVecFloat},
      {"FatJet_phi", BranchType::kVecFloat},
      {"FatJet_mass", BranchType::kVecFloat},
      {"FatJet_rawFactor", BranchType::kVecFloat},
      {"FatJet_area", BranchType::kVecFloat},
      {"FatJet_msoftdrop", BranchType::kVecFloat},
      {"FatJet_neHEF", BranchType::kVecFloat},
      {"FatJet_neEmEF", BranchType::kVecFloat},
      {"FatJet_chHEF", BranchType::kVecFloat},
      {"FatJet_muEF", BranchType::kVecFloat},
      {"FatJet_chEmEF", BranchType::kVecFloat},
      {"FatJet_chMultiplicity", BranchType::kVecInt16},
      {"FatJet_neMultiplicity", BranchType::kVecInt16},
      {"FatJet_jetId", BranchType::kVecUInt8, true},
      {"FatJet_genJetAK8Idx", BranchType::kVecInt16, true},
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
      {"SubJet_rawFactor", BranchType::kVecFloat},
      {"SubJet_btagDeepB", BranchType::kVecFloat, true},
      {"SubJet_nBHadrons", BranchType::kVecUInt8, true},
      {"SubJet_nCHadrons", BranchType::kVecUInt8, true},
      {"SubJet_partonFlavour", BranchType::kVecInt16, true},
      {"GenJet_pt", BranchType::kVecFloat},
      {"GenJet_eta", BranchType::kVecFloat},
      {"GenJet_phi", BranchType::kVecFloat},
      {"GenJet_mass", BranchType::kVecFloat},
      {"GenJetAK8_pt", BranchType::kVecFloat},
      {"GenJetAK8_eta", BranchType::kVecFloat},
      {"GenJetAK8_phi", BranchType::kVecFloat},
      {"GenJetAK8_mass", BranchType::kVecFloat},
      {"GenJetAK8_nBHadrons", BranchType::kVecUInt8, true},
      {"GenJetAK8_nCHadrons", BranchType::kVecUInt8, true},
      {"GenJetAK8_partonFlavour", BranchType::kVecInt16, true},
      {"SubGenJetAK8_pt", BranchType::kVecFloat},
      {"SubGenJetAK8_eta", BranchType::kVecFloat},
      {"SubGenJetAK8_phi", BranchType::kVecFloat},
      {"SubGenJetAK8_mass", BranchType::kVecFloat},
      {"GenPart_pt", BranchType::kVecFloat, true},
      {"GenPart_eta", BranchType::kVecFloat, true},
      {"GenPart_phi", BranchType::kVecFloat, true},
      {"GenPart_mass", BranchType::kVecFloat, true},
      {"GenPart_pdgId", BranchType::kVecInt32, true},
      {"GenPart_status", BranchType::kVecInt32, true},
      {"GenPart_statusFlags", BranchType::kVecUInt16, true},
      {"GenPart_genPartIdxMother", BranchType::kVecInt16, true},
  };

  for (const auto &tagger : config.tagger_names) {
    specs.push_back({"FatJet_" + tagger, BranchType::kVecFloat, true});
  }
  for (const auto &trigger : config.required_triggers) {
    specs.push_back({trigger, BranchType::kBool, true});
  }
  return specs;
}

void HeavyFlavBaseProducer::select_leptons(Event &event) const {
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
  jme_corrector_->correct_event(event);

  auto fatjets = event.collection(fatjet_name_).objects();
  auto ak4jets = event.collection("Jet").objects();

  for (auto &fj : fatjets) {
    fj.set("idx", static_cast<std::int32_t>(fj.index()));
    fj.set("is_qualified", true);
    const auto subjets = subjets_for(fj, event, subjet_name_);
    fj.set("subjets", subjets);
    LorentzVector subjet_sum;
    for (const auto &sj : subjets) {
      subjet_sum += sj.p4();
    }
    fj.set("msoftdrop", static_cast<float>(subjet_sum.M()));
  }

  fatjets = filter_objects(sort_by_pt(std::move(fatjets)), [&](const auto &fj) {
    return fj.pt() > 200.0f && std::abs(fj.eta()) < 2.4f && pass_v15_jet_id(fj, false);
  });
  ak4jets = filter_objects(sort_by_pt(std::move(ak4jets)), [&](const auto &jet) {
    return jet.pt() > 25.0f && std::abs(jet.eta()) < 2.4f && pass_v15_jet_id(jet, true); // tight_lep_veto: true
  });

  const auto &loose = event.get<std::vector<ObjectView>>("looseLeptons");
  auto clean_fatjets = std::vector<ObjectView>{};
  for (auto &fj : fatjets) {
    auto [idx, dr] = closest_index(fj, loose);
    if (idx < 0 || dr >= jet_cone_size_) {
      clean_fatjets.push_back(fj);
    }
  }

  auto clean_ak4jets = std::vector<ObjectView>{};
  for (auto &jet : ak4jets) {
    auto [idx, dr] = closest_index(jet, loose);
    if (idx < 0 || dr >= 0.4f) {
      clean_ak4jets.push_back(jet);
    }
  }

  float ht = 0.0f;
  for (const auto &jet : clean_ak4jets) {
    ht += jet.pt();
  }

  event.set("fatjets", clean_fatjets);
  event.set("ak4jets", clean_ak4jets);
  event.set("ht", ht);
}

void HeavyFlavBaseProducer::load_gen_history(Event &event, std::vector<ObjectView> &fatjets) const {
  fatjet_gen_matching_->process(event, fatjets);
}

void HeavyFlavBaseProducer::fill_base_event_info(Event &event) {
  out_.reset();
  out_.fill("run", event.scalar<std::uint32_t>("run"));
  out_.fill("luminosityBlock", event.scalar<std::uint32_t>("luminosityBlock"));
  out_.fill("event", event.scalar<std::uint64_t>("event"));
  out_.fill("jetR", jet_cone_size_);
  out_.fill("year", config_.year_value);
  out_.fill("lumiwgt", config_.lumi_weight);

  bool met_filters = safe_bool(event, "Flag_goodVertices") && safe_bool(event, "Flag_globalSuperTightHalo2016Filter") &&
                     safe_bool(event, "Flag_EcalDeadCellTriggerPrimitiveFilter") && safe_bool(event, "Flag_BadPFMuonFilter") &&
                     safe_bool(event, "Flag_BadPFMuonDzFilter") && safe_bool(event, "Flag_eeBadScFilter");
  if (config_.era == "2016APV" || config_.era == "2016" || config_.era == "2017" || config_.era == "2018") {
    met_filters = met_filters && safe_bool(event, "Flag_HBHENoiseFilter") && safe_bool(event, "Flag_HBHENoiseIsoFilter");
  }
  if (config_.era == "2017" || config_.era == "2018" || config_.era == "2022" || config_.era == "2022EE" || config_.era == "2023" ||
      config_.era == "2023BPix" || config_.era == "2024") {
    met_filters = met_filters && safe_bool(event, "Flag_ecalBadCalibFilter");
  }
  if (config_.era == "2022" || config_.era == "2022EE" || config_.era == "2023" || config_.era == "2023BPix" || config_.era == "2024") {
    met_filters = met_filters && safe_bool(event, "Flag_hfNoisyHitsFilter");
  }
  out_.fill("passmetfilters", met_filters);
  out_.fill("l1PreFiringWeight", 1.0f);
  out_.fill("l1PreFiringWeightUp", 1.0f);
  out_.fill("l1PreFiringWeightDown", 1.0f);
  out_.fill("nlep", static_cast<std::int32_t>(event.get<std::vector<ObjectView>>("looseLeptons").size()));
  out_.fill("ht", event.get<float>("ht"));
  out_.fill("met", event.get<float>("met_pt"));
  out_.fill("metphi", event.get<float>("met_phi"));
  out_.fill("jetVetoFlag", false);
  out_.fill("genWeight", event.is_mc() ? event.scalar<float>("genWeight") : 1.0f);
  out_.fill("LHEScaleWeight", event.vector<float>("LHEScaleWeight"));
  pu_weight_producer_->fill(event, out_);
  top_pt_weight_producer_->fill(event, out_);
}

void HeavyFlavBaseProducer::fill_fatjet_info(Event &event, const std::vector<ObjectView> &fatjets) {
  if (fatjets.empty()) {
    return;
  }
  const auto &fj = fatjets.front();
  out_.fill("fj_1_is_qualified", fj.get<bool>("is_qualified"));
  out_.fill("fj_1_pt", fj.pt());
  out_.fill("fj_1_eta", fj.eta());
  out_.fill("fj_1_phi", fj.phi());
  out_.fill("fj_1_mass", fj.mass());
  out_.fill("fj_1_sdmass", fj.get<float>("msoftdrop"));
  out_.fill("fj_1_tau1", safe_object_float(fj, "tau1", 0.0f));
  out_.fill("fj_1_tau2", safe_object_float(fj, "tau2", 0.0f));
  out_.fill("fj_1_tau3", safe_object_float(fj, "tau3", 0.0f));
  out_.fill("fj_1_tau4", safe_object_float(fj, "tau4", 0.0f));
  for (const auto &tagger : config_.tagger_names) {
    out_.fill("fj_1_" + tagger, safe_object_float(fj, tagger, -99.0f));
  }

  const auto subjets = fj.extra<std::vector<ObjectView>>("subjets");
  if (!subjets.empty()) {
    out_.fill("fj_1_sj1_pt", subjets[0].pt());
    out_.fill("fj_1_sj1_eta", subjets[0].eta());
    out_.fill("fj_1_sj1_phi", subjets[0].phi());
    out_.fill("fj_1_sj1_mass", subjets[0].mass());
    out_.fill("fj_1_sj1_btagdeepcsv", safe_object_float(subjets[0], "btagDeepB", -1.0f));
    out_.fill("fj_1_sj1_nbhadrons", safe_object_int(subjets[0], "nBHadrons", -1));
    out_.fill("fj_1_sj1_nchadrons", safe_object_int(subjets[0], "nCHadrons", -1));
    out_.fill("fj_1_sj1_partonflavour", safe_object_int(subjets[0], "partonFlavour", -1));
  }
  if (subjets.size() > 1U) {
    out_.fill("fj_1_deltaR_sj12", delta_r(subjets[0], subjets[1]));
    out_.fill("fj_1_sj2_pt", subjets[1].pt());
    out_.fill("fj_1_sj2_eta", subjets[1].eta());
    out_.fill("fj_1_sj2_phi", subjets[1].phi());
    out_.fill("fj_1_sj2_mass", subjets[1].mass());
    out_.fill("fj_1_sj2_btagdeepcsv", safe_object_float(subjets[1], "btagDeepB", -1.0f));
    out_.fill("fj_1_sj2_nbhadrons", safe_object_int(subjets[1], "nBHadrons", -1));
    out_.fill("fj_1_sj2_nchadrons", safe_object_int(subjets[1], "nCHadrons", -1));
    out_.fill("fj_1_sj2_partonflavour", safe_object_int(subjets[1], "partonFlavour", -1));
  } else {
    out_.fill("fj_1_deltaR_sj12", 99.0f);
  }

  const auto gen_fatjets = event.collection(genfatjet_name_).objects();
  const auto gen_idx = safe_object_int(fj, "genJetAK8Idx", -1);
  if (gen_idx >= 0 && static_cast<std::size_t>(gen_idx) < gen_fatjets.size()) {
    const auto &gen_fj = gen_fatjets[static_cast<std::size_t>(gen_idx)];
    out_.fill("fj_1_genfj_nbhadrons", safe_object_int(gen_fj, "nBHadrons", -1));
    out_.fill("fj_1_genfj_nchadrons", safe_object_int(gen_fj, "nCHadrons", -1));
    out_.fill("fj_1_genfj_partonflavour", safe_object_int(gen_fj, "partonFlavour", -1));
  } else {
    out_.fill("fj_1_genfj_nbhadrons", std::int32_t{-1});
    out_.fill("fj_1_genfj_nchadrons", std::int32_t{-1});
    out_.fill("fj_1_genfj_partonflavour", std::int32_t{-1});
  }

  out_.fill("fj_1_nbhadrons", safe_object_int(fj, "nBHadrons", -1));
  out_.fill("fj_1_nchadrons", safe_object_int(fj, "nCHadrons", -1));
  out_.fill("fj_1_partonflavour", safe_object_int(fj, "partonFlavour", -1));
  out_.fill("fj_1_dr_H", safe_object_float(fj, "dr_H", 99.0f));
  out_.fill("fj_1_dr_H_daus", safe_object_float(fj, "dr_H_daus", 99.0f));
  out_.fill("fj_1_H_pt", safe_object_float(fj, "H_pt", -1.0f));
  out_.fill("fj_1_H_decay", safe_object_int(fj, "H_decay", 0));
  out_.fill("fj_1_dr_Z", safe_object_float(fj, "dr_Z", 99.0f));
  out_.fill("fj_1_dr_Z_daus", safe_object_float(fj, "dr_Z_daus", 99.0f));
  out_.fill("fj_1_Z_pt", safe_object_float(fj, "Z_pt", -1.0f));
  out_.fill("fj_1_Z_decay", safe_object_int(fj, "Z_decay", 0));
  out_.fill("fj_1_dr_W", safe_object_float(fj, "dr_W", 99.0f));
  out_.fill("fj_1_dr_W_daus", safe_object_float(fj, "dr_W_daus", 99.0f));
  out_.fill("fj_1_W_pt", safe_object_float(fj, "W_pt", -1.0f));
  out_.fill("fj_1_W_decay", safe_object_int(fj, "W_decay", 0));
  out_.fill("fj_1_dr_T", safe_object_float(fj, "dr_T", 99.0f));
  out_.fill("fj_1_dr_T_b", safe_object_float(fj, "dr_T_b", 99.0f));
  out_.fill("fj_1_dr_T_Wq_max", safe_object_float(fj, "dr_T_Wq_max", 99.0f));
  out_.fill("fj_1_dr_T_Wq_min", safe_object_float(fj, "dr_T_Wq_min", 99.0f));
  out_.fill("fj_1_T_Wq_max_pdgId", safe_object_int(fj, "T_Wq_max_pdgId", 0));
  out_.fill("fj_1_T_Wq_min_pdgId", safe_object_int(fj, "T_Wq_min_pdgId", 0));
  out_.fill("fj_1_T_pt", safe_object_float(fj, "T_pt", -1.0f));
}

}  // namespace nano
