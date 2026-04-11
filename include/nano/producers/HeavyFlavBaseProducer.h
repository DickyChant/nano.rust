#pragma once

#include "nano/core/Event.h"
#include "nano/core/OutputModel.h"
#include "nano/core/Collection.h"

#include <memory>
#include <string>
#include <unordered_map>
#include <vector>

namespace nano {

class JetMETCorrector;
class PuWeightProducer;
class TopPtWeightProducer;
class FatjetGenMatching;

struct JmeDataTagConfig {
  std::uint32_t run_start = 0;
  std::string tag;
};

struct JmeEraConfig {
  std::string payload_prefix;
  std::string jet_jec_tag_mc;
  std::string fatjet_jec_tag_mc;
  std::string jer_tag_mc;
  std::string met_xy_corr_era;
  std::vector<JmeDataTagConfig> data_tags;
};

struct PuEraConfig {
  std::string payload_subdir;
  std::string correction_key;
};

struct BTagConfig {
  std::string branch;
  float loose = 0.0f;
  float medium = 0.0f;
  float tight = 0.0f;
  float xtight = 0.0f;
  float xxtight = 0.0f;
};

struct ProducerConfig {
  std::string era = "2024";
  std::string channel;
  std::string nano_version = "V15";
  std::string selection;
  std::vector<std::string> required_triggers;
  std::vector<std::string> tagger_names;
  BTagConfig btag_config;
  float year_value = 0.0f;
  float lumi_weight = 1.0f;
  std::string jme_payload_dir;
  std::string jme_jer_smear_json;
  std::unordered_map<std::string, JmeEraConfig> jme_eras;
  std::string pu_payload_dir;
  std::unordered_map<std::string, PuEraConfig> pu_eras;
};

class HeavyFlavBaseProducer {
public:
  explicit HeavyFlavBaseProducer(ProducerConfig config);
  virtual ~HeavyFlavBaseProducer();

  virtual void begin_file();
  virtual bool analyze(Event &event) = 0;

  OutputModel &output() { return out_; }
  const OutputModel &output() const { return out_; }
  static std::vector<BranchSpec> default_schema(const ProducerConfig &config);

protected:
  void select_leptons(Event &event) const;
  void correct_jets_and_met(Event &event) const;
  void load_gen_history(Event &event, std::vector<ObjectView> &fatjets) const;
  void fill_base_event_info(Event &event);
  void fill_fatjet_info(Event &event, const std::vector<ObjectView> &fatjets);

  ProducerConfig config_;
  float jet_cone_size_ = 0.8f;
  std::string fatjet_name_ = "FatJet";
  std::string subjet_name_ = "SubJet";
  std::string genfatjet_name_ = "GenJetAK8";
  std::unique_ptr<JetMETCorrector> jme_corrector_;
  std::unique_ptr<PuWeightProducer> pu_weight_producer_;
  std::unique_ptr<TopPtWeightProducer> top_pt_weight_producer_;
  std::unique_ptr<FatjetGenMatching> fatjet_gen_matching_;
  OutputModel out_;
};

}  // namespace nano
