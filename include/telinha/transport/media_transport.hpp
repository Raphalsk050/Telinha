#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>

#include "telinha/core/clock.hpp"
#include "telinha/core/result.hpp"
#include "telinha/core/span.hpp"

namespace tl::transport {

enum class TransportRole : std::uint8_t {
    Sender = 0,
    Receiver,
};

const char* to_string(TransportRole role) noexcept;

enum class ConnectionState : std::uint8_t {
    New = 0,
    Gathering,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
};

const char* to_string(ConnectionState state) noexcept;

enum class CandidatePolicy : std::uint8_t {
    HostOnly = 0,
    HostAndServerReflexive,
    All,
};

const char* to_string(CandidatePolicy policy) noexcept;

enum class WireVideoCodec : std::uint8_t {
    Unknown = 0,
    H264,
    Vp8,
    Vp9,
    Av1,
};

const char* to_string(WireVideoCodec codec) noexcept;

enum class WireFrameKind : std::uint8_t {
    Delta = 0,
    Key,
};

const char* to_string(WireFrameKind kind) noexcept;

struct EncodedVideoFrame {
    Span<const std::byte> bitstream;
    Nanoseconds capture_time_ns = 0;
    Nanoseconds encode_end_time_ns = 0;
    std::uint64_t frame_index = 0;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    std::int32_t average_qp = 0;
    WireVideoCodec codec = WireVideoCodec::Unknown;
    WireFrameKind kind = WireFrameKind::Delta;
    std::uint8_t temporal_index = 0;
    std::uint8_t spatial_index = 0;

    [[nodiscard]] bool valid() const noexcept { return !bitstream.empty(); }
};

struct PcmAudioBlock {
    Span<const std::int16_t> interleaved;
    Nanoseconds capture_time_ns = 0;
    std::uint32_t sample_rate_hz = 0;
    std::uint32_t frames_per_channel = 0;
    std::uint16_t channels = 0;

    [[nodiscard]] bool valid() const noexcept { return !interleaved.empty(); }
};

struct IceServer {
    const char* url = nullptr;
    const char* username = nullptr;
    const char* credential = nullptr;
};

struct TransportConfig {
    TransportRole role = TransportRole::Sender;
    CandidatePolicy candidate_policy = CandidatePolicy::HostAndServerReflexive;
    Span<const IceServer> ice_servers;

    std::uint16_t local_port_min = 0;
    std::uint16_t local_port_max = 0;

    std::uint32_t start_bitrate_bps = 8u * 1000u * 1000u;
    std::uint32_t min_bitrate_bps = 500u * 1000u;
    std::uint32_t max_bitrate_bps = 40u * 1000u * 1000u;

    bool enable_forward_error_correction = true;
    bool enable_retransmission = true;
};

struct TransportStats {
    std::uint64_t video_bytes_sent = 0;
    std::uint64_t audio_bytes_sent = 0;
    std::uint64_t packets_sent = 0;
    std::uint64_t packets_lost = 0;
    std::uint64_t retransmitted_packets = 0;
    std::uint64_t keyframes_requested = 0;
    Nanoseconds round_trip_time_ns = 0;
    std::uint32_t target_bitrate_bps = 0;
    std::uint32_t pacer_queue_ms = 0;
    ConnectionState state = ConnectionState::New;

    [[nodiscard]] double loss_ratio() const noexcept
    {
        const std::uint64_t total = packets_sent + packets_lost;
        return total == 0 ? 0.0 : static_cast<double>(packets_lost) / static_cast<double>(total);
    }
};

class TransportObserver {
public:
    virtual ~TransportObserver() = default;

    virtual void on_connection_state_changed(ConnectionState state) noexcept = 0;

    virtual void on_target_bitrate_changed(std::uint32_t bits_per_second,
                                           std::uint32_t framerate_hz) noexcept = 0;

    virtual void on_keyframe_requested() noexcept = 0;

    virtual void on_packet_loss_detected(double loss_ratio) noexcept { (void)loss_ratio; }

    virtual void on_long_term_reference_acknowledged(std::uint32_t index) noexcept { (void)index; }

    virtual void on_local_description(Span<const char> description) noexcept = 0;

    virtual void on_local_candidate(Span<const char> candidate) noexcept = 0;

    virtual void on_round_trip_time(Nanoseconds round_trip_ns) noexcept { (void)round_trip_ns; }

protected:
    TransportObserver() = default;
};

class MediaTransport {
public:
    virtual ~MediaTransport() = default;

    MediaTransport(const MediaTransport&) = delete;
    MediaTransport& operator=(const MediaTransport&) = delete;

    virtual Outcome start(TransportObserver& observer) = 0;
    virtual void stop() noexcept = 0;

    [[nodiscard]] virtual Outcome create_local_description() = 0;
    [[nodiscard]] virtual Outcome set_remote_description(Span<const char> description) = 0;
    [[nodiscard]] virtual Outcome add_remote_candidate(Span<const char> candidate) = 0;

    [[nodiscard]] virtual Outcome send_video(const EncodedVideoFrame& frame) = 0;
    [[nodiscard]] virtual Outcome send_audio(const PcmAudioBlock& block) = 0;

    [[nodiscard]] virtual TransportStats stats() const noexcept = 0;

protected:
    MediaTransport() = default;
};

[[nodiscard]] Result<std::unique_ptr<MediaTransport>> create_media_transport(
    const TransportConfig& config);

}  // namespace tl::transport
