#pragma once

#include <cstdint>

#include "telinha/core/config.hpp"
#include "telinha/core/result.hpp"
#include "telinha/core/span.hpp"

namespace tl::diag {
inline constexpr std::size_t kMaxDiagnosticItems = 96;
inline constexpr std::size_t kDiagNameCapacity = 64;
inline constexpr std::size_t kDiagDetailCapacity = 192;
inline constexpr std::size_t kMaxStunServers = 4;
inline constexpr std::size_t kStunHostCapacity = 64;
inline constexpr std::size_t kAddressTextCapacity = 48;

enum class DiagCategory : std::uint8_t {
    System = 0,
    Graphics,
    Capture,
    Encode,
    Audio,
    Network,
};

inline constexpr std::size_t kDiagCategoryCount = 6;

const char* to_string(DiagCategory category) noexcept;

enum class DiagVerdict : std::uint8_t {
    Ready = 0,
    Degraded,
    Blocked,
    NotProbed,
};

const char* to_string(DiagVerdict verdict) noexcept;

enum class NatMapping : std::uint8_t {
    Unknown = 0,
    Blocked,
    EndpointIndependent,
    AddressDependent,
};

const char* to_string(NatMapping mapping) noexcept;

struct DiagItem {
    DiagCategory category = DiagCategory::System;
    bool available = false;
    Status status = Status::Ok;
    std::int32_t platform_code = 0;
    char name[kDiagNameCapacity] = {};
    char detail[kDiagDetailCapacity] = {};
};

struct StunServer {
    char host[kStunHostCapacity] = {};
    std::uint16_t port = 3478;
};

struct NetworkProbeResult {
    bool probed = false;
    NatMapping mapping = NatMapping::Unknown;
    std::uint32_t servers_answered = 0;
    std::uint16_t local_port = 0;
    std::uint16_t first_public_port = 0;
    std::uint16_t second_public_port = 0;
    char public_address[kAddressTextCapacity] = {};
};

struct DiagnosticReport {
    DiagItem items[kMaxDiagnosticItems] = {};
    std::uint32_t item_count = 0;
    DiagVerdict verdicts[kDiagCategoryCount] = {};
    NetworkProbeResult network{};

    [[nodiscard]] DiagVerdict verdict(DiagCategory category) const noexcept
    {
        return verdicts[static_cast<std::size_t>(category)];
    }

    [[nodiscard]] Span<const DiagItem> items_view() const noexcept
    {
        return Span<const DiagItem>{items, item_count};
    }
};

struct DiagnosticOptions {
    bool probe_network = false;
    std::uint32_t stun_timeout_ms = 1500;
    Span<const StunServer> stun_servers;
};

[[nodiscard]] Span<const StunServer> default_stun_servers() noexcept;

[[nodiscard]] Outcome collect_diagnostics(DiagnosticReport& out) noexcept;
[[nodiscard]] Outcome collect_diagnostics(DiagnosticReport& out,
                                          const DiagnosticOptions& options) noexcept;
}  // namespace tl::diag
