#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>

#include "telinha/audio/audio_format.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/result.hpp"

namespace tl::audio {

inline constexpr std::uint32_t kMaxChannels = 8;
inline constexpr std::uint32_t kResamplerTapsPerPhase = 32;
inline constexpr std::uint32_t kResamplerMaxPhases = 1024;

class PolyphaseResampler {
public:
    PolyphaseResampler() noexcept = default;

    PolyphaseResampler(const PolyphaseResampler&) = delete;
    PolyphaseResampler& operator=(const PolyphaseResampler&) = delete;

    [[nodiscard]] Outcome configure(std::uint32_t input_rate, std::uint32_t output_rate,
                                    std::uint32_t channels,
                                    std::uint32_t max_input_frames) noexcept;

    void reset() noexcept;

    [[nodiscard]] bool bypass() const noexcept { return bypass_; }

    [[nodiscard]] std::uint32_t max_output_frames(std::uint32_t input_frames) const noexcept;

    [[nodiscard]] std::uint32_t process(const float* interleaved_input, std::uint32_t input_frames,
                                        float* interleaved_output,
                                        std::uint32_t output_capacity_frames) noexcept;

private:
    std::unique_ptr<float[]> taps_;
    std::unique_ptr<float[]> work_;

    std::uint32_t interpolation_ = 1;
    std::uint32_t decimation_ = 1;
    std::uint32_t channels_ = 0;
    std::uint32_t max_input_frames_ = 0;
    std::uint32_t phase_ = 0;
    std::uint32_t skip_ = 0;
    bool bypass_ = true;
};

class AudioConverter {
public:
    AudioConverter() noexcept = default;

    AudioConverter(const AudioConverter&) = delete;
    AudioConverter& operator=(const AudioConverter&) = delete;

    [[nodiscard]] Outcome configure(const AudioFormat& input, const AudioFormat& output,
                                    std::uint32_t max_input_frames) noexcept;

    void reset() noexcept;

    [[nodiscard]] bool configured() const noexcept { return configured_; }

    [[nodiscard]] std::uint32_t max_output_frames() const noexcept { return max_output_frames_; }

    [[nodiscard]] std::uint32_t max_output_bytes() const noexcept
    {
        return max_output_frames_ * output_.bytes_per_frame();
    }

    [[nodiscard]] std::uint32_t convert(const std::byte* input, std::uint32_t input_frames,
                                        std::byte* output,
                                        std::uint32_t output_capacity_frames) noexcept;

    [[nodiscard]] std::uint32_t convert_silence(std::uint32_t input_frames, std::byte* output,
                                                std::uint32_t output_capacity_frames) noexcept;

private:
    void decode(const std::byte* input, std::uint32_t frames) noexcept;
    void map_channels(std::uint32_t frames) noexcept;
    void encode(const float* source, std::uint32_t frames, std::byte* output) noexcept;

    AudioFormat input_;
    AudioFormat output_;

    std::unique_ptr<float[]> decoded_;
    std::unique_ptr<float[]> mapped_;
    std::unique_ptr<float[]> resampled_;
    float coefficients_[kMaxChannels * kMaxChannels] = {};

    PolyphaseResampler resampler_;

    std::uint32_t max_input_frames_ = 0;
    std::uint32_t max_output_frames_ = 0;
    bool passthrough_ = false;
    bool configured_ = false;
};

}  // namespace tl::audio
