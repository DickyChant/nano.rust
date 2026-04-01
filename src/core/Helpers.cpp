#include "nano/core/Helpers.h"

namespace nano {

float delta_phi(float phi1, float phi2) {
  float dphi = phi1 - phi2;
  constexpr float kPi = static_cast<float>(M_PI);
  while (dphi > kPi) {
    dphi -= 2.0f * kPi;
  }
  while (dphi < -kPi) {
    dphi += 2.0f * kPi;
  }
  return dphi;
}

float delta_phi(const ObjectView &a, const ObjectView &b) {
  return delta_phi(a.phi(), b.phi());
}

float delta_r(const ObjectView &a, const ObjectView &b) {
  const auto deta = a.eta() - b.eta();
  const auto dphi = delta_phi(a, b);
  return std::sqrt(deta * deta + dphi * dphi);
}

LorentzVector polar_p4(const ObjectView &obj) {
  return LorentzVector(obj.pt(), obj.eta(), obj.phi(), obj.mass());
}

LorentzVector met_p4(float pt, float phi) {
  return LorentzVector(pt, 0.0f, phi, 0.0f);
}

std::pair<int, float> closest_index(const ObjectView &obj, const std::vector<ObjectView> &collection) {
  int best_index = -1;
  float best_dr = 1.0e6f;
  for (std::size_t i = 0; i < collection.size(); ++i) {
    const auto dr = delta_r(obj, collection[i]);
    if (dr < best_dr) {
      best_dr = dr;
      best_index = static_cast<int>(i);
    }
  }
  return {best_index, best_dr};
}

}  // namespace nano
