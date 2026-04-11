#include "nano/helpers/JetMETCorrector.h"

#include "nano/core/Collection.h"
#include "nano/core/Helpers.h"

#include "FatJetVariationsCalculator.h"
#include "JetVariationsCalculator.h"
#include "Type1METVariationsCalculator.h"

#include <ROOT/RVec.hxx>

#include <algorithm>
#include <cmath>
#include <limits>
#include <stdexcept>
#include <string_view>
#include <utility>

namespace nano {

namespace {

using RVecF = ROOT::VecOps::RVec<float>;
using RVecI = ROOT::VecOps::RVec<int>;

template <typename T>
ROOT::VecOps::RVec<T> to_rvec(const std::vector<T> &values) {
  return ROOT::VecOps::RVec<T>(values.begin(), values.end());
}

RVecF zeros_f(std::size_t n) {
  return RVecF(n, 0.f);
}

RVecI zeros_i(std::size_t n, int value = 0) {
  return RVecI(n, value);
}

RVecI to_rvec_int16(const std::vector<std::int16_t> &values) {
  RVecI out(values.size(), 0);
  for (std::size_t i = 0; i < values.size(); ++i) {
    out[i] = static_cast<int>(values[i]);
  }
  return out;
}

RVecI to_rvec_uint8(const std::vector<std::uint8_t> &values) {
  RVecI out(values.size(), 0);
  for (std::size_t i = 0; i < values.size(); ++i) {
    out[i] = static_cast<int>(values[i]);
  }
  return out;
}

float get_scalar_or(Event &event, std::string_view name, float fallback) {
  const auto *info = event.schema().find(name);
  return info ? event.scalar<float>(name) : fallback;
}

std::uint32_t get_run_number(const Event &event) {
  return event.scalar<std::uint32_t>("run");
}

int rnd_seed(const Event &event, const std::vector<ObjectView> &jets, int extra = 0) {
  const auto run = static_cast<int>(event.scalar<std::uint32_t>("run"));
  const auto lumi = static_cast<int>(event.scalar<std::uint32_t>("luminosityBlock"));
  const auto event_number = static_cast<int>(event.scalar<std::uint64_t>("event") & 0x7fffffffULL);
  int seed = (run << 20) + (lumi << 10) + event_number + extra;
  if (!jets.empty()) {
    seed += static_cast<int>(jets.front().eta() / 0.01f);
  }
  return seed;
}

std::string join_path(std::string base, std::string_view tail) {
  if (!base.empty() && base.back() == '/') {
    base.pop_back();
  }
  return base + "/" + std::string(tail);
}

std::string normalize_nanoaod_version(std::string_view nano_version) {
  if (nano_version.empty()) {
    return "NanoAODv15";
  }
  if (nano_version.rfind("NanoAOD", 0) == 0) {
    return std::string(nano_version);
  }
  if (nano_version[0] == 'V' || nano_version[0] == 'v') {
    return "NanoAODv" + std::string(nano_version.substr(1));
  }
  return "NanoAOD" + std::string(nano_version);
}

std::vector<ObjectView> sort_by_pt(std::vector<ObjectView> objects) {
  std::sort(objects.begin(), objects.end(), [](const auto &a, const auto &b) { return a.pt() > b.pt(); });
  return objects;
}

void apply_nominal_jets(Event &event, const std::string &object_name, const ROOT::VecOps::RVec<float> &pt,
                        const ROOT::VecOps::RVec<float> &mass) {
  auto objects = event.collection(object_name).objects();
  for (std::size_t i = 0; i < objects.size() && i < pt.size(); ++i) {
    auto &obj = objects[i];
    const auto corrected_mass = std::max(0.f, mass[i]);

    obj.set("pt", pt[i]);
    obj.set("mass", corrected_mass);
    obj.set("p4", ObjectView::LorentzVector(pt[i], obj.eta(), obj.phi(), corrected_mass));
  }
}

}  // namespace

JetMETCorrector::JetMETCorrector(const ProducerConfig &config)
    : config_(config), era_setup_(build_era_setup(config)), payload_paths_(resolve_payload_paths(config, era_setup_)) {
  mc_bundle_ = std::make_unique<CalculatorBundle>(make_bundle(era_setup_.jet_jec_tag_mc, true));
  for (const auto &[run_start, tag] : era_setup_.data_tags) {
    static_cast<void>(run_start);
    data_bundles_.emplace(tag, std::make_unique<CalculatorBundle>(make_bundle(tag, false)));
  }
}

JetMETCorrector::~JetMETCorrector() = default;

JetMETCorrector::EraSetup JetMETCorrector::build_era_setup(const ProducerConfig &config) {
  const auto it = config.jme_eras.find(config.era);
  if (it == config.jme_eras.end()) {
    throw std::runtime_error("Unsupported JME era: " + config.era);
  }
  const auto &era_cfg = it->second;
  EraSetup setup;
  setup.payload_subdir = era_cfg.payload_prefix + normalize_nanoaod_version(config.nano_version);
  setup.jet_jec_tag_mc = era_cfg.jet_jec_tag_mc;
  setup.fatjet_jec_tag_mc = era_cfg.fatjet_jec_tag_mc;
  setup.jer_tag_mc = era_cfg.jer_tag_mc;
  setup.met_xy_corr_era = era_cfg.met_xy_corr_era;
  for (const auto &tag : era_cfg.data_tags) {
    setup.data_tags.emplace_back(tag.run_start, tag.tag);
  }
  return setup;
}

JetMETCorrector::PayloadPaths JetMETCorrector::resolve_payload_paths(const ProducerConfig &config, const EraSetup &setup) {
  PayloadPaths out;
  out.payload_dir = join_path(join_path(config.jme_payload_dir, setup.payload_subdir), "latest");
  out.jet_jerc_json = join_path(out.payload_dir, "jet_jerc.json.gz");
  out.fatjet_jerc_json = join_path(out.payload_dir, "fatJet_jerc.json.gz");
  out.jer_smear_json = config.jme_jer_smear_json;
  return out;
}

JetMETCorrector::CalculatorBundle JetMETCorrector::make_bundle(const std::string &jec_tag, bool is_mc) const {
  CalculatorBundle bundle;
  const auto jer_tag = is_mc ? era_setup_.jer_tag_mc : std::string{};

  bundle.ak4_jets = std::make_unique<JetVariationsCalculator>(JetVariationsCalculator::create(
      payload_paths_.jet_jerc_json,  // jsonFile
      "AK4PFPuppi",                  // jetAlgo
      jec_tag,                       // jecTag
      "L1L2L3Res",                   // jecLevel
      std::vector<std::string>{},    // jesUncertainties
      false,                         // addHEM2018Issue
      jer_tag,                       // jerTag
      payload_paths_.jer_smear_json, // jsonFileSmearingTool
      "JERSmear",                    // smearingToolName
      false,                         // splitJER
      true,                          // doGenMatch
      0.2f,                          // genMatch_maxDR
      3.f));                         // genMatch_maxDPT
  bundle.fatjet_jets = std::make_unique<FatJetVariationsCalculator>(FatJetVariationsCalculator::create(
      payload_paths_.fatjet_jerc_json, // jsonFile
      "AK8PFPuppi",                    // jetAlgo
      jec_tag,                         // jecTag
      "L1L2L3Res",                     // jecLevel
      std::vector<std::string>{},      // jesUncertainties
      false,                           // addHEM2018Issue
      jer_tag,                         // jerTag
      payload_paths_.jer_smear_json,   // jsonFileSmearingTool
      "JERSmear",                      // smearingToolName
      false,                           // splitJER
      true,                            // doGenMatch
      0.4f,                            // genMatch_maxDR
      3.f,                             // genMatch_maxDPT
      payload_paths_.jet_jerc_json,    // jsonFileSubjet
      "AK4PFPuppi",                    // jetAlgoSubjet
      jec_tag,                         // jecTagSubjet
      "L1L2L3Res"));                   // jecLevelSubjet
  bundle.subjets = std::make_unique<JetVariationsCalculator>(JetVariationsCalculator::create(
      payload_paths_.jet_jerc_json,  // jsonFile
      "AK4PFPuppi",                  // jetAlgo
      jec_tag,                       // jecTag
      "L1L2L3Res",                   // jecLevel
      std::vector<std::string>{},    // jesUncertainties
      false,                         // addHEM2018Issue
      jer_tag,                       // jerTag
      payload_paths_.jer_smear_json, // jsonFileSmearingTool
      "JERSmear",                    // smearingToolName
      false,                         // splitJER
      true,                          // doGenMatch
      0.2f,                          // genMatch_maxDR
      3.f));                         // genMatch_maxDPT
  bundle.met = std::make_unique<Type1METVariationsCalculator>(Type1METVariationsCalculator::create(
      payload_paths_.jet_jerc_json,  // jsonFile
      "AK4PFPuppi",                  // jetAlgo
      jec_tag,                       // jecTag
      "L1L2L3Res",                   // jecLevel
      "L1FastJet",                   // l1JecTag
      15.f,                          // unclEnThreshold
      0.9f,                          // emEnFracThreshold
      std::vector<std::string>{},    // jesUncertainties
      false,                         // addHEM2018Issue
      true,                          // isT1SmearedMET
      false,                         // isXYCorrMET
      "",                            // jsonXYCorrMET
      era_setup_.met_xy_corr_era,    // eraForXYCorrMET
      is_mc,                         // isMC
      jer_tag,                       // jerTag
      payload_paths_.jer_smear_json, // jsonFileSmearingTool
      "JERSmear",                    // smearingToolName
      false,                         // splitJER
      true,                          // doGenMatch
      0.2f,                          // genMatch_maxDR
      3.f));                         // genMatch_maxDPT
  return bundle;
}

const JetMETCorrector::CalculatorBundle &JetMETCorrector::bundle_for_event(const Event &event) const {
  if (event.is_mc()) {
    return *mc_bundle_;
  }
  const auto run = get_run_number(event);
  std::string selected = era_setup_.data_tags.front().second;
  for (const auto &[start, tag] : era_setup_.data_tags) {
    if (start <= run) {
      selected = tag;
    }
  }
  const auto it = data_bundles_.find(selected);
  if (it == data_bundles_.end()) {
    throw std::runtime_error("Missing JME data bundle for era tag: " + selected);
  }
  return *it->second;
}

void JetMETCorrector::correct_event(Event &event) const {
  const auto &bundle = bundle_for_event(event);
  const auto is_mc = event.is_mc();
  const auto run = static_cast<int>(get_run_number(event));
  const auto rho = get_scalar_or(event, "Rho_fixedGridRhoFastjetAll", get_scalar_or(event, "fixedGridRhoFastjetAll", 0.f));

  auto ak4_jets = event.collection("Jet").objects();
  auto fatjet_jets = event.collection("FatJet").objects();
  auto subjets = event.collection("SubJet").objects();

  const auto jet_pt = to_rvec(event.vector<float>("Jet_pt"));
  const auto jet_eta = to_rvec(event.vector<float>("Jet_eta"));
  const auto jet_phi = to_rvec(event.vector<float>("Jet_phi"));
  const auto jet_mass = to_rvec(event.vector<float>("Jet_mass"));
  const auto jet_raw = to_rvec(event.vector<float>("Jet_rawFactor"));
  const auto jet_area = to_rvec(event.vector<float>("Jet_area"));
  const auto jet_jetid = to_rvec_uint8(event.vector<std::uint8_t>("Jet_jetId"));
  const auto jet_genidx = is_mc ? to_rvec_int16(event.vector<std::int16_t>("Jet_genJetIdx")) : zeros_i(jet_pt.size(), -1);
  const auto jet_parton = is_mc ? to_rvec_int16(event.vector<std::int16_t>("Jet_partonFlavour")) : zeros_i(jet_pt.size(), 0);

  const auto gen_pt = is_mc ? to_rvec(event.vector<float>("GenJet_pt")) : RVecF{};
  const auto gen_eta = is_mc ? to_rvec(event.vector<float>("GenJet_eta")) : RVecF{};
  const auto gen_phi = is_mc ? to_rvec(event.vector<float>("GenJet_phi")) : RVecF{};
  const auto gen_mass = is_mc ? to_rvec(event.vector<float>("GenJet_mass")) : RVecF{};

  const auto jet_seed = rnd_seed(event, ak4_jets);
  const auto jet_result =
      bundle.ak4_jets->produce(jet_pt, jet_eta, jet_phi, jet_mass, jet_raw, jet_area, jet_jetid, rho, jet_genidx, jet_parton,
                               jet_seed, run, gen_pt, gen_eta, gen_phi, gen_mass);

  const auto lowpt_rawpt = to_rvec(event.vector<float>("CorrT1METJet_rawPt"));
  const auto lowpt_eta = to_rvec(event.vector<float>("CorrT1METJet_eta"));
  const auto lowpt_phi = to_rvec(event.vector<float>("CorrT1METJet_phi"));
  const auto lowpt_area = to_rvec(event.vector<float>("CorrT1METJet_area"));
  const auto lowpt_muon = to_rvec(event.vector<float>("CorrT1METJet_muonSubtrFactor"));
  const auto lowpt_zero = zeros_f(lowpt_rawpt.size());
  const auto met_dx = get_scalar_or(event, "MET_MetUnclustEnUpDeltaX", 0.f);
  const auto met_dy = get_scalar_or(event, "MET_MetUnclustEnUpDeltaY", 0.f);

  const auto met_result = bundle.met->produce(
      jet_pt, jet_eta, jet_phi, jet_mass, jet_raw, jet_area, to_rvec(event.vector<float>("Jet_muonSubtrFactor")),
      to_rvec(event.vector<float>("Jet_neEmEF")), to_rvec(event.vector<float>("Jet_chEmEF")), jet_jetid, rho, jet_genidx, jet_parton,
      jet_seed, run, gen_pt, gen_eta, gen_phi, gen_mass, event.scalar<float>("RawPuppiMET_phi"), event.scalar<float>("RawPuppiMET_pt"),
      lowpt_rawpt, lowpt_eta, lowpt_phi, lowpt_area, lowpt_muon, lowpt_zero, lowpt_zero, met_dx, met_dy, static_cast<unsigned char>(0));

  apply_nominal_jets(event, "Jet", jet_result.pt(0), jet_result.mass(0));

  const auto fatjet_pt = to_rvec(event.vector<float>("FatJet_pt"));
  const auto fatjet_eta = to_rvec(event.vector<float>("FatJet_eta"));
  const auto fatjet_phi = to_rvec(event.vector<float>("FatJet_phi"));
  const auto fatjet_mass = to_rvec(event.vector<float>("FatJet_mass"));
  const auto fatjet_raw = to_rvec(event.vector<float>("FatJet_rawFactor"));
  const auto fatjet_area = to_rvec(event.vector<float>("FatJet_area"));
  const auto fatjet_msd = to_rvec(event.vector<float>("FatJet_msoftdrop"));
  const auto fatjet_sj1 = to_rvec_int16(event.vector<std::int16_t>("FatJet_subJetIdx1"));
  const auto fatjet_sj2 = to_rvec_int16(event.vector<std::int16_t>("FatJet_subJetIdx2"));
  const auto fatjet_jetid = to_rvec_uint8(event.vector<std::uint8_t>("FatJet_jetId"));
  const auto fatjet_genidx = is_mc ? to_rvec_int16(event.vector<std::int16_t>("FatJet_genJetAK8Idx")) : zeros_i(fatjet_pt.size(), -1);
  const auto genfatjet_pt = is_mc ? to_rvec(event.vector<float>("GenJetAK8_pt")) : RVecF{};
  const auto genfatjet_eta = is_mc ? to_rvec(event.vector<float>("GenJetAK8_eta")) : RVecF{};
  const auto genfatjet_phi = is_mc ? to_rvec(event.vector<float>("GenJetAK8_phi")) : RVecF{};
  const auto genfatjet_mass = is_mc ? to_rvec(event.vector<float>("GenJetAK8_mass")) : RVecF{};

  const auto subjet_pt = to_rvec(event.vector<float>("SubJet_pt"));
  const auto subjet_eta = to_rvec(event.vector<float>("SubJet_eta"));
  const auto subjet_phi = to_rvec(event.vector<float>("SubJet_phi"));
  const auto subjet_mass = to_rvec(event.vector<float>("SubJet_mass"));
  const auto subjet_raw = to_rvec(event.vector<float>("SubJet_rawFactor"));

  const auto fatjet_seed = rnd_seed(event, fatjet_jets);
  const auto fatjet_result = bundle.fatjet_jets->produce(fatjet_pt, fatjet_eta, fatjet_phi, fatjet_mass, fatjet_raw, fatjet_area,
                                                         fatjet_msd, fatjet_sj1, fatjet_sj2, subjet_pt, subjet_eta, subjet_phi, subjet_mass,
                                                         subjet_raw, fatjet_jetid, rho, fatjet_genidx, fatjet_seed, run, genfatjet_pt,
                                                         genfatjet_eta, genfatjet_phi, genfatjet_mass);
  apply_nominal_jets(event, "FatJet", fatjet_result.pt(0), fatjet_result.mass(0));

  const auto gen_sub_pt = is_mc ? to_rvec(event.vector<float>("SubGenJetAK8_pt")) : RVecF{};
  const auto gen_sub_eta = is_mc ? to_rvec(event.vector<float>("SubGenJetAK8_eta")) : RVecF{};
  const auto gen_sub_phi = is_mc ? to_rvec(event.vector<float>("SubGenJetAK8_phi")) : RVecF{};
  const auto gen_sub_mass = is_mc ? to_rvec(event.vector<float>("SubGenJetAK8_mass")) : RVecF{};
  const auto subjet_seed = rnd_seed(event, subjets);
  const auto subjet_result =
      bundle.subjets->produce(subjet_pt, subjet_eta, subjet_phi, subjet_mass, subjet_raw, zeros_f(subjet_pt.size()), zeros_i(subjet_pt.size(), 0),
                              rho, zeros_i(subjet_pt.size(), -1), zeros_i(subjet_pt.size(), 0), subjet_seed, run, gen_sub_pt, gen_sub_eta,
                              gen_sub_phi, gen_sub_mass);
  apply_nominal_jets(event, "SubJet", subjet_result.pt(0), subjet_result.mass(0));

  fatjet_jets = event.collection("FatJet").objects();
  subjets = event.collection("SubJet").objects();
  for (auto &fj : fatjet_jets) {
    std::vector<ObjectView> linked_subjets;
    for (const auto attr : {"subJetIdx1", "subJetIdx2"}) {
      const auto idx = fj.get<std::int32_t>(attr);
      if (idx >= 0 && static_cast<std::size_t>(idx) < subjets.size()) {
        linked_subjets.push_back(subjets[idx]);
      }
    }
    linked_subjets = sort_by_pt(std::move(linked_subjets));
    fj.set("subjets", linked_subjets);
    fj.set("is_qualified", true);

    auto groomed = LorentzVector();
    for (const auto &sj : linked_subjets) {
      groomed += sj.p4();
    }
    fj.set("msoftdrop", static_cast<float>(std::abs(groomed.M())));
  }

  event.set("met_pt", static_cast<float>(met_result.pt(0)));
  event.set("met_phi", static_cast<float>(met_result.phi(0)));
}

}  // namespace nano
