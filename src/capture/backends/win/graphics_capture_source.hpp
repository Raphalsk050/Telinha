#pragma once

#include <memory>

#include "telinha/capture/capture_source.hpp"

namespace tl::capture::win {

[[nodiscard]] bool graphics_capture_supported() noexcept;

[[nodiscard]] Result<std::unique_ptr<CaptureSource>> create_graphics_capture_source(
    const CaptureTarget& target, const CaptureOptions& options) noexcept;

}  // namespace tl::capture::win
