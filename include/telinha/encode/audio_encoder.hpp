#pragma once

#include <cstdint>
#include <memory>

#include "telinha/audio/captured_audio.hpp"
#include "telinha/core/result.hpp"
#include "telinha/encode/encoded_packet.hpp"

namespace tl::encode {

struct AudioEncoderConfig {
    AudioCodec codec = AudioCodec::Opus;
    std::uint32_t sample_rate = 48000;
    std::uint16_t channels = 2;
    std::uint32_t bitrate_bps = 128000;
    std::uint32_t frame_duration_us = 20000;
    bool full_band_music = true;
    bool inband_forward_error_correction = true;
};

class AudioEncoder {
public:
    virtual ~AudioEncoder() = default;

    AudioEncoder(const AudioEncoder&) = delete;
    AudioEncoder& operator=(const AudioEncoder&) = delete;

    virtual Outcome start() = 0;
    virtual void stop() noexcept = 0;

    [[nodiscard]] virtual Outcome submit(const audio::CapturedAudio& audio) = 0;
    [[nodiscard]] virtual Outcome poll(EncodedAudioPacket& out) = 0;
    virtual void release() noexcept = 0;

    [[nodiscard]] virtual Outcome set_target_bitrate(std::uint32_t bits_per_second) noexcept = 0;

protected:
    AudioEncoder() = default;
};

[[nodiscard]] Result<std::unique_ptr<AudioEncoder>> create_audio_encoder(
    const AudioEncoderConfig& config);

}  // namespace tl::encode
