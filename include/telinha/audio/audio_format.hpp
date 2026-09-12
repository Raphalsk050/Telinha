#pragma once

#include <cstdint>

#include "telinha/core/config.hpp"

namespace tl::audio {

enum class SampleFormat : std::uint8_t {
    Unknown = 0,
    Int16,
    Float32,
};

const char* to_string(SampleFormat format) noexcept;

[[nodiscard]] constexpr std::uint32_t bytes_per_sample(SampleFormat format) noexcept
{
    switch (format) {
        case SampleFormat::Int16: return 2;
        case SampleFormat::Float32: return 4;
        case SampleFormat::Unknown: break;
    }
    return 0;
}

struct AudioFormat {
    std::uint32_t sample_rate = 0;
    std::uint16_t channels = 0;
    SampleFormat sample_format = SampleFormat::Unknown;

    [[nodiscard]] constexpr std::uint32_t bytes_per_frame() const noexcept
    {
        return bytes_per_sample(sample_format) * channels;
    }

    [[nodiscard]] constexpr bool valid() const noexcept
    {
        return sample_rate != 0 && channels != 0 && sample_format != SampleFormat::Unknown;
    }

    friend constexpr bool operator==(const AudioFormat& a, const AudioFormat& b) noexcept
    {
        return a.sample_rate == b.sample_rate && a.channels == b.channels &&
               a.sample_format == b.sample_format;
    }
    friend constexpr bool operator!=(const AudioFormat& a, const AudioFormat& b) noexcept
    {
        return !(a == b);
    }
};

}  // namespace tl::audio
