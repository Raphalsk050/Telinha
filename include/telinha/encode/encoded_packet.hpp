#pragma once

#include <cstddef>
#include <cstdint>

#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/span.hpp"

namespace tl::encode {

enum class VideoCodec : std::uint8_t {
    Unknown = 0,
    H264,
    Hevc,
    Av1,
};

const char* to_string(VideoCodec codec) noexcept;

enum class AudioCodec : std::uint8_t {
    Unknown = 0,
    Opus,
};

const char* to_string(AudioCodec codec) noexcept;

struct EncodedVideoPacket {
    Span<const std::byte> bitstream;
    VideoCodec codec = VideoCodec::Unknown;
    std::uint64_t frame_index = 0;
    Nanoseconds capture_time_ns = 0;
    Nanoseconds encode_begin_ns = 0;
    Nanoseconds encode_end_ns = 0;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    std::uint32_t average_qp = 0;
    std::uint8_t temporal_id = 0;
    bool keyframe = false;

    [[nodiscard]] bool valid() const noexcept { return !bitstream.empty(); }
};

struct EncodedAudioPacket {
    Span<const std::byte> bitstream;
    AudioCodec codec = AudioCodec::Unknown;
    Nanoseconds capture_time_ns = 0;
    std::uint32_t sample_rate = 0;
    std::uint32_t frame_count = 0;
    std::uint16_t channels = 0;

    [[nodiscard]] bool valid() const noexcept { return !bitstream.empty(); }
};

}  // namespace tl::encode
