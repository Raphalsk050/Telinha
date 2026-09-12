#pragma once

#include <cstdint>
#include <memory>

#include "telinha/core/clock.hpp"
#include "telinha/core/result.hpp"
#include "telinha/receive/pcm.hpp"

namespace tl::receive {

enum class AudioRendererBackend : std::uint8_t {
    Automatic = 0,
    Wasapi,
    Headless,
};

const char* to_string(AudioRendererBackend backend) noexcept;

struct AudioRendererConfig {
    AudioRendererBackend backend = AudioRendererBackend::Automatic;
    PcmFormat format{48000, 2};
    Nanoseconds target_buffer_ns = 40ull * kNanosecondsPerMillisecond;
    Nanoseconds ring_capacity_ns = 500ull * kNanosecondsPerMillisecond;
    bool exclusive_mode = false;
};

struct AudioRendererInfo {
    AudioRendererBackend backend = AudioRendererBackend::Automatic;
    PcmFormat format;
    Nanoseconds device_period_ns = 0;
};

struct AudioRendererStats {
    std::uint64_t frames_submitted = 0;
    std::uint64_t frames_rendered = 0;
    std::uint64_t frames_dropped = 0;
    std::uint64_t underruns = 0;
    std::uint64_t format_rejections = 0;

    void reset() noexcept
    {
        frames_submitted = 0;
        frames_rendered = 0;
        frames_dropped = 0;
        underruns = 0;
        format_rejections = 0;
    }
};

class AudioRenderer {
public:
    virtual ~AudioRenderer() = default;

    AudioRenderer(const AudioRenderer&) = delete;
    AudioRenderer& operator=(const AudioRenderer&) = delete;

    virtual Outcome start() = 0;
    virtual void stop() noexcept = 0;

    [[nodiscard]] virtual Outcome submit(const PcmFrame& frame) = 0;

    [[nodiscard]] virtual Nanoseconds buffered_ns() const noexcept = 0;

    [[nodiscard]] virtual AudioRendererInfo info() const noexcept = 0;
    [[nodiscard]] virtual const AudioRendererStats& stats() const noexcept = 0;

protected:
    AudioRenderer() = default;
};

[[nodiscard]] bool audio_renderer_backend_available(AudioRendererBackend backend) noexcept;

[[nodiscard]] Result<std::unique_ptr<AudioRenderer>> create_audio_renderer(
    const AudioRendererConfig& config);

}  // namespace tl::receive
