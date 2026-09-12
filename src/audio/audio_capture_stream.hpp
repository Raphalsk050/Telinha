#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>

#include "audio_converter.hpp"
#include "packet_ring.hpp"
#include "telinha/audio/audio_format.hpp"
#include "telinha/audio/captured_audio.hpp"
#include "telinha/core/clock.hpp"
#include "telinha/core/latency_histogram.hpp"
#include "telinha/core/result.hpp"

namespace tl::audio {

struct AudioCaptureCounters {
    LatencyHistogram packet_interval_ns;
    LatencyHistogram convert_ns;

    std::uint64_t packets_captured = 0;
    std::uint64_t packets_silent = 0;
    std::uint64_t packets_discontinuous = 0;
    std::uint64_t packets_synthesised = 0;
    std::uint64_t packets_dropped_ring_full = 0;
    std::uint64_t frames_captured = 0;
    std::uint64_t device_changes = 0;
};

class AudioCaptureStream {
public:
    AudioCaptureStream() noexcept = default;

    AudioCaptureStream(const AudioCaptureStream&) = delete;
    AudioCaptureStream& operator=(const AudioCaptureStream&) = delete;

    [[nodiscard]] Outcome configure(const AudioFormat& device_format,
                                    const AudioFormat& transmit_format,
                                    std::uint32_t max_device_frames,
                                    Nanoseconds ring_capacity_ns) noexcept;

    void reset() noexcept;

    [[nodiscard]] Outcome rebind_device_format(const AudioFormat& device_format,
                                               std::uint32_t max_device_frames) noexcept;

    [[nodiscard]] bool configured() const noexcept { return configured_; }
    [[nodiscard]] AudioFormat transmit_format() const noexcept { return transmit_format_; }
    [[nodiscard]] AudioFormat device_format() const noexcept { return device_format_; }
    [[nodiscard]] std::uint32_t max_device_frames() const noexcept { return max_device_frames_; }

    [[nodiscard]] bool submit(const std::byte* device_samples, std::uint32_t device_frames,
                              Nanoseconds device_time_ns, Nanoseconds capture_time_ns,
                              bool timestamp_valid, bool silent, bool discontinuity) noexcept;

    [[nodiscard]] bool submit_silence(std::uint32_t device_frames, Nanoseconds device_time_ns,
                                      Nanoseconds capture_time_ns) noexcept;

    void note_device_change() noexcept;

    [[nodiscard]] bool acquire(CapturedAudio& out) noexcept;
    void release() noexcept;

    [[nodiscard]] const AudioCaptureCounters& counters() const noexcept { return counters_; }

    [[nodiscard]] std::uint64_t packets_delivered() const noexcept { return packets_delivered_; }
    [[nodiscard]] std::uint64_t frames_delivered() const noexcept { return frames_delivered_; }

private:
    [[nodiscard]] bool publish(std::uint32_t produced_frames, Nanoseconds device_time_ns,
                               Nanoseconds capture_time_ns, bool timestamp_valid, bool silent,
                               bool discontinuity) noexcept;

    AudioConverter converter_;
    PacketRing ring_;

    std::unique_ptr<std::byte[]> scratch_;
    std::unique_ptr<std::byte[]> staging_;

    AudioFormat device_format_;
    AudioFormat transmit_format_;

    std::uint32_t max_device_frames_ = 0;
    std::uint32_t max_transmit_frames_ = 0;
    std::uint32_t staging_frames_ = 0;

    std::uint64_t packet_index_ = 0;
    std::uint64_t stream_position_frames_ = 0;
    Nanoseconds last_submit_ns_ = 0;

    AudioCaptureCounters counters_;

    std::uint64_t packets_delivered_ = 0;
    std::uint64_t frames_delivered_ = 0;

    bool pending_discontinuity_ = false;
    bool held_ = false;
    bool configured_ = false;
};

}  // namespace tl::audio
