#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <memory>
#include <mutex>
#include <thread>

#include "telinha/app/app_options.hpp"
#include "telinha/app/machine_events.hpp"
#include "telinha/app/signaling.hpp"
#include "telinha/audio/audio_source.hpp"
#include "telinha/capture/capture_pipeline.hpp"
#include "telinha/encode/video_encoder.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::app {

inline constexpr std::uint32_t kMaxSenderPeers = 8;

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

class SenderSession final {
public:
    SenderSession() noexcept;
    ~SenderSession();

    SenderSession(const SenderSession&) = delete;
    SenderSession& operator=(const SenderSession&) = delete;

    [[nodiscard]] Outcome initialize(const SenderOptions& options);
    [[nodiscard]] Outcome run();

    void request_stop() noexcept;

private:
    struct Peer;

    class PeerObserver final : public transport::TransportObserver {
    public:
        PeerObserver() noexcept = default;

        void bind(SenderSession& session, Peer& peer) noexcept
        {
            session_ = &session;
            peer_ = &peer;
        }

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
        SenderSession* session_ = nullptr;
        Peer* peer_ = nullptr;
    };

    enum class PeerPhase : std::uint8_t {
        Gathering = 0,
        AwaitingAnswer,
        Connecting,
        Connected,
    };

    struct Peer {
        explicit Peer(SenderSession& session) noexcept { observer.bind(session, *this); }

        PeerObserver observer;
        SignalingCollector signaling;
        std::unique_ptr<char[]> token;
        std::unique_ptr<transport::SessionBlob> remote_blob;
        char id[kPeerIdCapacity] = {};
        Nanoseconds deadline_ns = 0;
        PeerPhase phase = PeerPhase::Gathering;
        PeerFormat format = PeerFormat::Token;
        std::atomic<std::uint32_t> state{0};
        std::atomic<std::uint32_t> target_bitrate_bps{0};
        std::atomic<bool> detached{false};
        std::unique_ptr<transport::MediaTransport> media;
    };

    [[nodiscard]] Outcome resolve_target(capture::CaptureTargetInfo& out);
    [[nodiscard]] Outcome open_capture();
    [[nodiscard]] Outcome open_encoder(const capture::CaptureSourceInfo& info);
    [[nodiscard]] Outcome open_video();
    [[nodiscard]] Outcome open_audio();
    [[nodiscard]] Outcome create_peer(std::unique_ptr<Peer>& out);
    [[nodiscard]] Outcome open_transport(Peer& peer);
    [[nodiscard]] Outcome negotiate();
    [[nodiscard]] Outcome await_connection();
    [[nodiscard]] Outcome run_multi();

    [[nodiscard]] Outcome switch_target(const capture::CaptureTarget& target);
    [[nodiscard]] Outcome apply_quality(const MachineCommand& command);
    [[nodiscard]] Outcome switch_audio(const MachineCommand& command);
    void handle_command(const MachineCommand& command);
    void close_video() noexcept;

    void add_peer(const MachineCommand& command);
    void answer_peer(const MachineCommand& command);
    void remove_peer(const MachineCommand& command);
    [[nodiscard]] Outcome start_peer(Peer& peer);
    void service_peers();
    void publish_invite(std::uint32_t slot);
    void fail_peer(std::uint32_t slot, const Outcome& reason);
    void report_peer_failure(const char* id, const Outcome& reason);
    [[nodiscard]] std::unique_ptr<Peer> detach_peer(std::uint32_t slot) noexcept;
    static void destroy_peer(std::unique_ptr<Peer> peer) noexcept;
    [[nodiscard]] int find_peer(const char* id) const noexcept;
    [[nodiscard]] static bool peer_connected(const Peer& peer) noexcept;
    [[nodiscard]] bool peer_sendable(const Peer& peer) const noexcept;
    [[nodiscard]] std::uint32_t connected_peers() const noexcept;
    [[nodiscard]] std::uint32_t lowest_peer_bitrate() const noexcept;

    void apply_pending_controls() noexcept;
    void apply_peer_bitrate() noexcept;
    void send_encoded(const transport::EncodedVideoFrame& frame);
    void drain_encoder(std::uint32_t first_timeout_ms);
    void note_encoder_refusal(const Outcome& refused) noexcept;
    [[nodiscard]] std::uint32_t frame_rate_millihertz() const noexcept;
    void pump_video();
    void audio_thread_main();
    void report(Nanoseconds local_now_ns);

    SenderOptions options_;
    transport::IceServer ice_servers_[kMaxIceServers] = {};
    std::unique_ptr<capture::CapturePipeline> pipeline_;
    std::unique_ptr<encode::VideoEncoder> encoder_;
    std::unique_ptr<audio::AudioSource> audio_source_;
    std::unique_ptr<Peer> peers_[kMaxSenderPeers];
    SenderCounters counters_;

    std::unique_ptr<std::int16_t[]> audio_scratch_;
    std::size_t audio_scratch_frames_ = 0;
    std::thread audio_thread_;
    std::mutex send_mutex_;

    LatencyHistogram capture_ns_;
    LatencyHistogram encode_ns_;
    LatencyHistogram send_ns_;

    std::uint32_t max_fps_ = 0;
    std::uint32_t max_width_ = 0;
    std::uint32_t max_height_ = 0;
    std::uint32_t max_bitrate_bps_ = 0;
    std::uint32_t applied_bitrate_bps_ = 0;
    Nanoseconds next_frame_ns_ = 0;
    Nanoseconds last_refusal_log_ns_ = 0;
    std::uint32_t encoder_refusals_ = 0;

    Nanoseconds last_report_ns_ = 0;
    std::atomic<std::uint32_t> pending_bitrate_{0};
    std::atomic<bool> keyframe_pending_{false};
    std::atomic<bool> audio_stop_{false};
    std::atomic<bool> stop_{false};
    bool multi_ = false;
};

[[nodiscard]] int run_sender(const SenderOptions& options) noexcept;

}  // namespace tl::app
