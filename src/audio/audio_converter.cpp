#include "audio_converter.hpp"

#include <algorithm>
#include <cmath>
#include <cstring>
#include <new>

namespace tl::audio {
namespace {

constexpr double kPi = 3.14159265358979323846;
constexpr double kKaiserBeta = 8.6;
constexpr float kAttenuatedGain = 0.70710678f;

[[nodiscard]] std::uint32_t greatest_common_divisor(std::uint32_t a, std::uint32_t b) noexcept
{
    while (b != 0) {
        const std::uint32_t remainder = a % b;
        a = b;
        b = remainder;
    }
    return a;
}

[[nodiscard]] double bessel_i0(double x) noexcept
{
    double sum = 1.0;
    double term = 1.0;
    for (int k = 1; k < 64; ++k) {
        const double factor = x / (2.0 * static_cast<double>(k));
        term *= factor * factor;
        sum += term;
        if (term < sum * 1e-17) {
            break;
        }
    }
    return sum;
}

[[nodiscard]] double normalized_sinc(double x) noexcept
{
    if (x == 0.0) {
        return 1.0;
    }
    const double scaled = kPi * x;
    return std::sin(scaled) / scaled;
}

[[nodiscard]] double windowed_sinc(std::size_t index, std::size_t length, double cutoff,
                                   double window_denominator) noexcept
{
    const double center = static_cast<double>(length - 1) / 2.0;
    const double offset = static_cast<double>(index) - center;
    const double position =
        2.0 * static_cast<double>(index) / static_cast<double>(length - 1) - 1.0;
    const double window =
        bessel_i0(kKaiserBeta * std::sqrt(std::max(0.0, 1.0 - position * position))) /
        window_denominator;
    return 2.0 * cutoff * normalized_sinc(2.0 * cutoff * offset) * window;
}

void stereo_weights_for_speaker(std::uint32_t speaker, float& left, float& right) noexcept
{
    left = 0.0f;
    right = 0.0f;
    switch (speaker) {
        case 0x1: left = 1.0f; break;
        case 0x2: right = 1.0f; break;
        case 0x4:
            left = kAttenuatedGain;
            right = kAttenuatedGain;
            break;
        case 0x8: break;
        case 0x10: left = kAttenuatedGain; break;
        case 0x20: right = kAttenuatedGain; break;
        case 0x40: left = kAttenuatedGain; break;
        case 0x80: right = kAttenuatedGain; break;
        case 0x100:
            left = 0.5f;
            right = 0.5f;
            break;
        case 0x200: left = kAttenuatedGain; break;
        case 0x400: right = kAttenuatedGain; break;
        default: break;
    }
}

[[nodiscard]] std::uint32_t default_channel_mask(std::uint32_t channels) noexcept
{
    switch (channels) {
        case 1: return 0x4;
        case 2: return 0x3;
        case 3: return 0x7;
        case 4: return 0x33;
        case 5: return 0x37;
        case 6: return 0x3F;
        case 7: return 0x13F;
        case 8: return 0x63F;
        default: return 0;
    }
}

void build_channel_coefficients(const AudioFormat& input, const AudioFormat& output,
                                float* coefficients) noexcept
{
    std::memset(coefficients, 0, kMaxChannels * kMaxChannels * sizeof(float));

    const std::uint32_t inputs = input.channels;
    const std::uint32_t outputs = output.channels;
    const std::uint32_t channel_mask = default_channel_mask(inputs);

    if (inputs == outputs) {
        for (std::uint32_t channel = 0; channel < inputs; ++channel) {
            coefficients[channel * kMaxChannels + channel] = 1.0f;
        }
        return;
    }

    if (inputs == 1) {
        for (std::uint32_t channel = 0; channel < outputs; ++channel) {
            coefficients[channel * kMaxChannels] = 1.0f;
        }
        return;
    }

    if (outputs == 1) {
        const float weight = 1.0f / static_cast<float>(inputs);
        for (std::uint32_t channel = 0; channel < inputs; ++channel) {
            coefficients[channel] = weight;
        }
        return;
    }

    if (outputs == 2) {
        if (channel_mask != 0) {
            std::uint32_t assigned = 0;
            for (std::uint32_t bit = 0; bit < 32 && assigned < inputs; ++bit) {
                const std::uint32_t speaker = std::uint32_t{1} << bit;
                if ((channel_mask & speaker) == 0) {
                    continue;
                }
                float left = 0.0f;
                float right = 0.0f;
                stereo_weights_for_speaker(speaker, left, right);
                coefficients[assigned] = left;
                coefficients[kMaxChannels + assigned] = right;
                ++assigned;
            }
            if (assigned == inputs) {
                return;
            }
            std::memset(coefficients, 0, kMaxChannels * kMaxChannels * sizeof(float));
        }

        coefficients[0] = 1.0f;
        coefficients[kMaxChannels + 1] = 1.0f;
        for (std::uint32_t channel = 2; channel < inputs; ++channel) {
            coefficients[channel] = kAttenuatedGain;
            coefficients[kMaxChannels + channel] = kAttenuatedGain;
        }
        return;
    }

    const std::uint32_t shared = std::min(inputs, outputs);
    for (std::uint32_t channel = 0; channel < shared; ++channel) {
        coefficients[channel * kMaxChannels + channel] = 1.0f;
    }
}

}  // namespace

Outcome PolyphaseResampler::configure(std::uint32_t input_rate, std::uint32_t output_rate,
                                      std::uint32_t channels,
                                      std::uint32_t max_input_frames) noexcept
{
    taps_.reset();
    work_.reset();
    phase_ = 0;
    skip_ = 0;
    bypass_ = true;
    channels_ = 0;
    max_input_frames_ = 0;

    if (input_rate == 0 || output_rate == 0 || channels == 0 || channels > kMaxChannels ||
        max_input_frames == 0) {
        return fail(Status::InvalidArgument, "resampler: rate, channel or block size is zero");
    }

    channels_ = channels;
    max_input_frames_ = max_input_frames;
    interpolation_ = 1;
    decimation_ = 1;

    if (input_rate == output_rate) {
        return ok();
    }

    const std::uint32_t divisor = greatest_common_divisor(input_rate, output_rate);
    interpolation_ = output_rate / divisor;
    decimation_ = input_rate / divisor;

    if (interpolation_ > kResamplerMaxPhases) {
        return fail(Status::NotSupported, "resampler: rate pair needs more phases than budgeted");
    }

    const std::size_t tap_count = static_cast<std::size_t>(interpolation_) * kResamplerTapsPerPhase;
    taps_.reset(new (std::nothrow) float[tap_count]);
    if (taps_ == nullptr) {
        return fail(Status::OutOfMemory, "resampler: filter taps");
    }

    const double cutoff =
        0.5 *
        std::min(1.0, static_cast<double>(interpolation_) / static_cast<double>(decimation_)) /
        static_cast<double>(interpolation_);
    const double window_denominator = bessel_i0(kKaiserBeta);

    for (std::uint32_t phase = 0; phase < interpolation_; ++phase) {
        double sum = 0.0;
        for (std::uint32_t tap = 0; tap < kResamplerTapsPerPhase; ++tap) {
            const std::size_t index =
                static_cast<std::size_t>(phase) + static_cast<std::size_t>(tap) * interpolation_;
            const double value = windowed_sinc(index, tap_count, cutoff, window_denominator);
            taps_[static_cast<std::size_t>(phase) * kResamplerTapsPerPhase + tap] =
                static_cast<float>(value);
            sum += value;
        }
        if (sum > 1e-12 || sum < -1e-12) {
            const float scale = static_cast<float>(1.0 / sum);
            for (std::uint32_t tap = 0; tap < kResamplerTapsPerPhase; ++tap) {
                taps_[static_cast<std::size_t>(phase) * kResamplerTapsPerPhase + tap] *= scale;
            }
        }
    }

    const std::size_t work_samples =
        (static_cast<std::size_t>(kResamplerTapsPerPhase - 1) + max_input_frames_) * channels_;
    work_.reset(new (std::nothrow) float[work_samples]);
    if (work_ == nullptr) {
        taps_.reset();
        return fail(Status::OutOfMemory, "resampler: work buffer");
    }
    std::memset(work_.get(), 0, work_samples * sizeof(float));

    bypass_ = false;
    return ok();
}

void PolyphaseResampler::reset() noexcept
{
    phase_ = 0;
    skip_ = 0;
    if (work_ != nullptr && channels_ != 0) {
        const std::size_t history =
            static_cast<std::size_t>(kResamplerTapsPerPhase - 1) * channels_;
        std::memset(work_.get(), 0, history * sizeof(float));
    }
}

std::uint32_t PolyphaseResampler::max_output_frames(std::uint32_t input_frames) const noexcept
{
    if (bypass_) {
        return input_frames;
    }
    const std::uint64_t scaled = static_cast<std::uint64_t>(input_frames) * interpolation_;
    return static_cast<std::uint32_t>(scaled / decimation_ + 2);
}

std::uint32_t PolyphaseResampler::process(const float* interleaved_input,
                                          std::uint32_t input_frames, float* interleaved_output,
                                          std::uint32_t output_capacity_frames) noexcept
{
    if (channels_ == 0 || input_frames == 0) {
        return 0;
    }

    if (bypass_) {
        const std::uint32_t frames = std::min(input_frames, output_capacity_frames);
        std::memcpy(interleaved_output, interleaved_input,
                    static_cast<std::size_t>(frames) * channels_ * sizeof(float));
        return frames;
    }

    if (input_frames > max_input_frames_ ||
        output_capacity_frames < max_output_frames(input_frames)) {
        return 0;
    }

    constexpr std::uint32_t kHistoryFrames = kResamplerTapsPerPhase - 1;
    float* work = work_.get();
    std::memcpy(work + static_cast<std::size_t>(kHistoryFrames) * channels_, interleaved_input,
                static_cast<std::size_t>(input_frames) * channels_ * sizeof(float));

    const std::uint32_t total_frames = kHistoryFrames + input_frames;
    std::uint32_t index = kHistoryFrames + skip_;
    std::uint32_t phase = phase_;
    std::uint32_t produced = 0;

    while (index < total_frames && produced < output_capacity_frames) {
        const float* phase_taps =
            taps_.get() + static_cast<std::size_t>(phase) * kResamplerTapsPerPhase;
        float* destination = interleaved_output + static_cast<std::size_t>(produced) * channels_;

        for (std::uint32_t channel = 0; channel < channels_; ++channel) {
            const float* sample = work + static_cast<std::size_t>(index) * channels_ + channel;
            float accumulator = 0.0f;
            for (std::uint32_t tap = 0; tap < kResamplerTapsPerPhase; ++tap) {
                accumulator += phase_taps[tap] * sample[-static_cast<std::ptrdiff_t>(tap) *
                                                        static_cast<std::ptrdiff_t>(channels_)];
            }
            destination[channel] = accumulator;
        }

        ++produced;
        phase += decimation_;
        index += phase / interpolation_;
        phase %= interpolation_;
    }

    skip_ = index > total_frames ? index - total_frames : 0;
    phase_ = phase;

    std::memmove(work, work + static_cast<std::size_t>(input_frames) * channels_,
                 static_cast<std::size_t>(kHistoryFrames) * channels_ * sizeof(float));
    return produced;
}

Outcome AudioConverter::configure(const AudioFormat& input, const AudioFormat& output,
                                  std::uint32_t max_input_frames) noexcept
{
    configured_ = false;
    passthrough_ = false;
    decoded_.reset();
    mapped_.reset();
    resampled_.reset();

    if (!input.valid() || !output.valid() || max_input_frames == 0) {
        return fail(Status::InvalidArgument, "audio converter: format or block size is invalid");
    }

    if (input.channels > kMaxChannels || output.channels > kMaxChannels) {
        return fail(Status::NotSupported, "audio converter: channel count beyond the fixed budget");
    }

    input_ = input;
    output_ = output;
    max_input_frames_ = max_input_frames;

    if (input.sample_rate == output.sample_rate && input.channels == output.channels &&
        input.sample_format == output.sample_format) {
        passthrough_ = true;
        max_output_frames_ = max_input_frames;
        configured_ = true;
        return ok();
    }

    decoded_.reset(
        new (std::nothrow) float[static_cast<std::size_t>(max_input_frames) * input.channels]);
    mapped_.reset(
        new (std::nothrow) float[static_cast<std::size_t>(max_input_frames) * output.channels]);
    if (decoded_ == nullptr || mapped_ == nullptr) {
        return fail(Status::OutOfMemory, "audio converter: staging buffers");
    }

    TL_TRY(resampler_.configure(input.sample_rate, output.sample_rate, output.channels,
                                max_input_frames));

    max_output_frames_ = resampler_.max_output_frames(max_input_frames);

    if (!resampler_.bypass()) {
        resampled_.reset(new (
            std::nothrow) float[static_cast<std::size_t>(max_output_frames_) * output.channels]);
        if (resampled_ == nullptr) {
            return fail(Status::OutOfMemory, "audio converter: resampled buffer");
        }
    }

    build_channel_coefficients(input_, output_, coefficients_);
    configured_ = true;
    return ok();
}

void AudioConverter::reset() noexcept
{
    resampler_.reset();
}

void AudioConverter::decode(const std::byte* input, std::uint32_t frames) noexcept
{
    const std::size_t samples = static_cast<std::size_t>(frames) * input_.channels;
    float* destination = decoded_.get();

    if (input_.sample_format == SampleFormat::Float32) {
        std::memcpy(destination, input, samples * sizeof(float));
        return;
    }

    const std::int16_t* source = reinterpret_cast<const std::int16_t*>(input);
    for (std::size_t sample = 0; sample < samples; ++sample) {
        destination[sample] = static_cast<float>(source[sample]) * (1.0f / 32768.0f);
    }
}

void AudioConverter::map_channels(std::uint32_t frames) noexcept
{
    const std::uint32_t inputs = input_.channels;
    const std::uint32_t outputs = output_.channels;
    const float* source = decoded_.get();
    float* destination = mapped_.get();

    if (inputs == outputs) {
        std::memcpy(destination, source,
                    static_cast<std::size_t>(frames) * outputs * sizeof(float));
        return;
    }

    for (std::uint32_t frame = 0; frame < frames; ++frame) {
        const float* input_frame = source + static_cast<std::size_t>(frame) * inputs;
        float* output_frame = destination + static_cast<std::size_t>(frame) * outputs;
        for (std::uint32_t out_channel = 0; out_channel < outputs; ++out_channel) {
            const float* weights = coefficients_ + out_channel * kMaxChannels;
            float accumulator = 0.0f;
            for (std::uint32_t in_channel = 0; in_channel < inputs; ++in_channel) {
                accumulator += weights[in_channel] * input_frame[in_channel];
            }
            output_frame[out_channel] = accumulator;
        }
    }
}

void AudioConverter::encode(const float* source, std::uint32_t frames, std::byte* output) noexcept
{
    const std::size_t samples = static_cast<std::size_t>(frames) * output_.channels;

    if (output_.sample_format == SampleFormat::Float32) {
        std::memcpy(output, source, samples * sizeof(float));
        return;
    }

    std::int16_t* destination = reinterpret_cast<std::int16_t*>(output);
    for (std::size_t sample = 0; sample < samples; ++sample) {
        const float scaled = source[sample] * 32767.0f;
        const float clamped =
            scaled > 32767.0f ? 32767.0f : (scaled < -32768.0f ? -32768.0f : scaled);
        destination[sample] =
            static_cast<std::int16_t>(clamped >= 0.0f ? clamped + 0.5f : clamped - 0.5f);
    }
}

std::uint32_t AudioConverter::convert(const std::byte* input, std::uint32_t input_frames,
                                      std::byte* output,
                                      std::uint32_t output_capacity_frames) noexcept
{
    if (!configured_ || input == nullptr || output == nullptr || input_frames == 0 ||
        input_frames > max_input_frames_) {
        return 0;
    }

    if (passthrough_) {
        const std::uint32_t frames = std::min(input_frames, output_capacity_frames);
        std::memcpy(output, input, static_cast<std::size_t>(frames) * output_.bytes_per_frame());
        return frames;
    }

    decode(input, input_frames);
    map_channels(input_frames);

    if (resampler_.bypass()) {
        if (input_frames > output_capacity_frames) {
            return 0;
        }
        encode(mapped_.get(), input_frames, output);
        return input_frames;
    }

    const std::uint32_t produced =
        resampler_.process(mapped_.get(), input_frames, resampled_.get(), max_output_frames_);
    if (produced > output_capacity_frames) {
        return 0;
    }
    encode(resampled_.get(), produced, output);
    return produced;
}

std::uint32_t AudioConverter::convert_silence(std::uint32_t input_frames, std::byte* output,
                                              std::uint32_t output_capacity_frames) noexcept
{
    if (!configured_ || output == nullptr || input_frames == 0 ||
        input_frames > max_input_frames_) {
        return 0;
    }

    if (passthrough_) {
        const std::uint32_t frames = std::min(input_frames, output_capacity_frames);
        std::memset(output, 0, static_cast<std::size_t>(frames) * output_.bytes_per_frame());
        return frames;
    }

    std::memset(mapped_.get(), 0,
                static_cast<std::size_t>(input_frames) * output_.channels * sizeof(float));

    if (resampler_.bypass()) {
        if (input_frames > output_capacity_frames) {
            return 0;
        }
        encode(mapped_.get(), input_frames, output);
        return input_frames;
    }

    const std::uint32_t produced =
        resampler_.process(mapped_.get(), input_frames, resampled_.get(), max_output_frames_);
    if (produced > output_capacity_frames) {
        return 0;
    }
    encode(resampled_.get(), produced, output);
    return produced;
}

}  // namespace tl::audio
