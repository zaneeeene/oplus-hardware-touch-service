#pragma once

#include "oplus_touch/config.hpp"

#include <cstdint>
#include <optional>

namespace oplus::touch {

constexpr int32_t kPanelFeatureAodCaps = 217;
constexpr int32_t kPanelFeatureUdfpsType = 211;
constexpr int32_t kPanelAodCapabilities = 1 | 2 | 4 | 8;
constexpr int32_t kPanelUdfpsLocalHbm = 16;
constexpr uint32_t kPanelTransactionGetFeatureValue = 1;
constexpr uint32_t kPanelTransactionSetFeatureValue = 2;
constexpr uint32_t kPanelTransactionGetInfo = 3;
constexpr uint32_t kPanelTransactionGetInterfaceVersion = 16'777'215;
constexpr uint32_t kPanelTransactionGetInterfaceHash = 16'777'214;
constexpr int32_t kPanelAidlVersion = 1;
constexpr const char* kPanelAidlHash =
    "39afca22ac77253421ad681eeb2bae0c6ff62c13";

std::optional<int32_t> panel_feature_value(int32_t feature);
bool start_panel_feature_service(const AodPanelServiceConfig& config);

}  // namespace oplus::touch
