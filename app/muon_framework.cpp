#include "nano/core/Event.h"
#include "nano/io/NanoReader.h"
#include "nano/producers/HeavyFlavMuonSampleProducer.h"

#include <iostream>
#include <string>

int main(int argc, char **argv) {
  if (argc < 3) {
    std::cerr << "Usage: muon_framework <file.root> <tree-name>\n";
    return 1;
  }

  nano::NanoReader reader(argv[2], argv[1], nano::BranchSchema(nano::HeavyFlavBaseProducer::default_schema()));
  nano::ProducerConfig config;
  config.year = "2024";
  config.channel = "muon";
  config.jet_type = "ak8";
  config.nano_version = "V15";
  nano::HeavyFlavMuonSampleProducer producer(config);
  producer.begin_file();

  for (std::size_t i = 0; i < reader.entries(); ++i) {
    nano::Event event(reader, i);
    if (!producer.analyze(event)) {
      continue;
    }
    const auto &values = producer.output().values();
    std::cout << "entry=" << i << " muon_pt=" << std::get<float>(values.at("muon_pt"))
              << " fj_1_pt=" << std::get<float>(values.at("fj_1_pt")) << "\n";
  }
  return 0;
}
