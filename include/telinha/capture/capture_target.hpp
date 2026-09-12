#pragma once

#include <cstdint>

#include "telinha/core/config.hpp"

namespace tl::capture {
enum class CaptureTargetKind : std::uint8_t {
    None = 0,
    Monitor,
    Window,
};

const char* to_string(CaptureTargetKind kind) noexcept;

struct CaptureTarget {
    CaptureTargetKind kind = CaptureTargetKind::None;
    std::uint64_t handle = 0;

    [[nodiscard]] constexpr bool valid() const noexcept
    {
        return kind != CaptureTargetKind::None && handle != 0;
    }

    [[nodiscard]] static constexpr CaptureTarget monitor(std::uint64_t handle) noexcept
    {
        return CaptureTarget{CaptureTargetKind::Monitor, handle};
    }

    [[nodiscard]] static constexpr CaptureTarget window(std::uint64_t handle) noexcept
    {
        return CaptureTarget{CaptureTargetKind::Window, handle};
    }

    friend constexpr bool operator==(const CaptureTarget& a, const CaptureTarget& b) noexcept
    {
        return a.kind == b.kind && a.handle == b.handle;
    }
    friend constexpr bool operator!=(const CaptureTarget& a, const CaptureTarget& b) noexcept
    {
        return !(a == b);
    }
};

inline constexpr std::uint32_t kTargetNameCapacity = 128;

struct CaptureTargetInfo {
    CaptureTarget target;
    std::int32_t x = 0;
    std::int32_t y = 0;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    std::uint32_t refresh_millihertz = 0;
    std::uint32_t process_id = 0;
    bool primary = false;
    char name[kTargetNameCapacity] = {};
};
}  // namespace tl::capture
