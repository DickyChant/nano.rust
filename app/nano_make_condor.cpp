#include "runtime_common.h"

#include <filesystem>
#include <fstream>
#include <iostream>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <unistd.h>

namespace fs = std::filesystem;

namespace {

struct CliOptions {
  std::string input_yaml;
  std::string output_dir;
  std::string config_file;
  std::string channel = "muon";
  std::string tree_name = "Events";
  long long num_events = -1;
  std::size_t nfiles_per_job = 1;
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
    if (arg == "--input-yaml") {
      opts.input_yaml = need_value("--input-yaml");
    } else if (arg == "--output-dir") {
      opts.output_dir = need_value("--output-dir");
    } else if (arg == "--config") {
      opts.config_file = need_value("--config");
    } else if (arg == "--channel") {
      opts.channel = need_value("--channel");
    } else if (arg == "--tree-name") {
      opts.tree_name = need_value("--tree-name");
    } else if (arg == "--num-events") {
      opts.num_events = std::stoll(need_value("--num-events"));
    } else if (arg == "--nfiles-per-job") {
      opts.nfiles_per_job = static_cast<std::size_t>(std::stoul(need_value("--nfiles-per-job")));
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
  if (opts.input_yaml.empty() || opts.output_dir.empty() || opts.config_file.empty()) {
    throw std::runtime_error("Usage: nano_make_condor --input-yaml <samples.yaml> --output-dir <dir> --config <card.yaml> [--nfiles-per-job 1]");
  }
  return opts;
}

std::string write_merged_config(const fs::path &path, const YAML::Node &settings) {
  nano::runtime::dump_yaml_file(settings, path.string());
  return path.string();
}

void write_process_script(const fs::path &path) {
  std::ofstream out(path);
  out << "#!/usr/bin/env bash\n";
  out << "set -euo pipefail\n";
  out << "source /cvmfs/sft.cern.ch/lcg/views/LCG_108/x86_64-el9-gcc13-opt/setup.sh\n";
  out << "WORKDIR=$(pwd)\n";
  out << "REPO_DIR=${WORKDIR}/repo\n";
  out << "if [ ! -d \"${REPO_DIR}\" ]; then\n";
  out << "  mkdir -p \"${REPO_DIR}\"\n";
  out << "  tar -xzf repo.tar.gz -C \"${REPO_DIR}\" --strip-components=0\n";
  out << "fi\n";
  out << "if [ ! -x \"${REPO_DIR}/build/nano_run\" ]; then\n";
  out << "  cmake -S \"${REPO_DIR}\" -B \"${REPO_DIR}/build\"\n";
  out << "  cmake --build \"${REPO_DIR}/build\" -j\n";
  out << "fi\n";
  out << "MANIFEST=$1\n";
  out << "OUTPUT=$2\n";
  out << "TREE_NAME=$3\n";
  out << "NUM_EVENTS=$4\n";
  out << "CONFIG=$5\n";
  out << "CHANNEL=$6\n";
  out << "INPUTS=$(cat \"${MANIFEST}\" | paste -sd, -)\n";
  out << "CMD=(\"${REPO_DIR}/build/nano_run\" --input-files \"${INPUTS}\" --output-file \"${OUTPUT}\" --tree-name \"${TREE_NAME}\" --num-events \"${NUM_EVENTS}\" --channel \"${CHANNEL}\" --config \"${CONFIG}\")\n";
  out << "printf 'Running command:'\n";
  out << "printf ' %q' \"${CMD[@]}\"\n";
  out << "printf '\\n'\n";
  out << "\"${CMD[@]}\"\n";
  out.close();
  fs::permissions(path, fs::perms::owner_exec | fs::perms::owner_read | fs::perms::owner_write | fs::perms::group_exec |
                            fs::perms::group_read | fs::perms::others_exec | fs::perms::others_read,
                  fs::perm_options::add);
}

}  // namespace

int main(int argc, char **argv) {
  try {
    const auto cli = parse_args(argc, argv);
    auto settings = nano::runtime::load_config_with_extends(cli.config_file);
    for (const auto &[key, value] : cli.overrides) {
      nano::runtime::apply_override(settings, key, value);
    }

    fs::create_directories("run");
    const auto era = settings["era"] ? settings["era"].as<std::string>() : std::string("era");
    const auto workdir = fs::path("run") / ("condor_" + cli.channel + "_" + era + "_" + std::to_string(::getpid()));
    const auto jobs_dir = workdir / "jobs";
    const auto pieces_dir = workdir / "pieces";
    fs::create_directories(jobs_dir);
    fs::create_directories(pieces_dir);

    const auto merged_config = write_merged_config(workdir / "config_snapshot.yaml", settings);
    write_process_script(workdir / "process.sh");

    const auto tarball = (workdir / "repo.tar.gz").string();
    const auto tar_cmd = "tar czf " + tarball + " --exclude='./run' --exclude='./build' .";
    const auto rc = std::system(tar_cmd.c_str());
    if (rc != 0) {
      throw std::runtime_error("Failed to create repository tarball");
    }

    const auto sample_map = nano::runtime::parse_sample_yaml(cli.input_yaml);
    std::vector<std::pair<std::string, std::string>> queue_rows;
    std::size_t job_index = 0;
    for (const auto &[sample, datasets] : sample_map) {
      std::vector<std::string> files;
      for (const auto &dataset : datasets) {
        const auto resolved = nano::runtime::resolve_dataset_entry(dataset);
        files.insert(files.end(), resolved.begin(), resolved.end());
      }
      const auto chunks = nano::runtime::chunk_join(files, cli.nfiles_per_job);
      for (const auto &chunk : chunks) {
        const auto manifest = jobs_dir / ("job_" + std::to_string(job_index) + ".txt");
        std::ofstream out(manifest);
        for (const auto &file : nano::runtime::split_csv(chunk)) {
          out << file << "\n";
        }
        const auto output_piece = cli.output_dir + "/pieces/piece_" + std::to_string(job_index) + ".root";
        queue_rows.emplace_back(manifest.string(), output_piece);
        ++job_index;
      }
    }

    std::ofstream jdl(workdir / "submit.jdl");
    jdl << "universe = vanilla\n";
    jdl << "executable = process.sh\n";
    jdl << "should_transfer_files = YES\n";
    jdl << "when_to_transfer_output = ON_EXIT\n";
    jdl << "transfer_input_files = repo.tar.gz,config_snapshot.yaml,$(manifest)\n";
    jdl << "output = logs/job_$(Cluster)_$(Process).out\n";
    jdl << "error = logs/job_$(Cluster)_$(Process).err\n";
    jdl << "log = logs/job_$(Cluster).log\n";
    jdl << "request_cpus = 1\n";
    jdl << "arguments = $(manifest) $(output_piece) " << cli.tree_name << " " << cli.num_events << " config_snapshot.yaml " << cli.channel << "\n";
    jdl << "queue manifest,output_piece from (\n";
    for (const auto &[manifest, output_piece] : queue_rows) {
      jdl << manifest << " " << output_piece << "\n";
    }
    jdl << ")\n";

    fs::create_directories(workdir / "logs");
    std::cout << "Created condor workdir: " << workdir << "\n";
    std::cout << "Jobs: " << queue_rows.size() << "\n";
    std::cout << "Next step:\n";
    std::cout << "  cd " << workdir << " && condor_submit submit.jdl\n";
    return 0;
  } catch (const std::exception &ex) {
    std::cerr << "nano_make_condor failed: " << ex.what() << "\n";
    return 1;
  }
}
