#pragma once

#include "nano/core/Event.h"
#include "nano/core/OutputModel.h"
#include "nano/core/Collection.h"

#include <string>
#include <vector>

namespace nano {

struct ProducerConfig {
  std::string year = "2024";
  std::string channel;
  std::string jet_type = "ak8";
  std::string nano_version = "V15";
};

class HeavyFlavBaseProducer {
public:
  explicit HeavyFlavBaseProducer(ProducerConfig config);
  virtual ~HeavyFlavBaseProducer() = default;

  virtual void begin_file();
  virtual bool analyze(Event &event) = 0;

  const OutputModel &output() const { return out_; }
  static std::vector<BranchSpec> default_schema();

protected:
  void select_leptons(Event &event) const;
  void correct_jets_and_met(Event &event) const;
  void load_gen_history(Event &event, std::vector<ObjectView> &fatjets) const;
  void eval_tagger(Event &event, std::vector<ObjectView> &jets) const;
  void eval_mass_regression(Event &event, std::vector<ObjectView> &jets) const;
  void fill_base_event_info(Event &event);
  void fill_fatjet_info(Event &event, const std::vector<ObjectView> &fatjets);

  ProducerConfig config_;
  float deepjet_wp_m_ = 0.1272f;
  float jet_cone_size_ = 0.8f;
  std::string fatjet_name_ = "FatJet";
  std::string subjet_name_ = "SubJet";
  std::string genfatjet_name_ = "GenJetAK8";
  OutputModel out_;
};

}  // namespace nano
