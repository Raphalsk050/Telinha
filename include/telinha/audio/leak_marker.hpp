#pragma once

#include <cmath>
#include <cstddef>
#include <cstdint>

namespace tl::audio {

// A marca que o app toca pela mesma saida das vozes da chamada: dois tons seguidos, agudos demais
// para o ouvido. Se ela aparece no som do computador que vai para quem assiste, o som do Telinha
// esta vazando, seja qual for o motivo. O mesmo par de tons esta em desktop/renderer/app.js.
inline constexpr double kLeakMarkerFirstHz = 19000.0;
inline constexpr double kLeakMarkerSecondHz = 19600.0;
inline constexpr std::uint32_t kLeakMarkerToneMs = 200;

class LeakMarkerDetector {
public:
    void reset() noexcept
    {
        first_ = {};
        second_ = {};
        guard_ = {};
        filled_ = 0;
        window_cos_ = 1.0;
        window_sin_ = 0.0;
        stage_ = Stage::Idle;
        run_ = 0;
        slack_ = 0;
    }

    // Devolve true no bloco em que a marca inteira acabou de passar.
    [[nodiscard]] bool feed(const std::int16_t* interleaved, std::size_t frames,
                            std::uint16_t channels, std::uint32_t sample_rate) noexcept
    {
        if (channels == 0 || sample_rate < kMinSampleRate) {
            return false;
        }
        if (sample_rate != sample_rate_) {
            configure(sample_rate);
        }

        bool detected = false;
        for (std::size_t frame = 0; frame < frames; ++frame) {
            double sample = 0.0;
            for (std::uint16_t channel = 0; channel < channels; ++channel) {
                sample += interleaved[frame * channels + channel];
            }
            sample /= channels;

            // Janela de Hann: sem ela, um som grave e alto espalha energia ate os tons da marca.
            sample *= 0.5 - 0.5 * window_cos_;
            const double next_cos = window_cos_ * step_cos_ - window_sin_ * step_sin_;
            window_sin_ = window_sin_ * step_cos_ + window_cos_ * step_sin_;
            window_cos_ = next_cos;

            first_.push(sample);
            second_.push(sample);
            guard_.push(sample);
            if (++filled_ == block_frames_) {
                detected = close_block() || detected;
            }
        }
        return detected;
    }

private:
    // Em blocos de 20 ms os tres tons medidos fecham ciclos inteiros, em qualquer taxa de
    // amostragem.
    static constexpr std::uint32_t kBlocksPerSecond = 50;
    static constexpr std::uint32_t kMinSampleRate = 44100;
    static constexpr double kGuardHz = 18400.0;
    static constexpr double kMinAmplitude = 8.0;
    static constexpr double kDominance = 16.0;
    static constexpr std::uint32_t kMinRun = 6;
    static constexpr std::uint32_t kMaxRun = 14;
    static constexpr std::uint32_t kMaxSlack = 2;
    static constexpr double kTwoPi = 6.283185307179586;

    enum class Stage : std::uint8_t { Idle, First, Between, Second, TooLong };
    enum class Tone : std::uint8_t { Neither, First, Second };

    struct Goertzel {
        double coefficient = 0.0;
        double s1 = 0.0;
        double s2 = 0.0;

        void push(double sample) noexcept
        {
            const double s0 = sample + coefficient * s1 - s2;
            s2 = s1;
            s1 = s0;
        }

        [[nodiscard]] double take_power() noexcept
        {
            const double power = s1 * s1 + s2 * s2 - coefficient * s1 * s2;
            s1 = 0.0;
            s2 = 0.0;
            return power;
        }
    };

    void configure(std::uint32_t sample_rate) noexcept
    {
        reset();
        sample_rate_ = sample_rate;
        block_frames_ = sample_rate / kBlocksPerSecond;
        step_cos_ = std::cos(kTwoPi / block_frames_);
        step_sin_ = std::sin(kTwoPi / block_frames_);
        first_.coefficient = 2.0 * std::cos(kTwoPi * kLeakMarkerFirstHz / sample_rate);
        second_.coefficient = 2.0 * std::cos(kTwoPi * kLeakMarkerSecondHz / sample_rate);
        guard_.coefficient = 2.0 * std::cos(kTwoPi * kGuardHz / sample_rate);
    }

    [[nodiscard]] bool close_block() noexcept
    {
        filled_ = 0;
        window_cos_ = 1.0;
        window_sin_ = 0.0;
        const double first = first_.take_power();
        const double second = second_.take_power();
        const double guard = guard_.take_power();
        // A janela de Hann deixa passar metade da amplitude de um tom.
        const double floor = kMinAmplitude * block_frames_ / 4.0;
        const double min_power = floor * floor;

        Tone tone = Tone::Neither;
        if (first >= min_power && first >= kDominance * second && first >= kDominance * guard) {
            tone = Tone::First;
        } else if (second >= min_power && second >= kDominance * first &&
                   second >= kDominance * guard) {
            tone = Tone::Second;
        }
        return advance(tone);
    }

    [[nodiscard]] bool advance(Tone tone) noexcept
    {
        switch (stage_) {
            case Stage::Idle: break;
            case Stage::First:
                if (tone == Tone::First) {
                    if (++run_ > kMaxRun) {
                        stage_ = Stage::TooLong;
                    }
                    return false;
                }
                if (run_ >= kMinRun && tone == Tone::Second) {
                    stage_ = Stage::Second;
                    run_ = 1;
                    return false;
                }
                if (run_ >= kMinRun) {
                    stage_ = Stage::Between;
                    slack_ = 1;
                    return false;
                }
                break;
            case Stage::Between:
                if (tone == Tone::Second) {
                    stage_ = Stage::Second;
                    run_ = 1;
                    return false;
                }
                if (tone == Tone::Neither && slack_ < kMaxSlack) {
                    ++slack_;
                    return false;
                }
                break;
            case Stage::Second:
                if (tone == Tone::Second) {
                    if (++run_ > kMaxRun) {
                        stage_ = Stage::TooLong;
                    }
                    return false;
                }
                if (run_ >= kMinRun) {
                    stage_ = Stage::Idle;
                    run_ = 0;
                    return true;
                }
                break;
            case Stage::TooLong:
                // Um tom continuo nao e a marca. So volta a procurar depois que ele parar.
                if (tone != Tone::Neither) {
                    return false;
                }
                break;
        }

        stage_ = tone == Tone::First ? Stage::First : Stage::Idle;
        run_ = tone == Tone::First ? 1 : 0;
        return false;
    }

    Goertzel first_;
    Goertzel second_;
    Goertzel guard_;
    std::uint32_t sample_rate_ = 0;
    std::uint32_t block_frames_ = 0;
    std::uint32_t filled_ = 0;
    double step_cos_ = 1.0;
    double step_sin_ = 0.0;
    double window_cos_ = 1.0;
    double window_sin_ = 0.0;
    Stage stage_ = Stage::Idle;
    std::uint32_t run_ = 0;
    std::uint32_t slack_ = 0;
};

}  // namespace tl::audio
