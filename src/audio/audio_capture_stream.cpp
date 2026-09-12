#include "audio_capture_stream.hpp"

#include <algorithm>
#include <cstring>
#include <new>

namespace tl::audio {
namespace {

constexpr Nanoseconds kMinimumRingCapacityNs = 40 * kNanosecondsPerMillisecond;
constexpr Nanoseconds kMaximumRingCapacityNs = 4 * kNanosecondsPerSecond;

}  // namespace

Outcome AudioCaptureStream::configure(const AudioFormat& device_format,
                                      const AudioFormat& transmit_format,
                                      std::uint32_t max_device_frames,
                                      Nanoseconds ring_capacity_ns) noexcept
{
    configured_ = false;
    held_ = false;
    scratch_.reset();
    staging_.reset();
    ring_.release();

    if (!device_format.valid() || !transmit_format.valid() || max_device_frames == 0) {
        return fail(Status::InvalidArgument, "audio stream: format or block size is invalid");
    }

    TL_TRY(converter_.configure(device_format, transmit_format, max_device_frames));

    device_format_ = device_format;
    transmit_format_ = transmit_format;
    max_device_frames_ = max_device_frames;
    max_transmit_frames_ = converter_.max_output_frames();
    staging_frames_ = max_transmit_frames_;

    const std::size_t packet_bytes =
        static_cast<std::size_t>(max_transmit_frames_) * transmit_format.bytes_per_frame();
    if (packet_bytes == 0) {
        return fail(Status::InvalidArgument, "audio stream: transmit format has no payload");
    }

    scratch_.reset(new (std::nothrow) std::byte[packet_bytes]);
    staging_.reset(new (std::nothrow) std::byte[packet_bytes]);
    if (scratch_ == nullptr || staging_ == nullptr) {
        return fail(Status::OutOfMemory, "audio stream: packet buffers");
    }

    const Nanoseconds clamped =
        std::clamp(ring_capacity_ns, kMinimumRingCapacityNs, kMaximumRingCapacityNs);
    const std::uint64_t ring_frames = clamped * transmit_format.sample_rate / kNanosecondsPerSecond;
    const std::size_t header_overhead =
        (static_cast<std::size_t>(ring_frames) / std::max<std::size_t>(max_transmit_frames_, 1) +
         2) *
        sizeof(PacketHeader);
    const std::size_t ring_bytes =
        static_cast<std::size_t>(ring_frames) * transmit_format.bytes_per_frame() +
        header_overhead + packet_bytes;

    if (!ring_.reserve(ring_bytes)) {
        return fail(Status::OutOfMemory, "audio stream: packet ring");
    }

    packet_index_ = 0;
    stream_position_frames_ = 0;
    packets_delivered_ = 0;
    frames_delivered_ = 0;
    last_submit_ns_ = 0;
    pending_discontinuity_ = false;
    counters_ = AudioCaptureCounters{};
    configured_ = true;
    return ok();
}

void AudioCaptureStream::reset() noexcept
{
    converter_.reset();
    ring_.clear();
    held_ = false;
    pending_discontinuity_ = true;
    last_submit_ns_ = 0;
}

Outcome AudioCaptureStream::rebind_device_format(const AudioFormat& device_format,
                                                 std::uint32_t max_device_frames) noexcept
{
    if (!configured_) {
        return fail(Status::InvalidArgument, "audio stream: rebind before configure");
    }
    if (!device_format.valid() || max_device_frames == 0) {
        return fail(Status::InvalidArgument, "audio stream: rebind format is invalid");
    }

    AudioConverter replacement;
    TL_TRY(replacement.configure(device_format, transmit_format_, max_device_frames));

    if (replacement.max_output_frames() > staging_frames_) {
        return fail(Status::OutOfRange, "audio stream: rebind needs a larger packet budget");
    }

    TL_TRY(converter_.configure(device_format, transmit_format_, max_device_frames));
    device_format_ = device_format;
    max_device_frames_ = max_device_frames;
    max_transmit_frames_ = converter_.max_output_frames();
    pending_discontinuity_ = true;
    return ok();
}

void AudioCaptureStream::note_device_change() noexcept
{
    ++counters_.device_changes;
    pending_discontinuity_ = true;
}

bool AudioCaptureStream::publish(std::uint32_t produced_frames, Nanoseconds device_time_ns,
                                 Nanoseconds capture_time_ns, bool timestamp_valid, bool silent,
                                 bool discontinuity) noexcept
{
    if (produced_frames == 0) {
        return true;
    }

    PacketHeader header;
    header.frame_count = produced_frames;
    header.payload_bytes = produced_frames * transmit_format_.bytes_per_frame();
    header.stream_frame_position = stream_position_frames_;
    header.capture_time_ns = capture_time_ns;
    header.device_time_ns = device_time_ns;
    header.flags = 0;
    if (silent) {
        header.flags |= static_cast<std::uint32_t>(PacketFlag::Silent);
    }
    if (discontinuity || pending_discontinuity_) {
        header.flags |= static_cast<std::uint32_t>(PacketFlag::Discontinuity);
    }
    header.timestamp_valid = timestamp_valid ? 1u : 0u;

    while (!ring_.write(header, scratch_.get())) {
        if (!ring_.drop_oldest()) {
            ++counters_.packets_dropped_ring_full;
            return false;
        }
        ++counters_.packets_dropped_ring_full;
    }

    pending_discontinuity_ = false;
    stream_position_frames_ += produced_frames;
    ++packet_index_;
    return true;
}

bool AudioCaptureStream::submit(const std::byte* device_samples, std::uint32_t device_frames,
                                Nanoseconds device_time_ns, Nanoseconds capture_time_ns,
                                bool timestamp_valid, bool silent, bool discontinuity) noexcept
{
    if (!configured_ || device_frames == 0 || device_frames > max_device_frames_) {
        return false;
    }

    if (last_submit_ns_ != 0 && capture_time_ns > last_submit_ns_) {
        counters_.packet_interval_ns.record(capture_time_ns - last_submit_ns_);
    }
    last_submit_ns_ = capture_time_ns;

    const Nanoseconds convert_start = now_ns();
    const std::uint32_t produced =
        silent || device_samples == nullptr
            ? converter_.convert_silence(device_frames, scratch_.get(), max_transmit_frames_)
            : converter_.convert(device_samples, device_frames, scratch_.get(),
                                 max_transmit_frames_);
    counters_.convert_ns.record(now_ns() - convert_start);

    ++counters_.packets_captured;
    counters_.frames_captured += device_frames;
    if (silent) {
        ++counters_.packets_silent;
    }
    if (discontinuity) {
        ++counters_.packets_discontinuous;
    }

    return publish(produced, device_time_ns, capture_time_ns, timestamp_valid, silent,
                   discontinuity);
}

bool AudioCaptureStream::submit_silence(std::uint32_t device_frames, Nanoseconds device_time_ns,
                                        Nanoseconds capture_time_ns) noexcept
{
    if (!configured_ || device_frames == 0) {
        return false;
    }

    bool delivered = true;
    std::uint32_t remaining = device_frames;
    Nanoseconds cursor_ns = device_time_ns;

    while (remaining != 0) {
        const std::uint32_t chunk = std::min(remaining, max_device_frames_);
        const Nanoseconds convert_start = now_ns();
        const std::uint32_t produced =
            converter_.convert_silence(chunk, scratch_.get(), max_transmit_frames_);
        counters_.convert_ns.record(now_ns() - convert_start);

        ++counters_.packets_captured;
        ++counters_.packets_silent;
        ++counters_.packets_synthesised;
        counters_.frames_captured += chunk;

        delivered = publish(produced, cursor_ns, capture_time_ns, true, true, false) && delivered;

        cursor_ns +=
            static_cast<Nanoseconds>(chunk) * kNanosecondsPerSecond / device_format_.sample_rate;
        remaining -= chunk;
    }

    last_submit_ns_ = capture_time_ns;
    return delivered;
}

bool AudioCaptureStream::acquire(CapturedAudio& out) noexcept
{
    if (!configured_) {
        return false;
    }

    const std::uint32_t capacity_bytes = staging_frames_ * transmit_format_.bytes_per_frame();

    PacketHeader header;
    if (!ring_.read(header, staging_.get(), capacity_bytes)) {
        return false;
    }

    out.samples = Span<const std::byte>(staging_.get(), header.payload_bytes);
    out.format = transmit_format_;
    out.frame_count = header.frame_count;
    out.device_time_ns = header.device_time_ns;
    out.acquire_time_ns = header.capture_time_ns;
    out.stream_position_frames = header.stream_frame_position;
    out.silent = has_flag(header.flags, PacketFlag::Silent);
    out.discontinuity = has_flag(header.flags, PacketFlag::Discontinuity);
    out.timestamp_valid = header.timestamp_valid != 0;

    ++packets_delivered_;
    frames_delivered_ += header.frame_count;
    held_ = true;
    return true;
}

void AudioCaptureStream::release() noexcept
{
    held_ = false;
}

}  // namespace tl::audio
