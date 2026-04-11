#include "nano/core/Event.h"
#include "nano/io/NanoReader.h"
#include "nano/producers/HeavyFlavMuonSampleProducer.h"
#include "runtime_common.h"

#include <TDirectory.h>
#include <TEntryList.h>
#include <TFile.h>
#include <TTree.h>

#include <cmath>
#include <exception>
#include <iostream>
#include <memory>
#include <string>
#include <vector>

namespace {

nano::ProducerConfig make_config(const YAML::Node &settings) {
  nano::ProducerConfig config;
  config.channel = "muon";
  config.era = settings["era"].as<std::string>();
  config.nano_version = settings["nano_version"].as<std::string>();
  config.selection = settings["selections"]["muon"].as<std::string>();
  config.required_triggers = nano::runtime::yaml_string_list(settings, "required_triggers");
  const auto tagger_node = settings["tagger_names"][config.nano_version][config.era];
  if (!tagger_node) {
    throw std::runtime_error("Missing tagger_names config for muon smoke test");
  }
  for (const auto &item : tagger_node) {
    config.tagger_names.push_back(item.as<std::string>());
  }

  const auto btag_node = settings["btag"][config.nano_version][config.era];
  if (!btag_node) {
    throw std::runtime_error("Missing btag config for muon smoke test");
  }
  config.btag_config.branch = btag_node["branch"].as<std::string>();
  config.btag_config.loose = btag_node["loose"] ? btag_node["loose"].as<float>() : 0.0f;
  config.btag_config.medium = btag_node["medium"] ? btag_node["medium"].as<float>() : 0.0f;
  config.btag_config.tight = btag_node["tight"] ? btag_node["tight"].as<float>() : 0.0f;
  config.btag_config.xtight = btag_node["xtight"] ? btag_node["xtight"].as<float>() : 0.0f;
  config.btag_config.xxtight = btag_node["xxtight"] ? btag_node["xxtight"].as<float>() : 0.0f;
  config.year_value = settings["year_values"][config.era].as<float>();
  config.lumi_weight = settings["lumi_values"][config.era].as<float>();

  config.jme_payload_dir = settings["jec"]["payload_dir"].as<std::string>();
  config.jme_jer_smear_json = settings["jec"]["jer_smear_json"].as<std::string>();
  for (const auto &item : settings["jec"]["eras"]) {
    nano::JmeEraConfig era_cfg;
    era_cfg.payload_prefix = item.second["payload_prefix"].as<std::string>();
    era_cfg.jet_jec_tag_mc = item.second["jet_jec_tag_mc"].as<std::string>();
    era_cfg.fatjet_jec_tag_mc = item.second["fatjet_jec_tag_mc"].as<std::string>();
    era_cfg.jer_tag_mc = item.second["jer_tag_mc"].as<std::string>();
    era_cfg.met_xy_corr_era = item.second["met_xy_corr_era"].as<std::string>();
    if (item.second["data_tags"]) {
      for (const auto &tag : item.second["data_tags"]) {
        era_cfg.data_tags.push_back({tag["run_start"].as<std::uint32_t>(), tag["tag"].as<std::string>()});
      }
    }
    config.jme_eras[item.first.as<std::string>()] = era_cfg;
  }

  config.pu_payload_dir = settings["pu"]["payload_dir"].as<std::string>();
  for (const auto &item : settings["pu"]["eras"]) {
    config.pu_eras[item.first.as<std::string>()] = {
        item.second["payload_subdir"].as<std::string>(),
        item.second["correction_key"].as<std::string>(),
    };
  }
  return config;
}

std::vector<Long64_t> first_preselected_entries(TTree &tree, Long64_t raw_cap) {
  tree.Draw(">>elist_muon_smoke",
            "(Sum$(Muon_pt>55 && abs(Muon_eta)<2.4 && Muon_tightId && Muon_miniPFRelIso_all<0.10)>0 && nFatJet>0) && (Entry$<10000)",
            "entrylist");
  auto *elist = dynamic_cast<TEntryList *>(gDirectory->Get("elist_muon_smoke"));
  std::vector<Long64_t> out;
  if (!elist) {
    return out;
  }
  const auto n = std::min<Long64_t>(elist->GetN(), raw_cap);
  out.reserve(static_cast<std::size_t>(n));
  for (Long64_t i = 0; i < n; ++i) {
    out.push_back(elist->GetEntry(i));
  }
  return out;
}

}  // namespace

int main(int argc, char **argv) {
  if (argc < 3) {
    std::cerr << "Usage: muon_smoke_test <file.root> <tree-name>\n";
    return 2;
  }

  try {
    auto input = std::unique_ptr<TFile>(TFile::Open(argv[1], "READ"));
    if (!input || input->IsZombie()) {
      throw std::runtime_error("Failed to open input file");
    }
    auto *tree = dynamic_cast<TTree *>(input->Get(argv[2]));
    if (!tree) {
      throw std::runtime_error("Missing Events tree");
    }

    const auto settings = nano::runtime::load_config_with_extends(std::string(NANOAODTOOLS_SOURCE_DIR) + "/configs/muon_2024.yaml");
    const auto config = make_config(settings);
    nano::NanoReader reader(argv[2], argv[1], nano::BranchSchema(nano::HeavyFlavBaseProducer::default_schema(config)));

    nano::HeavyFlavMuonSampleProducer producer(config);
    producer.begin_file();

    const auto entries = first_preselected_entries(*tree, 10000);
    std::size_t accepted = 0;
    std::uint64_t first_event = 0;
    for (const auto entry : entries) {
      nano::Event event(reader, static_cast<std::size_t>(entry));
      if (accepted == 0) {
        first_event = event.scalar<std::uint64_t>("event");
      }
      if (producer.analyze(event)) {
        ++accepted;
      }
    }

    std::cout << "processed=" << entries.size() << " accepted=" << accepted << " first_event=" << first_event << "\n";
    if (accepted < 79 || accepted > 81) {
      std::cerr << "Unexpected accepted count: " << accepted << " (expected reference-compatible range 79-81)\n";
      return 1;
    }
    return 0;
  } catch (const std::exception &ex) {
    std::cerr << "muon_smoke_test failed: " << ex.what() << "\n";
    return 1;
  }
}
