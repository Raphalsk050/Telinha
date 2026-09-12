#pragma once

#include <cstdint>
#include <memory>

#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/latency_histogram.hpp"
#include "telinha/core/result.hpp"
#include "telinha/receive/clock_offset.hpp"
#include "telinha/receive/packet_queue.hpp"

namespace tl::receive {

inline constexpr std::uint32_t kInvalidSlot = 0xFFFFFFFFu;

enum class JitterPull : std::uint8_t {
    Empty = 0,
    Waiting,
    Ready,
    Discard,
};

const char* to_string(JitterPull pull) noexcept;

struct JitterConfig {
    Nanoseconds initial_delay_ns = 60ull * kNanosecondsPerMillisecond;
    Nanoseconds min_delay_ns = 15ull * kNanosecondsPerMillisecond;
    Nanoseconds max_delay_ns = 400ull * kNanosecondsPerMillisecond;
    Nanoseconds reorder_wait_ns = 25ull * kNanosecondsPerMillisecond;
    Nanoseconds delay_margin_ns = 15ull * kNanosecondsPerMillisecond;
    std::uint32_t capacity = 64;
};

struct JitterStats {
    LatencyHistogram queue_delay_ns;

    std::uint64_t inserted = 0;
    std::uint64_t emitted = 0;
    std::uint64_t duplicates = 0;
    std::uint64_t late = 0;
    std::uint64_t overflow = 0;
    std::uint64_t gaps = 0;
    std::uint64_t frames_skipped = 0;
    std::uint64_t keyframes_requested = 0;
    Nanoseconds current_delay_ns = 0;

    void reset() noexcept
    {
        queue_delay_ns.reset();
        inserted = 0;
        emitted = 0;
        duplicates = 0;
        late = 0;
        overflow = 0;
        gaps = 0;
        frames_skipped = 0;
        keyframes_requested = 0;
        current_delay_ns = 0;
    }
};

class VideoJitterBuffer {
public:
    VideoJitterBuffer() noexcept = default;
    ~VideoJitterBuffer();

    VideoJitterBuffer(const VideoJitterBuffer&) = delete;
    VideoJitterBuffer& operator=(const VideoJitterBuffer&) = delete;

    [[nodiscard]] Outcome reserve(const JitterConfig& config);

    [[nodiscard]] bool insert(const VideoPacketHeader& header,
                              std::uint32_t& evicted_slot) noexcept;

    [[nodiscard]] JitterPull pull(Nanoseconds local_now_ns, const ClockOffsetEstimator& offset,
                                  VideoPacketHeader& out) noexcept;

    void observe_audio_delay(Nanoseconds measured_delay_ns) noexcept;

    void update_delay(Nanoseconds network_spread_ns) noexcept;

    [[nodiscard]] bool take_keyframe_request() noexcept;

    void reset() noexcept;

    [[nodiscard]] std::uint32_t size() const noexcept { return size_; }
    [[nodiscard]] const JitterStats& stats() const noexcept { return stats_; }
    [[nodiscard]] Nanoseconds delay_ns() const noexcept { return delay_ns_; }

private:
    [[nodiscard]] Nanoseconds playout_ns(const VideoPacketHeader& header,
                                         const ClockOffsetEstimator& offset) const noexcept;

    void pop_front() noexcept;

    std::unique_ptr<VideoPacketHeader[]> entries_;
    JitterConfig config_;
    std::uint32_t capacity_ = 0;
    std::uint32_t size_ = 0;
    std::uint64_t next_index_ = 0;
    Nanoseconds delay_ns_ = 0;
    Nanoseconds audio_delay_ns_ = 0;
    JitterStats stats_;
    bool started_ = false;
    bool keyframe_requested_ = false;
    bool audio_delay_valid_ = false;
};

}  // namespace tl::receive
