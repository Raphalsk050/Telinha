#pragma once

#include <atomic>
#include <cstdint>
#include <memory>
#include <mutex>
#include <thread>

#include "telinha/app/app_options.hpp"
#include "telinha/app/signaling.hpp"
#include "telinha/audio/audio_source.hpp"
#include "telinha/capture/capture_pipeline.hpp"
#include "telinha/encode/video_encoder.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::app {

struct SenderCounters {
    std::atomic<std::uint64_t> frames_captured{0};
    std::atomic<std::uint64_t> frames_encoded{0};
    std::atomic<std::uint64_t> frames_sent{0};
    std::atomic<std::uint64_t> keyframes{0};
    std::atomic<std::uint64_t> video_bytes{0};
    std::atomic<std::uint64_t> audio_blocks{0};
    std::atomic<std::uint64_t> audio_bytes{0};
    std::atomic<std::uint64_t> send_failures{0};
    std::atomic<std::uint64_t> bitrate_updates{0};
};

class SenderSession final : public transport::TransportObserver {
public:
    SenderSession() noexcept;
    ~SenderSession() override;

    SenderSession(const SenderSession&) = delete;
    SenderSession& operator=(const SenderSession&) = delete;

    [[nodiscard]] Outcome initialize(const SenderOptions& options);
    [[nodiscard]] Outcome run();

    void request_stop() noexcept;

    void on_connection_state_changed(transport::ConnectionState state) noexcept override;
    void on_target_bitrate_changed(std::uint32_t bits_per_second,
                                   std::uint32_t framerate_hz) noexcept override;
    void on_keyframe_requested() noexcept override;
    void on_packet_loss_detected(double loss_ratio) noexcept override;
    void on_local_description(Span<const char> description) noexcept override;
    void on_local_candidate(Span<const char> candidate) noexcept override;
    void on_gathering_complete() noexcept override;
    void on_round_trip_time(Nanoseconds round_trip_ns) noexcept override;

private:
    [[nodiscard]] Outcome resolve_target(capture::CaptureTargetInfo& out);
    [[nodiscard]] Outcome open_capture();
    [[nodiscard]] Outcome open_encoder(const capture::CaptureSourceInfo& info);
    [[nodiscard]] Outcome open_audio();
    [[nodiscard]] Outcome open_transport();
    [[nodiscard]] Outcome negotiate();
    [[nodiscard]] Outcome await_connection();

    void apply_pending_controls() noexcept;
    void pump_video();
    void audio_thread_main();
    void report(Nanoseconds local_now_ns);

    SenderOptions options_;
    capture::CapturePipeline pipeline_;
    std::unique_ptr<encode::VideoEncoder> encoder_;
    std::unique_ptr<audio::AudioSource> audio_source_;
    std::unique_ptr<transport::MediaTransport> transport_;
    SignalingCollector signaling_;
    std::unique_ptr<char[]> token_;
    std::unique_ptr<transport::SessionBlob> remote_blob_;
    SenderCounters counters_;

    std::unique_ptr<std::int16_t[]> audio_scratch_;
    std::size_t audio_scratch_frames_ = 0;
    std::thread audio_thread_;
    std::mutex send_mutex_;

    LatencyHistogram capture_ns_;
    LatencyHistogram encode_ns_;
    LatencyHistogram send_ns_;

    Nanoseconds last_report_ns_ = 0;
    std::atomic<std::uint32_t> pending_bitrate_{0};
    std::atomic<std::uint32_t> state_{0};
    std::atomic<bool> keyframe_pending_{false};
    std::atomic<bool> stop_{false};
};

[[nodiscard]] int run_sender(const SenderOptions& options) noexcept;

}  // namespace tl::app
