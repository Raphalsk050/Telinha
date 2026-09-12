#pragma once

#include <cstddef>
#include <cstdint>

#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/span.hpp"

namespace tl::receive {

struct PcmFormat {
    std::uint32_t sample_rate_hz = 0;
    std::uint16_t channels = 0;

    [[nodiscard]] constexpr bool valid() const noexcept
    {
        return sample_rate_hz != 0 && channels != 0;
    }

    [[nodiscard]] constexpr std::uint32_t bytes_per_frame() const noexcept
    {
        return static_cast<std::uint32_t>(sizeof(std::int16_t)) * channels;
    }

    [[nodiscard]] constexpr Nanoseconds frames_to_ns(std::uint64_t frames) const noexcept
    {
        return sample_rate_hz == 0 ? 0 : (frames * kNanosecondsPerSecond) / sample_rate_hz;
    }

    [[nodiscard]] constexpr std::uint64_t ns_to_frames(Nanoseconds ns) const noexcept
    {
        return (ns * sample_rate_hz) / kNanosecondsPerSecond;
    }

    friend constexpr bool operator==(const PcmFormat& a, const PcmFormat& b) noexcept
    {
        return a.sample_rate_hz == b.sample_rate_hz && a.channels == b.channels;
    }
    friend constexpr bool operator!=(const PcmFormat& a, const PcmFormat& b) noexcept
    {
        return !(a == b);
    }
};

struct PcmFrame {
    Span<const std::int16_t> interleaved;
    PcmFormat format;
    std::uint32_t frames_per_channel = 0;
    Nanoseconds remote_time_ns = 0;

    [[nodiscard]] bool valid() const noexcept
    {
        return frames_per_channel != 0 && format.valid() && !interleaved.empty();
    }
};

}  // namespace tl::receive
