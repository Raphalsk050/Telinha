#pragma once

#include "telinha/diag/diagnostics.hpp"

namespace tl::diag {
struct StunBinding {
    std::uint32_t address_v4 = 0;
    std::uint16_t port = 0;
};

[[nodiscard]] Outcome probe_nat_mapping(Span<const StunServer> servers, std::uint32_t timeout_ms,
                                        NetworkProbeResult& out) noexcept;
}  // namespace tl::diag
