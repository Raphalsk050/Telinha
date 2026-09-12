#pragma once

#include <cstdint>

namespace tl::capture {
enum class PixelFormat : std::uint8_t {
    Unknown = 0,
    B8G8R8A8Unorm,
    R8G8B8A8Unorm,
    R10G10B10A2Unorm,
    R16G16B16A16Float,
    NV12,
};

const char* to_string(PixelFormat format) noexcept;

[[nodiscard]] constexpr std::uint32_t bits_per_pixel(PixelFormat format) noexcept
{
    switch (format) {
        case PixelFormat::B8G8R8A8Unorm:
        case PixelFormat::R8G8B8A8Unorm:
        case PixelFormat::R10G10B10A2Unorm: return 32;
        case PixelFormat::R16G16B16A16Float: return 64;
        case PixelFormat::NV12: return 12;
        case PixelFormat::Unknown: break;
    }
    return 0;
}

[[nodiscard]] constexpr bool is_high_dynamic_range(PixelFormat format) noexcept
{
    return format == PixelFormat::R10G10B10A2Unorm || format == PixelFormat::R16G16B16A16Float;
}
}  // namespace tl::capture
