#pragma once

#include <memory>

#include "telinha/capture/capture_source.hpp"

namespace tl::capture::win {

[[nodiscard]] Result<std::unique_ptr<CaptureSource>> create_window_capture_source(
    const CaptureTarget& target, const CaptureOptions& options) noexcept;

}  // namespace tl::capture::win
