#pragma once

#include <atomic>
#include <cstdint>
#include <memory>

#include "telinha/app/app_options.hpp"
#include "telinha/app/signaling.hpp"
#include "telinha/receive/audio_renderer.hpp"
#include "telinha/receive/jitter_buffer.hpp"
#include "telinha/receive/packet_queue.hpp"
#include "telinha/receive/video_decoder.hpp"
#include "telinha/receive/video_renderer.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::app {

struct ReceiverCounters {
    std::atomic<std::uint64_t> video_frames{0};
    std::atomic<std::uint64_t> video_bytes{0};
    std::atomic<std::uint64_t> audio_blocks{0};
    std::atomic<std::uint64_t> audio_frames{0};
    std::atomic<std::uint64_t> video_dropped{0};
    std::atomic<std::uint64_t> audio_dropped{0};
    std::atomic<std::uint64_t> round_trip_ns{0};
};

class ReceiverSession final : public transport::TransportObserver {
public:
    ReceiverSession() noexcept;
    ~ReceiverSession() override;

    ReceiverSession(const ReceiverSession&) = delete;
    ReceiverSession& operator=(const ReceiverSession&) = delete;

    [[nodiscard]] Outcome initialize(const ReceiverOptions& options);
    [[nodiscard]] Outcome run();

    void request_stop() noexcept;

    void on_connection_state_changed(transport::ConnectionState state) noexcept override;
    void on_target_bitrate_changed(std::uint32_t bits_per_second,
                                   std::uint32_t framerate_hz) noexcept override;
    void on_keyframe_requested() noexcept override;
    void on_remote_video(const transport::EncodedVideoFrame& frame) noexcept override;
    void on_remote_audio(const transport::PcmAudioBlock& block) noexcept override;
    void on_local_description(Span<const char> description) noexcept override;
    void on_local_candidate(Span<const char> candidate) noexcept override;
    void on_gathering_complete() noexcept override;
    void on_round_trip_time(Nanoseconds round_trip_ns) noexcept override;

private:
    [[nodiscard]] Outcome negotiate();
    [[nodiscard]] Outcome publish_local(const char* label);
    [[nodiscard]] Outcome await_connection();
    [[nodiscard]] Outcome ensure_decoder(const receive::VideoPacketHeader& header);

    void drain_audio(Nanoseconds local_now_ns);
    void drain_video() noexcept;
    void feed_decoder(Nanoseconds local_now_ns);
    void present_next();
    void report(Nanoseconds local_now_ns);

    ReceiverOptions options_;
    std::unique_ptr<transport::MediaTransport> transport_;
    std::unique_ptr<receive::VideoRenderer> renderer_;
    std::unique_ptr<receive::AudioRenderer> audio_renderer_;
    std::unique_ptr<receive::VideoDecoder> decoder_;
    receive::VideoPacketQueue video_queue_;
    receive::AudioPacketQueue audio_queue_;
    receive::VideoJitterBuffer jitter_;
    receive::ClockOffsetEstimator offset_;
    SignalingCollector signaling_;
    std::unique_ptr<char[]> token_;
    std::unique_ptr<transport::SessionBlob> remote_blob_;
    ReceiverCounters counters_;
    std::uint64_t decode_submits_ = 0;
    std::uint64_t decode_failures_ = 0;
    std::uint64_t audio_submit_failures_ = 0;
    Nanoseconds last_report_ns_ = 0;
    std::atomic<std::uint32_t> state_{0};
    std::atomic<bool> stop_{false};
};

}  // namespace tl::app
