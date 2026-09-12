#pragma once

#include <atomic>
#include <cstdint>
#include <thread>

#include "api/audio/audio_device.h"
#include "api/audio/audio_device_defines.h"
#include "modules/audio_device/include/audio_device_default.h"
#include "telinha/core/result.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::transport::backend {

class RemoteAudioSink {
public:
    virtual ~RemoteAudioSink() = default;

    virtual void on_remote_audio(const PcmAudioBlock& block) noexcept = 0;

protected:
    RemoteAudioSink() = default;
};

class PushAudioDevice
    : public webrtc::webrtc_impl::AudioDeviceModuleDefault<webrtc::AudioDeviceModule> {
public:
    static constexpr std::uint32_t kMaxSampleRateHz = 48000;
    static constexpr std::uint16_t kMaxChannels = 2;
    static constexpr std::size_t kMaxSamplesPerBlock =
        static_cast<std::size_t>(kMaxSampleRateHz) / 100u * kMaxChannels;
    static constexpr std::uint32_t kPlayoutSampleRateHz = 48000;
    static constexpr std::uint16_t kPlayoutChannels = 2;

    ~PushAudioDevice() override;

    void set_remote_audio_sink(RemoteAudioSink* sink) noexcept;

    int32_t RegisterAudioCallback(webrtc::AudioTransport* audio_callback) override;

    int32_t Init() override;
    int32_t Terminate() override;
    [[nodiscard]] bool Initialized() const override;

    int32_t InitRecording() override;
    [[nodiscard]] bool RecordingIsInitialized() const override;
    int32_t StartRecording() override;
    int32_t StopRecording() override;
    [[nodiscard]] bool Recording() const override;

    int32_t InitPlayout() override;
    [[nodiscard]] bool PlayoutIsInitialized() const override;
    int32_t StartPlayout() override;
    int32_t StopPlayout() override;
    [[nodiscard]] bool Playing() const override;

    [[nodiscard]] Outcome submit(const PcmAudioBlock& block) noexcept;

    [[nodiscard]] std::uint64_t blocks_delivered() const noexcept
    {
        return blocks_delivered_.load(std::memory_order_relaxed);
    }
    [[nodiscard]] std::uint64_t blocks_dropped() const noexcept
    {
        return blocks_dropped_.load(std::memory_order_relaxed);
    }

private:
    void deliver(const std::int16_t* samples, std::size_t frames_per_channel,
                 std::uint32_t sample_rate_hz, std::uint16_t channels) noexcept;
    void reset_pending(std::uint32_t sample_rate_hz, std::uint16_t channels) noexcept;
    void run_playout() noexcept;
    void stop_playout_thread() noexcept;

    std::atomic<webrtc::AudioTransport*> audio_transport_{nullptr};
    std::atomic<RemoteAudioSink*> remote_audio_sink_{nullptr};
    std::atomic<bool> initialized_{false};
    std::atomic<bool> recording_initialized_{false};
    std::atomic<bool> recording_{false};
    std::atomic<bool> playout_initialized_{false};
    std::atomic<bool> playing_{false};
    std::atomic<std::uint64_t> blocks_delivered_{0};
    std::atomic<std::uint64_t> blocks_dropped_{0};

    std::thread playout_thread_;

    std::int16_t pending_[kMaxSamplesPerBlock] = {};
    std::size_t pending_samples_ = 0;
    std::uint32_t pending_sample_rate_hz_ = 0;
    std::uint16_t pending_channels_ = 0;
};

}  // namespace tl::transport::backend
