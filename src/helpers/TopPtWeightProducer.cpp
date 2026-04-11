#include "nano/helpers/TopPtWeightProducer.h"

#include "nano/core/Collection.h"

#include <cmath>

namespace nano {

namespace {

float clip(float value, float low, float high) {
  return std::max(low, std::min(high, value));
}

}  // namespace

void TopPtWeightProducer::begin_file(OutputModel &out) const {
  out.branch("topptWeight", 1.0f);
}

void TopPtWeightProducer::fill(Event &event, OutputModel &out) const {
  if (!event.is_mc()) {
    out.fill("topptWeight", 1.0f);
    return;
  }

  auto genparts = event.collection("GenPart").objects();
  std::vector<ObjectView> gen_tops;
  for (auto &gp : genparts) {
    if (std::abs(gp.get<std::int32_t>("pdgId")) != 6) {
      continue;
    }
    if ((gp.get<std::int32_t>("statusFlags") & (1 << 13)) == 0) {
      continue;
    }
    gen_tops.push_back(gp);
  }
  if (gen_tops.size() != 2U) {
    out.fill("topptWeight", 1.0f);
    return;
  }

  const auto wgt_nnlo = [](float pt) {
    const auto x = clip(pt, 0.0f, 2000.0f);
    return 0.103f * std::exp(-0.0118f * x) - 0.000134f * x + 0.973f;
  };
  out.fill("topptWeight", std::sqrt(wgt_nnlo(gen_tops[0].pt()) * wgt_nnlo(gen_tops[1].pt())));
}

}  // namespace nano
