#pragma once

#include "nano/core/Event.h"
#include "nano/core/OutputModel.h"

namespace nano {

class TopPtWeightProducer {
public:
  void begin_file(OutputModel &out) const;
  void fill(Event &event, OutputModel &out) const;
};

}  // namespace nano
