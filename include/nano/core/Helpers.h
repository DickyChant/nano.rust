#pragma once

#include "nano/core/Collection.h"

#include <Math/Vector4D.h>

#include <cmath>
#include <string>
#include <vector>

namespace nano {

using LorentzVector = ROOT::Math::PtEtaPhiMVector;

float delta_phi(float phi1, float phi2);
float delta_phi(const ObjectView &a, const ObjectView &b);
float delta_r(const ObjectView &a, const ObjectView &b);
LorentzVector polar_p4(const ObjectView &obj);
LorentzVector met_p4(float pt, float phi);

std::pair<int, float> closest_index(const ObjectView &obj, const std::vector<ObjectView> &collection);

}  // namespace nano
