#pragma once

#include "nano/core/Event.h"
#include "nano/producers/HeavyFlavBaseProducer.h"

#include <cstdint>
#include <map>
#include <memory>
#include <string>
#include <utility>

class FatJetVariationsCalculator;
class JetVariationsCalculator;
class Type1METVariationsCalculator;

namespace nano {

class JetMETCorrector {
public:
  explicit JetMETCorrector(const ProducerConfig &config);
  ~JetMETCorrector();

  void correct_event(Event &event) const;

private:
  struct PayloadPaths {
    std::string payload_dir;
    std::string jet_jerc_json;
    std::string fatjet_jerc_json;
    std::string jer_smear_json;
  };

  struct EraSetup {
    std::string jet_jec_tag_mc;
    std::string fatjet_jec_tag_mc;
    std::string jer_tag_mc;
    std::vector<std::pair<std::uint32_t, std::string>> data_tags;
    std::string payload_subdir;
    std::string met_xy_corr_era;
  };

  struct CalculatorBundle {
    std::unique_ptr<JetVariationsCalculator> ak4_jets;
    std::unique_ptr<FatJetVariationsCalculator> fatjet_jets;
    std::unique_ptr<JetVariationsCalculator> subjets;
    std::unique_ptr<Type1METVariationsCalculator> met;
  };

  static EraSetup build_era_setup(const ProducerConfig &config);
  static PayloadPaths resolve_payload_paths(const ProducerConfig &config, const EraSetup &setup);

  CalculatorBundle make_bundle(const std::string &jec_tag, bool is_mc) const;
  const CalculatorBundle &bundle_for_event(const Event &event) const;

  ProducerConfig config_;
  EraSetup era_setup_;
  PayloadPaths payload_paths_;
  std::unique_ptr<CalculatorBundle> mc_bundle_;
  std::map<std::string, std::unique_ptr<CalculatorBundle>> data_bundles_;
};

}  // namespace nano
