#include "nano/core/Event.h"
#include "nano/io/NanoReader.h"
#include "nano/io/RootOutputFile.h"
#include "nano/producers/HeavyFlavMuonSampleProducer.h"

#include "runtime_common.h"

#include <filesystem>
#include <iostream>
#include <memory>
#include <unordered_map>
#include <unistd.h>

namespace fs = std::filesystem;

namespace {

struct CliOptions {
  std::string input_files;
  std::string output_file;
  std::string tree_name = "Events";
  long long num_events = -1;
  std::string channel = "muon";
  std::string config_file;
  std::unordered_map<std::string, std::string> overrides;
};

CliOptions parse_args(int argc, char **argv) {
  CliOptions opts;
  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    const auto need_value = [&](const char *name) -> std::string {
      if (i + 1 >= argc) {
        throw std::runtime_error(std::string("Missing value for ") + name);
      }
      return argv[++i];
    };
    if (arg == "--input-files") {
      opts.input_files = need_value("--input-files");
    } else if (arg == "--output-file") {
      opts.output_file = need_value("--output-file");
    } else if (arg == "--tree-name") {
      opts.tree_name = need_value("--tree-name");
    } else if (arg == "--num-events") {
      opts.num_events = std::stoll(need_value("--num-events"));
    } else if (arg == "--channel") {
      opts.channel = need_value("--channel");
    } else if (arg == "--config") {
      opts.config_file = need_value("--config");
    } else if (arg == "--set") {
      const auto kv = need_value("--set");
      const auto pos = kv.find('=');
      if (pos == std::string::npos) {
        throw std::runtime_error("--set expects key=value");
      }
      opts.overrides[kv.substr(0, pos)] = kv.substr(pos + 1);
    } else {
      throw std::runtime_error("Unknown argument: " + arg);
    }
  }

  if (opts.input_files.empty() || opts.output_file.empty() || opts.config_file.empty()) {
    throw std::runtime_error("Usage: nano_run --input-files <files> --output-file <out.root> --config <card.yaml> [--channel muon] [--num-events -1] [--set key=value]");
  }
  return opts;
}

nano::ProducerConfig make_config(const YAML::Node &settings, const std::string &channel) {
  nano::ProducerConfig config;
  config.channel = channel;
  config.era = settings["era"].as<std::string>();
  config.nano_version = settings["nano_version"].as<std::string>();
  config.selection = settings["selections"][channel].as<std::string>();
  config.required_triggers = nano::runtime::yaml_string_list(settings, "required_triggers");
  const auto version_node = settings["tagger_names"][config.nano_version];
  if (!version_node) {
    throw std::runtime_error("Missing tagger_names config for nano version " + config.nano_version);
  }
  const auto year_node = version_node[config.era];
  if (!year_node) {
    throw std::runtime_error("Missing tagger_names config for nano version " + config.nano_version + " and era " + config.era);
  }
  for (const auto &item : year_node) {
    config.tagger_names.push_back(item.as<std::string>());
  }
  const auto btag_node = settings["btag"][config.nano_version][config.era];
  if (!btag_node) {
    throw std::runtime_error("Missing btag config for nano version " + config.nano_version + " and era " + config.era);
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

std::unique_ptr<nano::HeavyFlavBaseProducer> make_producer(const nano::ProducerConfig &config) {
  if (config.channel == "muon") {
    return std::make_unique<nano::HeavyFlavMuonSampleProducer>(config);
  }
  throw std::runtime_error("Unsupported channel: " + config.channel);
}

void process_one_file(const std::string &input_file, const std::string &output_file, const CliOptions &cli,
                      const YAML::Node &settings) {
  auto input = std::unique_ptr<TFile>(TFile::Open(input_file.c_str(), "READ"));
  if (!input || input->IsZombie()) {
    throw std::runtime_error("Failed to open input file: " + input_file);
  }
  auto *tree = dynamic_cast<TTree *>(input->Get(cli.tree_name.c_str()));
  if (!tree) {
    throw std::runtime_error("Missing tree " + cli.tree_name + " in " + input_file);
  }

  const auto config = make_config(settings, cli.channel);
  auto producer = make_producer(config);
  producer->begin_file();

  nano::RootOutputFile output(output_file);
  output.book_events(producer->output());

  nano::NanoReader reader(cli.tree_name, input_file, nano::BranchSchema(nano::HeavyFlavBaseProducer::default_schema(config)));
  const auto entry_list = nano::runtime::build_entry_list(*tree, config.selection, cli.num_events);

  std::size_t accepted = 0;
  for (const auto entry : entry_list) {
    nano::Event event(reader, static_cast<std::size_t>(entry));
    if (!producer->analyze(event)) {
      continue;
    }
    output.fill_event(producer->output());
    ++accepted;
  }

  nano::runtime::copy_selected_tree(*input, output.file(), "Runs");
  nano::runtime::copy_selected_tree(*input, output.file(), "LuminosityBlocks");
  output.write();

  std::cout << "input=" << input_file << " processed=" << entry_list.size() << " accepted=" << accepted << " output=" << output_file << "\n";
}

}  // namespace

int main(int argc, char **argv) {
  try {
    const auto cli = parse_args(argc, argv);
    auto settings = nano::runtime::load_config_with_extends(cli.config_file);
    for (const auto &[key, value] : cli.overrides) {
      nano::runtime::apply_override(settings, key, value);
    }

    auto inputs = nano::runtime::split_csv(cli.input_files);
    for (auto &input : inputs) {
      input = nano::runtime::normalize_input_path(input);
    }

    if (inputs.size() == 1U) {
      process_one_file(inputs.front(), cli.output_file, cli, settings);
      return 0;
    }

    const auto temp_dir = fs::path("run") / ("pieces_" + std::to_string(::getpid()));
    fs::create_directories(temp_dir);
    std::vector<std::string> piece_outputs;
    for (std::size_t i = 0; i < inputs.size(); ++i) {
      const auto piece = (temp_dir / ("piece_" + std::to_string(i) + ".root")).string();
      process_one_file(inputs[i], piece, cli, settings);
      piece_outputs.push_back(piece);
    }
    nano::runtime::merge_root_files(piece_outputs, cli.output_file);
    std::cout << "merged=" << cli.output_file << " pieces=" << piece_outputs.size() << "\n";
    return 0;
  } catch (const std::exception &ex) {
    std::cerr << "nano_run failed: " << ex.what() << "\n";
    return 1;
  }
}
