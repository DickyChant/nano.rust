#pragma once

#include "nano/producers/HeavyFlavBaseProducer.h"

namespace nano {

class HeavyFlavMuonSampleProducer : public HeavyFlavBaseProducer {
public:
  explicit HeavyFlavMuonSampleProducer(ProducerConfig config);

  void begin_file() override;
  bool analyze(Event &event) override;
};

}  // namespace nano
