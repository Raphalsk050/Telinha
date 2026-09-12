#pragma once

#include <cstddef>
#include <cstdint>

#include "telinha/audio/audio_format.hpp"
#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/span.hpp"

namespace tl::audio {

struct CapturedAudio {
    Span<const std::byte> samples;
    AudioFormat format;
    std::uint32_t frame_count = 0;
    Nanoseconds device_time_ns = 0;
    Nanoseconds acquire_time_ns = 0;
    std::uint64_t stream_position_frames = 0;
    bool silent = false;
    bool discontinuity = false;
    bool timestamp_valid = false;

    [[nodiscard]] bool valid() const noexcept { return frame_count != 0 && format.valid(); }
};

}  // namespace tl::audio
