#pragma once

#include "telinha/capture/capture_target.hpp"
#include "telinha/core/result.hpp"
#include "telinha/core/span.hpp"
#include "win_capture_common.hpp"

namespace tl::capture::win {

[[nodiscard]] Outcome enumerate_targets(CaptureTargetKind kind, Span<CaptureTargetInfo> out,
                                        std::uint32_t& written, std::uint32_t& available) noexcept;

[[nodiscard]] SurfaceRotation rotation_from_dxgi(DXGI_MODE_ROTATION rotation) noexcept;

[[nodiscard]] Outcome describe_monitor(HMONITOR monitor, CaptureTargetInfo& out) noexcept;

[[nodiscard]] Outcome describe_window(HWND window, CaptureTargetInfo& out) noexcept;

[[nodiscard]] bool window_is_offerable(HWND window) noexcept;

}  // namespace tl::capture::win
