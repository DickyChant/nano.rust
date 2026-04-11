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
  float best_dr = 1000.0f;
  for (std::size_t i = 0; i < collection.size(); ++i) {
    const auto dr = delta_r(obj, collection[i]);
    if (dr < best_dr) {
      best_dr = dr;
      best_index = static_cast<int>(i);
    }
  }
  return {best_index, best_dr};
}

bool safe_bool(const Event &event, std::string_view branch_name) {
  const auto *info = event.schema().find(branch_name);
  if (!info) {
    return false;
  }
  try {
    return event.scalar<bool>(branch_name);
  } catch (...) {
    return false;
  }
}

float safe_object_float(const ObjectView &obj, std::string_view attr, float fallback) {
  try {
    return obj.get<float>(attr);
  } catch (...) {
    return fallback;
  }
}

std::int32_t safe_object_int(const ObjectView &obj, std::string_view attr, std::int32_t fallback) {
  try {
    return obj.get<std::int32_t>(attr);
  } catch (...) {
    return fallback;
  }
}

bool pass_trigger(const Event &event, const std::vector<std::string> &triggers) {
  for (const auto &trigger : triggers) {
    if (safe_bool(event, trigger)) {
      return true;
    }
  }
  return false;
}

}  // namespace nano
