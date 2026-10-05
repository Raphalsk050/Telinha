#include <cmath>
#include <cstddef>
#include <cstdint>
#include <vector>

#include "telinha/audio/leak_marker.hpp"
#include "test_framework.hpp"

using namespace tl;

namespace {

constexpr double kTwoPi = 6.283185307179586;
constexpr std::uint16_t kChannels = 2;

struct Signal {
    explicit Signal(std::uint32_t rate) : sample_rate(rate) {}

    // Soma um tom ao trecho que comeca em start_ms, esticando o sinal se precisar.
    void add_tone(double hz, std::uint32_t start_ms, std::uint32_t duration_ms, double amplitude)
    {
        const std::size_t begin = static_cast<std::size_t>(sample_rate) * start_ms / 1000;
        const std::size_t count = static_cast<std::size_t>(sample_rate) * duration_ms / 1000;
        if (frames.size() < begin + count) {
            frames.resize(begin + count, 0.0);
        }
        for (std::size_t index = 0; index < count; ++index) {
            frames[begin + index] +=
                amplitude * std::sin(kTwoPi * hz * static_cast<double>(index) / sample_rate);
        }
    }

    void add_marker(std::uint32_t start_ms, double amplitude)
    {
        add_tone(audio::kLeakMarkerFirstHz, start_ms, audio::kLeakMarkerToneMs, amplitude);
        add_tone(audio::kLeakMarkerSecondHz, start_ms + audio::kLeakMarkerToneMs,
                 audio::kLeakMarkerToneMs, amplitude);
    }

    // Entrega em pedacos que nao casam com o bloco do detector, como a captura de verdade.
    [[nodiscard]] std::uint32_t detections(std::size_t chunk_frames = 441)
    {
        add_tone(1000.0, static_cast<std::uint32_t>(frames.size() * 1000 / sample_rate), 200, 0.0);

        std::vector<std::int16_t> interleaved(frames.size() * kChannels);
        for (std::size_t index = 0; index < frames.size(); ++index) {
            const auto sample = static_cast<std::int16_t>(std::lround(frames[index]));
            interleaved[index * kChannels] = sample;
            interleaved[index * kChannels + 1] = sample;
        }

        audio::LeakMarkerDetector detector;
        std::uint32_t found = 0;
        for (std::size_t offset = 0; offset < frames.size(); offset += chunk_frames) {
            const std::size_t count =
                frames.size() - offset < chunk_frames ? frames.size() - offset : chunk_frames;
            if (detector.feed(interleaved.data() + offset * kChannels, count, kChannels,
                              sample_rate)) {
                ++found;
            }
        }
        return found;
    }

    std::uint32_t sample_rate;
    std::vector<double> frames;
};

}  // namespace

TEST_CASE("leak_marker", "finds the marker at both common sample rates")
{
    for (const std::uint32_t rate : {48000u, 44100u}) {
        Signal signal(rate);
        signal.add_marker(300, 130.0);
        CHECK_EQ(signal.detections(), 1u);
        CHECK_EQ(signal.detections(480), 1u);
    }
}

TEST_CASE("leak_marker", "finds a quiet marker under loud program audio")
{
    Signal signal(48000);
    signal.add_tone(440.0, 0, 1500, 9000.0);
    signal.add_tone(3000.0, 0, 1500, 6000.0);
    signal.add_marker(500, 40.0);
    CHECK_EQ(signal.detections(), 1u);
}

TEST_CASE("leak_marker", "counts each marker once")
{
    Signal signal(48000);
    signal.add_marker(100, 130.0);
    signal.add_marker(1100, 130.0);
    CHECK_EQ(signal.detections(), 2u);
}

TEST_CASE("leak_marker", "ignores sounds that are not the marker")
{
    Signal silence(48000);
    silence.add_tone(1000.0, 0, 1000, 0.0);
    CHECK_EQ(silence.detections(), 0u);

    Signal steady(48000);
    steady.add_tone(audio::kLeakMarkerFirstHz, 0, 2000, 130.0);
    CHECK_EQ(steady.detections(), 0u);

    Signal first_only(48000);
    first_only.add_tone(audio::kLeakMarkerFirstHz, 100, audio::kLeakMarkerToneMs, 130.0);
    CHECK_EQ(first_only.detections(), 0u);

    Signal reversed(48000);
    reversed.add_tone(audio::kLeakMarkerSecondHz, 100, audio::kLeakMarkerToneMs, 130.0);
    reversed.add_tone(audio::kLeakMarkerFirstHz, 300, audio::kLeakMarkerToneMs, 130.0);
    CHECK_EQ(reversed.detections(), 0u);

    Signal too_short(48000);
    too_short.add_tone(audio::kLeakMarkerFirstHz, 100, 60, 130.0);
    too_short.add_tone(audio::kLeakMarkerSecondHz, 160, 60, 130.0);
    CHECK_EQ(too_short.detections(), 0u);

    Signal too_long(48000);
    too_long.add_tone(audio::kLeakMarkerFirstHz, 100, 600, 130.0);
    too_long.add_tone(audio::kLeakMarkerSecondHz, 700, 600, 130.0);
    CHECK_EQ(too_long.detections(), 0u);

    Signal both_at_once(48000);
    both_at_once.add_tone(audio::kLeakMarkerFirstHz, 100, 400, 130.0);
    both_at_once.add_tone(audio::kLeakMarkerSecondHz, 100, 400, 130.0);
    CHECK_EQ(both_at_once.detections(), 0u);

    Signal too_faint(48000);
    too_faint.add_marker(300, 2.0);
    CHECK_EQ(too_faint.detections(), 0u);
}
