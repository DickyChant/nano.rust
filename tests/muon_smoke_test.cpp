#include "nano/core/Event.h"
#include "nano/io/NanoReader.h"
#include "nano/producers/HeavyFlavMuonSampleProducer.h"

#include <algorithm>
#include <exception>
#include <iostream>

int main(int argc, char **argv) {
  if (argc < 3) {
    std::cerr << "Usage: muon_smoke_test <file.root> <tree-name>\n";
    return 2;
  }

  try {
    nano::NanoReader reader(argv[2], argv[1], nano::BranchSchema(nano::HeavyFlavBaseProducer::default_schema()));

    nano::ProducerConfig config;
    config.year = "2024";
    config.channel = "muon";
    config.jet_type = "ak8";
    config.nano_version = "V15";

    nano::HeavyFlavMuonSampleProducer producer(config);
    producer.begin_file();

    const std::size_t max_events = std::min<std::size_t>(100, reader.entries());
    std::size_t accepted = 0;
    for (std::size_t i = 0; i < max_events; ++i) {
      nano::Event event(reader, i);
      if (producer.analyze(event)) {
        ++accepted;
      }
    }

    std::cout << "processed=" << max_events << " accepted=" << accepted << "\n";
    return max_events == 100 ? 0 : 1;
  } catch (const std::exception &ex) {
    std::cerr << "muon_smoke_test failed: " << ex.what() << "\n";
    return 1;
  }
}
