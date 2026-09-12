#pragma once

#include <cstdint>
#include <memory>

#include "telinha/core/clock.hpp"
#include "telinha/core/latency_histogram.hpp"
#include "telinha/core/result.hpp"
#include "telinha/receive/decoded_media.hpp"

namespace tl::receive {

enum class RendererBackend : std::uint8_t {
    Automatic = 0,
    Direct3D11,
    Headless,
};

const char* to_string(RendererBackend backend) noexcept;

inline constexpr std::uint32_t kWindowTitleCapacity = 96;

struct VideoRendererConfig {
    RendererBackend backend = RendererBackend::Automatic;
    char title[kWindowTitleCapacity] = {};
    std::uint32_t width = 1280;
    std::uint32_t height = 720;
    std::uint32_t swapchain_buffers = 3;
    bool vertical_sync = false;
    bool allow_tearing = true;
    bool start_fullscreen = false;
};

struct VideoRendererInfo {
    RendererBackend backend = RendererBackend::Automatic;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    void* native_device = nullptr;
    bool accepts_gpu_surfaces = false;
};

struct RendererStats {
    LatencyHistogram present_ns;
    LatencyHistogram present_interval_ns;
    std::uint64_t frames_presented = 0;
    std::uint64_t frames_dropped_late = 0;
    std::uint64_t frames_repeated = 0;
    std::uint64_t swapchain_resets = 0;

    void reset() noexcept
    {
        present_ns.reset();
        present_interval_ns.reset();
        frames_presented = 0;
        frames_dropped_late = 0;
        frames_repeated = 0;
        swapchain_resets = 0;
    }
};

class VideoRenderer {
public:
    virtual ~VideoRenderer() = default;

    VideoRenderer(const VideoRenderer&) = delete;
    VideoRenderer& operator=(const VideoRenderer&) = delete;

    virtual Outcome start() = 0;
    virtual void stop() noexcept = 0;

    [[nodiscard]] virtual Outcome pump(bool& close_requested) noexcept = 0;

    [[nodiscard]] virtual Outcome present(const DecodedVideoFrame& frame) = 0;

    [[nodiscard]] virtual VideoRendererInfo info() const noexcept = 0;
    [[nodiscard]] virtual const RendererStats& stats() const noexcept = 0;

protected:
    VideoRenderer() = default;
};

[[nodiscard]] bool renderer_backend_available(RendererBackend backend) noexcept;

[[nodiscard]] Result<std::unique_ptr<VideoRenderer>> create_video_renderer(
    const VideoRendererConfig& config);

}  // namespace tl::receive
