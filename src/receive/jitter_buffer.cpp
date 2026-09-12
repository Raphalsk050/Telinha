#include "telinha/receive/jitter_buffer.hpp"

#include <new>

namespace tl::receive {
namespace {
constexpr Nanoseconds kGrowShift = 3;
constexpr Nanoseconds kShrinkShift = 5;
constexpr Nanoseconds kAudioSmoothing = 16;
}  // namespace

const char* to_string(JitterPull pull) noexcept
{
    switch (pull) {
        case JitterPull::Empty: return "Empty";
        case JitterPull::Waiting: return "Waiting";
        case JitterPull::Ready: return "Ready";
        case JitterPull::Discard: return "Discard";
    }
    return "Unknown";
}

VideoJitterBuffer::~VideoJitterBuffer() = default;

Outcome VideoJitterBuffer::reserve(const JitterConfig& config)
{
    if (config.capacity < 2) {
        return fail(Status::InvalidArgument, "VideoJitterBuffer::reserve");
    }
    if (config.min_delay_ns > config.max_delay_ns) {
        return fail(Status::InvalidArgument, "VideoJitterBuffer::reserve: delay range");
    }

    entries_.reset(new (std::nothrow) VideoPacketHeader[config.capacity]);
    if (!entries_) {
        return fail(Status::OutOfMemory, "VideoJitterBuffer::reserve");
    }

    config_ = config;
    capacity_ = config.capacity;
    reset();
    return ok();
}

void VideoJitterBuffer::reset() noexcept
{
    size_ = 0;
    next_index_ = 0;
    delay_ns_ = config_.initial_delay_ns;
    audio_delay_ns_ = 0;
    audio_delay_valid_ = false;
    started_ = false;
    keyframe_requested_ = false;
    stats_.reset();
    stats_.current_delay_ns = delay_ns_;
}

void VideoJitterBuffer::pop_front() noexcept
{
    for (std::uint32_t index = 1; index < size_; ++index) {
        entries_[index - 1] = entries_[index];
    }
    --size_;
}

bool VideoJitterBuffer::insert(const VideoPacketHeader& header,
                               std::uint32_t& evicted_slot) noexcept
{
    evicted_slot = kInvalidSlot;
    ++stats_.inserted;

    if (started_ && header.frame_index < next_index_) {
        ++stats_.late;
        evicted_slot = header.slot;
        return false;
    }

    for (std::uint32_t index = 0; index < size_; ++index) {
        if (entries_[index].frame_index == header.frame_index) {
            ++stats_.duplicates;
            evicted_slot = header.slot;
            return false;
        }
    }

    if (size_ == capacity_) {
        ++stats_.overflow;
        evicted_slot = entries_[0].slot;
        pop_front();
        if (!keyframe_requested_) {
            keyframe_requested_ = true;
            ++stats_.keyframes_requested;
        }
    }

    std::uint32_t position = size_;
    while (position > 0 && entries_[position - 1].frame_index > header.frame_index) {
        entries_[position] = entries_[position - 1];
        --position;
    }
    entries_[position] = header;
    ++size_;
    return true;
}

Nanoseconds VideoJitterBuffer::playout_ns(const VideoPacketHeader& header,
                                          const ClockOffsetEstimator& offset) const noexcept
{
    return offset.to_local(header.remote_time_ns) + delay_ns_;
}

JitterPull VideoJitterBuffer::pull(Nanoseconds local_now_ns, const ClockOffsetEstimator& offset,
                                   VideoPacketHeader& out) noexcept
{
    if (size_ == 0) {
        return JitterPull::Empty;
    }

    const VideoPacketHeader front = entries_[0];

    if (!started_ && !front.keyframe) {
        out = front;
        pop_front();
        ++stats_.frames_skipped;
        if (!keyframe_requested_) {
            keyframe_requested_ = true;
            ++stats_.keyframes_requested;
        }
        return JitterPull::Discard;
    }

    const Nanoseconds deadline = offset.ready() ? playout_ns(front, offset) : local_now_ns;

    if (started_ && front.frame_index != next_index_) {
        const Nanoseconds reorder_deadline = front.arrival_time_ns + config_.reorder_wait_ns;
        if (local_now_ns < reorder_deadline && local_now_ns < deadline) {
            return JitterPull::Waiting;
        }
        ++stats_.gaps;
        stats_.frames_skipped += front.frame_index - next_index_;
        if (!front.keyframe && !keyframe_requested_) {
            keyframe_requested_ = true;
            ++stats_.keyframes_requested;
        }
    }

    if (local_now_ns < deadline) {
        return JitterPull::Waiting;
    }

    out = front;
    pop_front();
    started_ = true;
    next_index_ = out.frame_index + 1;
    ++stats_.emitted;
    stats_.queue_delay_ns.record(local_now_ns - out.arrival_time_ns);
    return JitterPull::Ready;
}

void VideoJitterBuffer::observe_audio_delay(Nanoseconds measured_delay_ns) noexcept
{
    if (!audio_delay_valid_) {
        audio_delay_ns_ = measured_delay_ns;
        audio_delay_valid_ = true;
        return;
    }
    audio_delay_ns_ =
        (audio_delay_ns_ * (kAudioSmoothing - 1) + measured_delay_ns) / kAudioSmoothing;
}

void VideoJitterBuffer::update_delay(Nanoseconds network_spread_ns) noexcept
{
    Nanoseconds target = network_spread_ns + config_.delay_margin_ns;
    if (audio_delay_valid_ && audio_delay_ns_ > target) {
        target = audio_delay_ns_;
    }
    if (target < config_.min_delay_ns) {
        target = config_.min_delay_ns;
    }
    if (target > config_.max_delay_ns) {
        target = config_.max_delay_ns;
    }

    if (target > delay_ns_) {
        delay_ns_ += (target - delay_ns_ + ((1ull << kGrowShift) - 1)) >> kGrowShift;
    } else {
        delay_ns_ -= (delay_ns_ - target) >> kShrinkShift;
    }
    stats_.current_delay_ns = delay_ns_;
}

bool VideoJitterBuffer::take_keyframe_request() noexcept
{
    const bool requested = keyframe_requested_;
    keyframe_requested_ = false;
    return requested;
}

}  // namespace tl::receive
