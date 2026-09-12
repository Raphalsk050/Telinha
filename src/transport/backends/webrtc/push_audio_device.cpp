#include "backends/webrtc/push_audio_device.hpp"

#include <chrono>
#include <cstring>
#include <thread>

#include "telinha/core/clock.hpp"

namespace tl::transport::backend {

PushAudioDevice::~PushAudioDevice()
{
    stop_playout_thread();
}

void PushAudioDevice::set_remote_audio_sink(RemoteAudioSink* sink) noexcept
{
    remote_audio_sink_.store(sink, std::memory_order_release);
}

int32_t PushAudioDevice::RegisterAudioCallback(webrtc::AudioTransport* audio_callback)
{
    audio_transport_.store(audio_callback, std::memory_order_release);
    return 0;
}

int32_t PushAudioDevice::Init()
{
    initialized_.store(true, std::memory_order_release);
    return 0;
}

int32_t PushAudioDevice::Terminate()
{
    stop_playout_thread();
    playout_initialized_.store(false, std::memory_order_release);
    recording_.store(false, std::memory_order_release);
    recording_initialized_.store(false, std::memory_order_release);
    initialized_.store(false, std::memory_order_release);
    return 0;
}

bool PushAudioDevice::Initialized() const
{
    return initialized_.load(std::memory_order_acquire);
}

int32_t PushAudioDevice::InitRecording()
{
    recording_initialized_.store(true, std::memory_order_release);
    return 0;
}

bool PushAudioDevice::RecordingIsInitialized() const
{
    return recording_initialized_.load(std::memory_order_acquire);
}

int32_t PushAudioDevice::StartRecording()
{
    recording_.store(true, std::memory_order_release);
    return 0;
}

int32_t PushAudioDevice::StopRecording()
{
    recording_.store(false, std::memory_order_release);
    return 0;
}

bool PushAudioDevice::Recording() const
{
    return recording_.load(std::memory_order_acquire);
}

int32_t PushAudioDevice::InitPlayout()
{
    playout_initialized_.store(true, std::memory_order_release);
    return 0;
}

bool PushAudioDevice::PlayoutIsInitialized() const
{
    return playout_initialized_.load(std::memory_order_acquire);
}

int32_t PushAudioDevice::StartPlayout()
{
    if (playing_.exchange(true, std::memory_order_acq_rel)) {
        return 0;
    }
    playout_thread_ = std::thread([this] { run_playout(); });
    return 0;
}

int32_t PushAudioDevice::StopPlayout()
{
    stop_playout_thread();
    return 0;
}

bool PushAudioDevice::Playing() const
{
    return playing_.load(std::memory_order_acquire);
}

void PushAudioDevice::stop_playout_thread() noexcept
{
    playing_.store(false, std::memory_order_release);
    if (playout_thread_.joinable()) {
        playout_thread_.join();
    }
}

void PushAudioDevice::run_playout() noexcept
{
    constexpr std::size_t kFrames = kPlayoutSampleRateHz / 100u;
    alignas(64) std::int16_t buffer[kFrames * kPlayoutChannels] = {};

    auto next = std::chrono::steady_clock::now();
    while (playing_.load(std::memory_order_acquire)) {
        next += std::chrono::milliseconds(10);

        webrtc::AudioTransport* transport = audio_transport_.load(std::memory_order_acquire);
        if (transport != nullptr) {
            std::size_t produced = 0;
            std::int64_t elapsed_time_ms = -1;
            std::int64_t ntp_time_ms = -1;
            transport->NeedMorePlayData(kFrames, sizeof(std::int16_t) * kPlayoutChannels,
                                        kPlayoutChannels, kPlayoutSampleRateHz, buffer, produced,
                                        &elapsed_time_ms, &ntp_time_ms);

            RemoteAudioSink* sink = remote_audio_sink_.load(std::memory_order_acquire);
            if (sink != nullptr && produced > 0) {
                PcmAudioBlock block;
                block.interleaved = Span<const std::int16_t>(buffer, produced * kPlayoutChannels);
                block.capture_time_ns = now_ns();
                block.sample_rate_hz = kPlayoutSampleRateHz;
                block.frames_per_channel = static_cast<std::uint32_t>(produced);
                block.channels = kPlayoutChannels;
                sink->on_remote_audio(block);
            }
        }

        std::this_thread::sleep_until(next);
    }
}

void PushAudioDevice::reset_pending(std::uint32_t sample_rate_hz, std::uint16_t channels) noexcept
{
    pending_samples_ = 0;
    pending_sample_rate_hz_ = sample_rate_hz;
    pending_channels_ = channels;
}

void PushAudioDevice::deliver(const std::int16_t* samples, std::size_t frames_per_channel,
                              std::uint32_t sample_rate_hz, std::uint16_t channels) noexcept
{
    webrtc::AudioTransport* transport = audio_transport_.load(std::memory_order_acquire);
    if (transport == nullptr) {
        blocks_dropped_.fetch_add(1, std::memory_order_relaxed);
        return;
    }

    std::uint32_t new_microphone_level = 0;
    transport->RecordedDataIsAvailable(samples, frames_per_channel, sizeof(std::int16_t) * channels,
                                       channels, sample_rate_hz, 0, 0, 0, false,
                                       new_microphone_level);
    blocks_delivered_.fetch_add(1, std::memory_order_relaxed);
}

Outcome PushAudioDevice::submit(const PcmAudioBlock& block) noexcept
{
    if (!block.valid()) {
        return fail(Status::InvalidArgument, "submit: empty audio block");
    }
    if (block.channels == 0 || block.channels > kMaxChannels) {
        return fail(Status::NotSupported, "submit: only mono and stereo are supported");
    }
    if (block.sample_rate_hz == 0 || block.sample_rate_hz > kMaxSampleRateHz ||
        block.sample_rate_hz % 100u != 0) {
        return fail(Status::NotSupported, "submit: sample rate must divide into 10 ms blocks");
    }
    if (block.interleaved.size() !=
        static_cast<std::size_t>(block.frames_per_channel) * block.channels) {
        return fail(Status::InvalidArgument, "submit: sample count disagrees with the layout");
    }
    if (!recording_.load(std::memory_order_acquire)) {
        blocks_dropped_.fetch_add(1, std::memory_order_relaxed);
        return ok();
    }

    if (block.sample_rate_hz != pending_sample_rate_hz_ || block.channels != pending_channels_) {
        reset_pending(block.sample_rate_hz, block.channels);
    }

    const std::size_t block_frames = block.sample_rate_hz / 100u;
    const std::size_t block_samples = block_frames * block.channels;

    const std::int16_t* input = block.interleaved.data();
    std::size_t remaining = block.interleaved.size();

    if (pending_samples_ == 0 && remaining == block_samples) {
        deliver(input, block_frames, block.sample_rate_hz, block.channels);
        return ok();
    }

    while (remaining > 0) {
        const std::size_t wanted = block_samples - pending_samples_;
        const std::size_t taken = remaining < wanted ? remaining : wanted;
        std::memcpy(pending_ + pending_samples_, input, taken * sizeof(std::int16_t));
        pending_samples_ += taken;
        input += taken;
        remaining -= taken;

        if (pending_samples_ == block_samples) {
            deliver(pending_, block_frames, block.sample_rate_hz, block.channels);
            pending_samples_ = 0;
        }
    }

    return ok();
}

}  // namespace tl::transport::backend
