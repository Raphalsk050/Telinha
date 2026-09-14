#pragma once

#include <cstdint>
#include <memory>

#include "telinha/capture/capture_source.hpp"

namespace tl::capture::win {

[[nodiscard]] Outcome enumerate_capture_devices(Span<CaptureTargetInfo> out, std::uint32_t& written,
                                                std::uint32_t& available) noexcept;

[[nodiscard]] Result<std::unique_ptr<CaptureSource>> create_media_foundation_source(
    const CaptureTarget& target, const CaptureOptions& options) noexcept;

}  // namespace tl::capture::win
