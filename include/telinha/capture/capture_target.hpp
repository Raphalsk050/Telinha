#pragma once

#include <cstdint>

#include "telinha/core/config.hpp"

namespace tl::capture {
enum class SurfaceRotation : std::uint8_t {
    None = 0,
    Clockwise90,
    Clockwise180,
    Clockwise270,
};

const char* to_string(SurfaceRotation rotation) noexcept;

enum class CaptureTargetKind : std::uint8_t {
    None = 0,
    Monitor,
    Window,
    Device,
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

    [[nodiscard]] static constexpr CaptureTarget device(std::uint64_t handle) noexcept
    {
        return CaptureTarget{CaptureTargetKind::Device, handle};
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
    SurfaceRotation rotation = SurfaceRotation::None;
    bool primary = false;
    char name[kTargetNameCapacity] = {};
    std::uint8_t container_id[16] = {};
};
}  // namespace tl::capture
