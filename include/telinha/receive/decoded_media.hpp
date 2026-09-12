#pragma once

#include <cstddef>
#include <cstdint>

#include "telinha/capture/pixel_format.hpp"
#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"

namespace tl::receive {

enum class FrameMemory : std::uint8_t {
    None = 0,
    GpuTexture,
    CpuPlanar,
};

const char* to_string(FrameMemory memory) noexcept;

inline constexpr std::uint32_t kMaxPlanes = 3;

struct DecodedVideoFrame {
    FrameMemory memory = FrameMemory::None;
    void* gpu_texture = nullptr;
    std::uint32_t gpu_subresource = 0;
    const std::byte* plane_data[kMaxPlanes] = {};
    std::uint32_t plane_pitch[kMaxPlanes] = {};
    std::uint32_t plane_count = 0;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    capture::PixelFormat format = capture::PixelFormat::Unknown;
    std::uint64_t frame_index = 0;
    Nanoseconds remote_time_ns = 0;
    Nanoseconds decode_end_ns = 0;
    bool keyframe = false;

    [[nodiscard]] bool valid() const noexcept { return memory != FrameMemory::None; }
};

}  // namespace tl::receive
