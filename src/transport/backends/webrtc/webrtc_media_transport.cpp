#include <algorithm>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <memory>
#include <string>
#include <utility>
#include <vector>

#include "api/audio/audio_device.h"
#include "api/audio_codecs/builtin_audio_decoder_factory.h"
#include "api/audio_codecs/builtin_audio_encoder_factory.h"
#include "api/audio_options.h"
#include "api/create_peerconnection_factory.h"
#include "api/jsep.h"
#include "api/make_ref_counted.h"
#include "api/media_stream_interface.h"
#include "api/media_types.h"
#include "api/peer_connection_interface.h"
#include "api/rtc_error.h"
#include "api/rtp_parameters.h"
#include "api/rtp_sender_interface.h"
#include "api/rtp_transceiver_interface.h"
#include "api/scoped_refptr.h"
#include "api/stats/rtc_stats_collector_callback.h"
#include "api/stats/rtc_stats_report.h"
#include "api/stats/rtcstats_objects.h"
#include "api/transport/bitrate_settings.h"
#include "api/units/time_delta.h"
#include "api/video/video_frame.h"
#include "api/video/video_rotation.h"
#include "backends/webrtc/encoded_video_buffer.hpp"
#include "backends/webrtc/passthrough_video_decoder.hpp"
#include "backends/webrtc/passthrough_video_encoder.hpp"
#include "backends/webrtc/push_audio_device.hpp"
#include "backends/webrtc/push_video_source.hpp"
#include "rtc_base/thread.h"
#include "rtc_base/time_utils.h"
#include "telinha/core/clock.hpp"
#include "telinha/core/log.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::transport {
namespace {

using backend::EncodedVideoBuffer;
using backend::PassthroughVideoDecoderFactory;
using backend::PassthroughVideoEncoderFactory;
using backend::PushAudioDevice;
using backend::PushVideoSource;

constexpr std::size_t kMaxPendingCandidates = 64;
constexpr std::size_t kMaxCandidateBytes = 384;
constexpr std::size_t kMaxMidBytes = 32;
constexpr char kCandidateSeparator = '|';
constexpr char kVideoStreamId[] = "telinha";
constexpr char kVideoTrackId[] = "telinha_video";
constexpr char kAudioTrackId[] = "telinha_audio";

const IceServer kPublicStunServers[] = {
    {"stun:stun.l.google.com:19302", nullptr, nullptr},
    {"stun:stun1.l.google.com:19302", nullptr, nullptr},
    {"stun:stun.cloudflare.com:3478", nullptr, nullptr},
};

ConnectionState to_connection_state(
    webrtc::PeerConnectionInterface::PeerConnectionState state) noexcept
{
    switch (state) {
        case webrtc::PeerConnectionInterface::PeerConnectionState::kNew:
            return ConnectionState::New;
        case webrtc::PeerConnectionInterface::PeerConnectionState::kConnecting:
            return ConnectionState::Connecting;
        case webrtc::PeerConnectionInterface::PeerConnectionState::kConnected:
            return ConnectionState::Connected;
        case webrtc::PeerConnectionInterface::PeerConnectionState::kDisconnected:
            return ConnectionState::Disconnected;
        case webrtc::PeerConnectionInterface::PeerConnectionState::kFailed:
            return ConnectionState::Failed;
        case webrtc::PeerConnectionInterface::PeerConnectionState::kClosed:
            return ConnectionState::Closed;
    }
    return ConnectionState::New;
}

}  // namespace

class WebrtcMediaTransport final : public MediaTransport,
                                   public webrtc::PeerConnectionObserver,
                                   public backend::EncoderFeedback,
                                   public backend::RemoteVideoSink,
                                   public backend::RemoteAudioSink {
public:
    explicit WebrtcMediaTransport(const TransportConfig& config);
    ~WebrtcMediaTransport() override;

    [[nodiscard]] Outcome initialize();

    Outcome start(TransportObserver& observer) override;
    void stop() noexcept override;

    [[nodiscard]] Outcome create_local_description() override;
    [[nodiscard]] Outcome set_remote_description(Span<const char> description) override;
    [[nodiscard]] Outcome add_remote_candidate(Span<const char> candidate) override;

    [[nodiscard]] Outcome send_video(const EncodedVideoFrame& frame) override;
    [[nodiscard]] Outcome send_audio(const PcmAudioBlock& block) override;

    [[nodiscard]] TransportStats stats() const noexcept override;

    void OnSignalingChange(webrtc::PeerConnectionInterface::SignalingState state) override;
    void OnDataChannel(webrtc::scoped_refptr<webrtc::DataChannelInterface> channel) override;
    void OnRenegotiationNeeded() override;
    void OnIceGatheringChange(webrtc::PeerConnectionInterface::IceGatheringState state) override;
    void OnConnectionChange(webrtc::PeerConnectionInterface::PeerConnectionState state) override;
    void OnIceCandidate(const webrtc::IceCandidate* candidate) override;

    void on_keyframe_requested() noexcept override;
    void on_target_bitrate(std::uint32_t bits_per_second,
                           std::uint32_t framerate_hz) noexcept override;

    void on_remote_video(const EncodedVideoFrame& frame) noexcept override;
    void on_remote_audio(const PcmAudioBlock& block) noexcept override;

    void on_local_sdp_created(webrtc::SessionDescriptionInterface* description);
    void on_local_sdp_failed(webrtc::RTCError error);
    void on_stats_report(const webrtc::scoped_refptr<const webrtc::RTCStatsReport>& report);

private:
    class CreateSdpObserver : public webrtc::CreateSessionDescriptionObserver {
    public:
        explicit CreateSdpObserver(WebrtcMediaTransport& owner) : owner_(&owner) {}

        void OnSuccess(webrtc::SessionDescriptionInterface* description) override
        {
            owner_->on_local_sdp_created(description);
        }
        void OnFailure(webrtc::RTCError error) override
        {
            owner_->on_local_sdp_failed(std::move(error));
        }

    private:
        WebrtcMediaTransport* owner_;
    };

    class SetLocalObserver : public webrtc::SetLocalDescriptionObserverInterface {
    public:
        explicit SetLocalObserver(WebrtcMediaTransport& owner) : owner_(&owner) {}

        void OnSetLocalDescriptionComplete(webrtc::RTCError error) override
        {
            owner_->on_local_description_applied(std::move(error));
        }

    private:
        WebrtcMediaTransport* owner_;
    };

    class SetRemoteObserver : public webrtc::SetRemoteDescriptionObserverInterface {
    public:
        explicit SetRemoteObserver(WebrtcMediaTransport& owner) : owner_(&owner) {}

        void OnSetRemoteDescriptionComplete(webrtc::RTCError error) override
        {
            owner_->on_remote_description_applied(std::move(error));
        }

    private:
        WebrtcMediaTransport* owner_;
    };

    class StatsObserver : public webrtc::RTCStatsCollectorCallback {
    public:
        explicit StatsObserver(WebrtcMediaTransport& owner) : owner_(&owner) {}

        void OnStatsDelivered(
            const webrtc::scoped_refptr<const webrtc::RTCStatsReport>& report) override
        {
            owner_->on_stats_report(report);
        }

    private:
        WebrtcMediaTransport* owner_;
    };

    void on_local_description_applied(webrtc::RTCError error);
    void on_remote_description_applied(webrtc::RTCError error);
    [[nodiscard]] Outcome apply_remote_candidate(const char* line, std::size_t length);
    void flush_pending_candidates();
    void publish_state(ConnectionState state) noexcept;
    void schedule_stats_poll();
    [[nodiscard]] Outcome build_peer_connection();
    [[nodiscard]] Outcome attach_media();
    [[nodiscard]] const char* ice_failure_reason() const noexcept;

    TransportConfig config_;
    WireVideoCodec codec_ = WireVideoCodec::H264;
    TransportObserver* observer_ = nullptr;

    std::unique_ptr<webrtc::Thread> network_thread_;
    std::unique_ptr<webrtc::Thread> worker_thread_;
    std::unique_ptr<webrtc::Thread> signaling_thread_;

    webrtc::scoped_refptr<PushAudioDevice> audio_device_;
    webrtc::scoped_refptr<webrtc::PeerConnectionFactoryInterface> factory_;
    webrtc::scoped_refptr<webrtc::PeerConnectionInterface> peer_connection_;
    webrtc::scoped_refptr<PushVideoSource> video_source_;
    webrtc::scoped_refptr<webrtc::VideoTrackInterface> video_track_;
    webrtc::scoped_refptr<webrtc::AudioSourceInterface> audio_source_;
    webrtc::scoped_refptr<webrtc::AudioTrackInterface> audio_track_;
    webrtc::scoped_refptr<webrtc::RtpSenderInterface> video_sender_;
    webrtc::scoped_refptr<StatsObserver> stats_observer_;

    std::atomic<bool> running_{false};
    std::atomic<bool> remote_description_set_{false};
    std::atomic<ConnectionState> state_{ConnectionState::New};

    std::atomic<std::uint64_t> video_frames_submitted_{0};
    std::atomic<std::uint64_t> video_frames_rejected_{0};
    std::atomic<std::uint64_t> keyframes_requested_{0};
    std::atomic<std::uint32_t> target_bitrate_bps_{0};
    std::atomic<std::uint32_t> host_candidates_{0};
    std::atomic<std::uint32_t> reflexive_candidates_{0};
    std::atomic<std::uint32_t> relay_candidates_{0};

    std::atomic<std::uint64_t> video_bytes_sent_{0};
    std::atomic<std::uint64_t> audio_bytes_sent_{0};
    std::atomic<std::uint64_t> packets_sent_{0};
    std::atomic<std::uint64_t> packets_lost_{0};
    std::atomic<std::uint64_t> retransmitted_packets_{0};
    std::atomic<std::uint64_t> round_trip_time_ns_{0};
    std::atomic<std::uint32_t> pacer_queue_ms_{0};

    std::string local_description_;

    char pending_candidates_[kMaxPendingCandidates][kMaxCandidateBytes + kMaxMidBytes + 16] = {};
    std::size_t pending_candidate_count_ = 0;
};

WebrtcMediaTransport::WebrtcMediaTransport(const TransportConfig& config) : config_(config) {}

WebrtcMediaTransport::~WebrtcMediaTransport()
{
    stop();
}

Outcome WebrtcMediaTransport::initialize()
{
    network_thread_ = webrtc::Thread::CreateWithSocketServer();
    network_thread_->SetName("telinha_net", nullptr);
    if (!network_thread_->Start()) {
        return fail(Status::Unavailable, "initialize: network thread did not start");
    }

    worker_thread_ = webrtc::Thread::Create();
    worker_thread_->SetName("telinha_worker", nullptr);
    if (!worker_thread_->Start()) {
        return fail(Status::Unavailable, "initialize: worker thread did not start");
    }

    signaling_thread_ = webrtc::Thread::Create();
    signaling_thread_->SetName("telinha_signaling", nullptr);
    if (!signaling_thread_->Start()) {
        return fail(Status::Unavailable, "initialize: signaling thread did not start");
    }

    audio_device_ =
        worker_thread_->BlockingCall([] { return webrtc::make_ref_counted<PushAudioDevice>(); });
    audio_device_->set_remote_audio_sink(this);

    webrtc::SdpVideoFormat format = backend::sdp_format_for(codec_, "42e01f");

    factory_ = webrtc::CreatePeerConnectionFactory(
        network_thread_.get(), worker_thread_.get(), signaling_thread_.get(), audio_device_,
        webrtc::CreateBuiltinAudioEncoderFactory(), webrtc::CreateBuiltinAudioDecoderFactory(),
        std::make_unique<PassthroughVideoEncoderFactory>(format, codec_, *this),
        std::make_unique<PassthroughVideoDecoderFactory>(format, codec_, this), nullptr, nullptr);

    if (factory_ == nullptr) {
        return fail(Status::Unavailable, "initialize: peer connection factory was not created");
    }

    stats_observer_ = webrtc::make_ref_counted<StatsObserver>(*this);
    return ok();
}

Outcome WebrtcMediaTransport::build_peer_connection()
{
    webrtc::PeerConnectionInterface::RTCConfiguration configuration;
    configuration.sdp_semantics = webrtc::SdpSemantics::kUnifiedPlan;
    configuration.bundle_policy =
        webrtc::PeerConnectionInterface::BundlePolicy::kBundlePolicyMaxBundle;
    configuration.rtcp_mux_policy =
        webrtc::PeerConnectionInterface::RtcpMuxPolicy::kRtcpMuxPolicyRequire;
    configuration.continual_gathering_policy =
        webrtc::PeerConnectionInterface::ContinualGatheringPolicy::GATHER_ONCE;
    configuration.type = webrtc::PeerConnectionInterface::IceTransportsType::kAll;

    if (config_.candidate_policy != CandidatePolicy::HostOnly) {
        Span<const IceServer> servers = config_.ice_servers;
        if (servers.empty()) {
            servers = Span<const IceServer>(kPublicStunServers,
                                            sizeof(kPublicStunServers) / sizeof(IceServer));
        }
        for (const IceServer& entry : servers) {
            if (entry.url == nullptr || entry.url[0] == '\0') {
                continue;
            }
            const bool is_relay = std::strncmp(entry.url, "turn", 4) == 0;
            if (is_relay && config_.candidate_policy != CandidatePolicy::All) {
                continue;
            }
            webrtc::PeerConnectionInterface::IceServer server;
            server.urls.emplace_back(entry.url);
            if (entry.username != nullptr) {
                server.username = entry.username;
            }
            if (entry.credential != nullptr) {
                server.password = entry.credential;
            }
            configuration.servers.push_back(std::move(server));
        }
    }

    if (config_.local_port_min != 0 && config_.local_port_max >= config_.local_port_min) {
        configuration.port_allocator_config.min_port = config_.local_port_min;
        configuration.port_allocator_config.max_port = config_.local_port_max;
    }

    webrtc::PeerConnectionDependencies dependencies(this);
    webrtc::RTCErrorOr<webrtc::scoped_refptr<webrtc::PeerConnectionInterface>> created =
        factory_->CreatePeerConnectionOrError(configuration, std::move(dependencies));
    if (!created.ok()) {
        return fail(Status::Unavailable, "start: peer connection was refused",
                    static_cast<std::int32_t>(created.error().type()));
    }

    peer_connection_ = created.MoveValue();

    webrtc::BitrateSettings bitrate;
    bitrate.min_bitrate_bps = static_cast<int>(config_.min_bitrate_bps);
    bitrate.start_bitrate_bps = static_cast<int>(config_.start_bitrate_bps);
    bitrate.max_bitrate_bps = static_cast<int>(config_.max_bitrate_bps);
    peer_connection_->SetBitrate(bitrate);

    return ok();
}

Outcome WebrtcMediaTransport::attach_media()
{
    const bool sending = config_.role == TransportRole::Sender;

    webrtc::RtpTransceiverInit video_init;
    video_init.direction = sending ? webrtc::RtpTransceiverDirection::kSendOnly
                                   : webrtc::RtpTransceiverDirection::kRecvOnly;
    video_init.stream_ids.emplace_back(kVideoStreamId);

    webrtc::RtpTransceiverInit audio_init;
    audio_init.direction = video_init.direction;
    audio_init.stream_ids.emplace_back(kVideoStreamId);

    if (sending) {
        video_source_ = signaling_thread_->BlockingCall([] {
            webrtc::scoped_refptr<PushVideoSource> source =
                webrtc::make_ref_counted<PushVideoSource>();
            source->SetState(webrtc::MediaSourceInterface::SourceState::kLive);
            return source;
        });
        video_track_ = factory_->CreateVideoTrack(video_source_, kVideoTrackId);

        webrtc::AudioOptions audio_options;
        audio_options.echo_cancellation = false;
        audio_options.auto_gain_control = false;
        audio_options.noise_suppression = false;
        audio_source_ = factory_->CreateAudioSource(audio_options);
        audio_track_ = factory_->CreateAudioTrack(kAudioTrackId, audio_source_.get());

        webrtc::RTCErrorOr<webrtc::scoped_refptr<webrtc::RtpTransceiverInterface>> video =
            peer_connection_->AddTransceiver(video_track_, video_init);
        if (!video.ok()) {
            return fail(Status::Unavailable, "start: the video transceiver was refused");
        }
        video_sender_ = video.value()->sender();

        webrtc::RTCErrorOr<webrtc::scoped_refptr<webrtc::RtpTransceiverInterface>> audio =
            peer_connection_->AddTransceiver(audio_track_, audio_init);
        if (!audio.ok()) {
            return fail(Status::Unavailable, "start: the audio transceiver was refused");
        }

        webrtc::RtpParameters parameters = video_sender_->GetParameters();
        parameters.degradation_preference = webrtc::DegradationPreference::MAINTAIN_RESOLUTION;
        if (!parameters.encodings.empty()) {
            parameters.encodings[0].max_bitrate_bps = static_cast<int>(config_.max_bitrate_bps);
            parameters.encodings[0].min_bitrate_bps = static_cast<int>(config_.min_bitrate_bps);
            parameters.encodings[0].scale_resolution_down_by = 1.0;
        }
        const webrtc::RTCError applied = video_sender_->SetParameters(parameters);
        if (!applied.ok()) {
            return fail(Status::Unavailable, "start: the send parameters were refused");
        }
    } else {
        webrtc::RTCErrorOr<webrtc::scoped_refptr<webrtc::RtpTransceiverInterface>> video =
            peer_connection_->AddTransceiver(webrtc::MediaType::VIDEO, video_init);
        if (!video.ok()) {
            return fail(Status::Unavailable, "start: the video transceiver was refused");
        }
        webrtc::RTCErrorOr<webrtc::scoped_refptr<webrtc::RtpTransceiverInterface>> audio =
            peer_connection_->AddTransceiver(webrtc::MediaType::AUDIO, audio_init);
        if (!audio.ok()) {
            return fail(Status::Unavailable, "start: the audio transceiver was refused");
        }
    }

    return ok();
}

Outcome WebrtcMediaTransport::start(TransportObserver& observer)
{
    if (running_.load(std::memory_order_acquire)) {
        return fail(Status::AlreadyExists, "start: the transport is already running");
    }

    observer_ = &observer;
    TL_TRY(build_peer_connection());
    TL_TRY(attach_media());

    running_.store(true, std::memory_order_release);
    publish_state(ConnectionState::Gathering);
    schedule_stats_poll();
    return ok();
}

void WebrtcMediaTransport::stop() noexcept
{
    running_.store(false, std::memory_order_release);

    if (peer_connection_ != nullptr) {
        peer_connection_->Close();
        peer_connection_ = nullptr;
    }

    video_sender_ = nullptr;
    video_track_ = nullptr;
    audio_track_ = nullptr;
    audio_source_ = nullptr;
    video_source_ = nullptr;
    stats_observer_ = nullptr;
    factory_ = nullptr;

    if (audio_device_ != nullptr) {
        audio_device_->set_remote_audio_sink(nullptr);
        audio_device_ = nullptr;
    }

    if (signaling_thread_ != nullptr) {
        signaling_thread_->Stop();
        signaling_thread_ = nullptr;
    }
    if (worker_thread_ != nullptr) {
        worker_thread_->Stop();
        worker_thread_ = nullptr;
    }
    if (network_thread_ != nullptr) {
        network_thread_->Stop();
        network_thread_ = nullptr;
    }

    observer_ = nullptr;
}

Outcome WebrtcMediaTransport::create_local_description()
{
    if (!running_.load(std::memory_order_acquire)) {
        return fail(Status::Unavailable, "create_local_description: the transport is not running");
    }

    return signaling_thread_->BlockingCall([this]() -> Outcome {
        const webrtc::PeerConnectionInterface::SignalingState state =
            peer_connection_->signaling_state();
        webrtc::scoped_refptr<CreateSdpObserver> sdp_observer =
            webrtc::make_ref_counted<CreateSdpObserver>(*this);
        const webrtc::PeerConnectionInterface::RTCOfferAnswerOptions options;

        if (state == webrtc::PeerConnectionInterface::SignalingState::kStable &&
            !remote_description_set_.load(std::memory_order_acquire)) {
            peer_connection_->CreateOffer(sdp_observer.get(), options);
            return ok();
        }
        if (state == webrtc::PeerConnectionInterface::SignalingState::kHaveRemoteOffer) {
            peer_connection_->CreateAnswer(sdp_observer.get(), options);
            return ok();
        }
        return fail(Status::NotSupported,
                    "create_local_description: nothing to describe in this signaling state");
    });
}

void WebrtcMediaTransport::on_local_sdp_created(webrtc::SessionDescriptionInterface* description)
{
    std::unique_ptr<webrtc::SessionDescriptionInterface> owned(description);
    local_description_.clear();
    if (!owned->ToString(&local_description_)) {
        TL_LOG_ERROR("transport: the local description could not be serialised");
        return;
    }
    peer_connection_->SetLocalDescription(std::move(owned),
                                          webrtc::make_ref_counted<SetLocalObserver>(*this));
}

void WebrtcMediaTransport::on_local_sdp_failed(webrtc::RTCError error)
{
    TL_LOG_ERROR("transport: the local description failed: %s", error.message());
}

void WebrtcMediaTransport::on_local_description_applied(webrtc::RTCError error)
{
    if (!error.ok()) {
        TL_LOG_ERROR("transport: the local description was refused: %s", error.message());
        return;
    }
    if (observer_ != nullptr) {
        observer_->on_local_description(
            Span<const char>(local_description_.data(), local_description_.size()));
    }
}

Outcome WebrtcMediaTransport::set_remote_description(Span<const char> description)
{
    if (!running_.load(std::memory_order_acquire)) {
        return fail(Status::Unavailable, "set_remote_description: the transport is not running");
    }
    if (description.empty()) {
        return fail(Status::InvalidArgument, "set_remote_description: empty description");
    }

    std::string sdp(description.data(), description.size());

    return signaling_thread_->BlockingCall([this, &sdp]() -> Outcome {
        const webrtc::PeerConnectionInterface::SignalingState state =
            peer_connection_->signaling_state();
        webrtc::SdpType type = webrtc::SdpType::kOffer;
        if (state == webrtc::PeerConnectionInterface::SignalingState::kStable) {
            type = webrtc::SdpType::kOffer;
        } else if (state == webrtc::PeerConnectionInterface::SignalingState::kHaveLocalOffer) {
            type = webrtc::SdpType::kAnswer;
        } else {
            return fail(Status::NotSupported, "set_remote_description: unexpected signaling state");
        }

        webrtc::SdpParseError parse_error;
        std::unique_ptr<webrtc::SessionDescriptionInterface> parsed =
            webrtc::CreateSessionDescription(type, sdp, &parse_error);
        if (parsed == nullptr) {
            TL_LOG_ERROR("transport: the remote description did not parse: %s",
                         parse_error.description.c_str());
            return fail(Status::InvalidArgument,
                        "set_remote_description: the remote SDP did not parse");
        }

        peer_connection_->SetRemoteDescription(std::move(parsed),
                                               webrtc::make_ref_counted<SetRemoteObserver>(*this));
        return ok();
    });
}

void WebrtcMediaTransport::on_remote_description_applied(webrtc::RTCError error)
{
    if (!error.ok()) {
        TL_LOG_ERROR("transport: the remote description was refused: %s", error.message());
        return;
    }
    remote_description_set_.store(true, std::memory_order_release);
    flush_pending_candidates();
}

Outcome WebrtcMediaTransport::add_remote_candidate(Span<const char> candidate)
{
    if (!running_.load(std::memory_order_acquire)) {
        return fail(Status::Unavailable, "add_remote_candidate: the transport is not running");
    }
    if (candidate.empty() || candidate.size() >= sizeof(pending_candidates_[0])) {
        return fail(Status::InvalidArgument, "add_remote_candidate: malformed candidate line");
    }

    return signaling_thread_->BlockingCall([this, candidate]() -> Outcome {
        if (!remote_description_set_.load(std::memory_order_acquire)) {
            if (pending_candidate_count_ >= kMaxPendingCandidates) {
                return fail(Status::Full, "add_remote_candidate: too many early candidates");
            }
            std::memcpy(pending_candidates_[pending_candidate_count_], candidate.data(),
                        candidate.size());
            pending_candidates_[pending_candidate_count_][candidate.size()] = '\0';
            ++pending_candidate_count_;
            return ok();
        }
        return apply_remote_candidate(candidate.data(), candidate.size());
    });
}

Outcome WebrtcMediaTransport::apply_remote_candidate(const char* line, std::size_t length)
{
    const char* first = static_cast<const char*>(std::memchr(line, kCandidateSeparator, length));
    if (first == nullptr) {
        return fail(Status::InvalidArgument, "add_remote_candidate: the mid is missing");
    }
    const std::size_t after_first = static_cast<std::size_t>(first - line) + 1;
    const char* second = static_cast<const char*>(
        std::memchr(line + after_first, kCandidateSeparator, length - after_first));
    if (second == nullptr) {
        return fail(Status::InvalidArgument, "add_remote_candidate: the m line index is missing");
    }

    const std::string mid(line, static_cast<std::size_t>(first - line));
    const std::string index(first + 1, static_cast<std::size_t>(second - first) - 1);
    const std::size_t attribute_offset = static_cast<std::size_t>(second - line) + 1;
    const std::string attribute(line + attribute_offset, length - attribute_offset);

    int mline_index = 0;
    for (char digit : index) {
        if (digit < '0' || digit > '9') {
            return fail(Status::InvalidArgument,
                        "add_remote_candidate: the m line index is not a number");
        }
        mline_index = mline_index * 10 + (digit - '0');
    }

    webrtc::SdpParseError parse_error;
    std::unique_ptr<webrtc::IceCandidate> parsed(
        webrtc::CreateIceCandidate(mid, mline_index, attribute, &parse_error));
    if (parsed == nullptr) {
        TL_LOG_ERROR("transport: the remote candidate did not parse: %s",
                     parse_error.description.c_str());
        return fail(Status::InvalidArgument, "add_remote_candidate: the candidate did not parse");
    }

    if (!peer_connection_->AddIceCandidate(parsed.get())) {
        return fail(Status::Unavailable, "add_remote_candidate: the candidate was refused");
    }
    return ok();
}

void WebrtcMediaTransport::flush_pending_candidates()
{
    for (std::size_t index = 0; index < pending_candidate_count_; ++index) {
        const char* line = pending_candidates_[index];
        const Outcome applied = apply_remote_candidate(line, std::strlen(line));
        if (!applied.ok()) {
            TL_LOG_WARN("transport: a buffered candidate was dropped: %s", applied.error().context);
        }
    }
    pending_candidate_count_ = 0;
}

Outcome WebrtcMediaTransport::send_video(const EncodedVideoFrame& frame)
{
    if (!running_.load(std::memory_order_acquire)) {
        return fail(Status::Unavailable, "send_video: the transport is not running");
    }
    if (video_source_ == nullptr) {
        return fail(Status::NotSupported, "send_video: this transport was built to receive");
    }
    if (!frame.valid()) {
        return fail(Status::InvalidArgument, "send_video: empty bitstream");
    }
    if (frame.codec != codec_) {
        video_frames_rejected_.fetch_add(1, std::memory_order_relaxed);
        return fail(Status::NotSupported, "send_video: the codec was not the negotiated one");
    }
    if (frame.width == 0 || frame.height == 0) {
        return fail(Status::InvalidArgument, "send_video: the frame has no dimensions");
    }

    webrtc::scoped_refptr<EncodedVideoBuffer> buffer =
        webrtc::make_ref_counted<EncodedVideoBuffer>(frame);

    const Nanoseconds local_now = now_ns();
    const std::int64_t age_us =
        local_now > frame.capture_time_ns
            ? static_cast<std::int64_t>((local_now - frame.capture_time_ns) /
                                        kNanosecondsPerMicrosecond)
            : 0;

    const webrtc::VideoFrame video = webrtc::VideoFrame::Builder()
                                         .set_video_frame_buffer(buffer)
                                         .set_timestamp_us(webrtc::TimeMicros() - age_us)
                                         .set_rotation(webrtc::kVideoRotation_0)
                                         .build();

    video_source_->push(video);
    video_frames_submitted_.fetch_add(1, std::memory_order_relaxed);
    return ok();
}

Outcome WebrtcMediaTransport::send_audio(const PcmAudioBlock& block)
{
    if (!running_.load(std::memory_order_acquire)) {
        return fail(Status::Unavailable, "send_audio: the transport is not running");
    }
    if (audio_device_ == nullptr) {
        return fail(Status::Unavailable, "send_audio: there is no audio device");
    }
    return audio_device_->submit(block);
}

TransportStats WebrtcMediaTransport::stats() const noexcept
{
    TransportStats snapshot;
    snapshot.video_bytes_sent = video_bytes_sent_.load(std::memory_order_relaxed);
    snapshot.audio_bytes_sent = audio_bytes_sent_.load(std::memory_order_relaxed);
    snapshot.packets_sent = packets_sent_.load(std::memory_order_relaxed);
    snapshot.packets_lost = packets_lost_.load(std::memory_order_relaxed);
    snapshot.retransmitted_packets = retransmitted_packets_.load(std::memory_order_relaxed);
    snapshot.keyframes_requested = keyframes_requested_.load(std::memory_order_relaxed);
    snapshot.round_trip_time_ns = round_trip_time_ns_.load(std::memory_order_relaxed);
    snapshot.target_bitrate_bps = target_bitrate_bps_.load(std::memory_order_relaxed);
    snapshot.pacer_queue_ms = pacer_queue_ms_.load(std::memory_order_relaxed);
    snapshot.state = state_.load(std::memory_order_relaxed);
    return snapshot;
}

void WebrtcMediaTransport::publish_state(ConnectionState state) noexcept
{
    const ConnectionState previous = state_.exchange(state, std::memory_order_acq_rel);
    if (previous == state) {
        return;
    }
    if (observer_ != nullptr) {
        observer_->on_connection_state_changed(state);
    }
}

const char* WebrtcMediaTransport::ice_failure_reason() const noexcept
{
    if (config_.candidate_policy == CandidatePolicy::HostOnly) {
        return "only host candidates were offered, which never connects across the internet";
    }
    if (reflexive_candidates_.load(std::memory_order_relaxed) == 0) {
        return "no server reflexive candidate was gathered, the STUN servers were unreachable";
    }
    if (relay_candidates_.load(std::memory_order_relaxed) == 0) {
        return "one of the two networks needs a TURN relay, a direct connection cannot be made";
    }
    return "the relay candidates were rejected by the peer";
}

void WebrtcMediaTransport::OnSignalingChange(webrtc::PeerConnectionInterface::SignalingState state)
{
    (void)state;
}

void WebrtcMediaTransport::OnDataChannel(
    webrtc::scoped_refptr<webrtc::DataChannelInterface> channel)
{
    (void)channel;
}

void WebrtcMediaTransport::OnRenegotiationNeeded() {}

void WebrtcMediaTransport::OnIceGatheringChange(
    webrtc::PeerConnectionInterface::IceGatheringState state)
{
    if (state == webrtc::PeerConnectionInterface::IceGatheringState::kIceGatheringGathering) {
        publish_state(ConnectionState::Gathering);
    }
}

void WebrtcMediaTransport::OnConnectionChange(
    webrtc::PeerConnectionInterface::PeerConnectionState state)
{
    const ConnectionState mapped = to_connection_state(state);
    if (mapped == ConnectionState::Failed) {
        TL_LOG_ERROR("transport: the connection failed because %s", ice_failure_reason());
    }
    publish_state(mapped);
}

void WebrtcMediaTransport::OnIceCandidate(const webrtc::IceCandidate* candidate)
{
    if (candidate == nullptr || observer_ == nullptr) {
        return;
    }

    const std::string type_name(candidate->candidate().type_name());
    if (type_name == "host") {
        host_candidates_.fetch_add(1, std::memory_order_relaxed);
    } else if (type_name == "srflx" || type_name == "prflx") {
        reflexive_candidates_.fetch_add(1, std::memory_order_relaxed);
    } else if (type_name == "relay") {
        relay_candidates_.fetch_add(1, std::memory_order_relaxed);
    }

    std::string attribute;
    if (!candidate->ToString(&attribute)) {
        return;
    }

    std::string line;
    line.reserve(attribute.size() + kMaxMidBytes + 8);
    line.append(candidate->sdp_mid());
    line.push_back(kCandidateSeparator);
    line.append(std::to_string(candidate->sdp_mline_index()));
    line.push_back(kCandidateSeparator);
    line.append(attribute);

    observer_->on_local_candidate(Span<const char>(line.data(), line.size()));
}

void WebrtcMediaTransport::on_keyframe_requested() noexcept
{
    keyframes_requested_.fetch_add(1, std::memory_order_relaxed);
    if (observer_ != nullptr) {
        observer_->on_keyframe_requested();
    }
}

void WebrtcMediaTransport::on_target_bitrate(std::uint32_t bits_per_second,
                                             std::uint32_t framerate_hz) noexcept
{
    target_bitrate_bps_.store(bits_per_second, std::memory_order_relaxed);
    if (observer_ != nullptr) {
        observer_->on_target_bitrate_changed(bits_per_second, framerate_hz);
    }
}

void WebrtcMediaTransport::on_remote_video(const EncodedVideoFrame& frame) noexcept
{
    if (observer_ != nullptr) {
        observer_->on_remote_video(frame);
    }
}

void WebrtcMediaTransport::on_remote_audio(const PcmAudioBlock& block) noexcept
{
    if (observer_ != nullptr) {
        observer_->on_remote_audio(block);
    }
}

void WebrtcMediaTransport::schedule_stats_poll()
{
    if (!running_.load(std::memory_order_acquire) || signaling_thread_ == nullptr) {
        return;
    }
    signaling_thread_->PostDelayedTask(
        [this] {
            if (!running_.load(std::memory_order_acquire) || peer_connection_ == nullptr) {
                return;
            }
            peer_connection_->GetStats(stats_observer_.get());
            schedule_stats_poll();
        },
        webrtc::TimeDelta::Millis(1000));
}

void WebrtcMediaTransport::on_stats_report(
    const webrtc::scoped_refptr<const webrtc::RTCStatsReport>& report)
{
    if (report == nullptr) {
        return;
    }

    std::uint64_t video_bytes = 0;
    std::uint64_t audio_bytes = 0;
    std::uint64_t packets = 0;
    std::uint64_t retransmitted = 0;

    for (const webrtc::RTCOutboundRtpStreamStats* outbound :
         report->GetStatsOfType<webrtc::RTCOutboundRtpStreamStats>()) {
        const bool is_video = outbound->kind.has_value() && *outbound->kind == "video";
        const std::uint64_t bytes = outbound->bytes_sent.value_or(0);
        if (is_video) {
            video_bytes += bytes;
        } else {
            audio_bytes += bytes;
        }
        packets += outbound->packets_sent.value_or(0);
        retransmitted += outbound->retransmitted_packets_sent.value_or(0);
    }

    video_bytes_sent_.store(video_bytes, std::memory_order_relaxed);
    audio_bytes_sent_.store(audio_bytes, std::memory_order_relaxed);
    packets_sent_.store(packets, std::memory_order_relaxed);
    retransmitted_packets_.store(retransmitted, std::memory_order_relaxed);

    std::uint64_t lost = 0;
    double round_trip_seconds = 0.0;
    for (const webrtc::RTCRemoteInboundRtpStreamStats* remote :
         report->GetStatsOfType<webrtc::RTCRemoteInboundRtpStreamStats>()) {
        const std::int32_t reported = remote->packets_lost.value_or(0);
        if (reported > 0) {
            lost += static_cast<std::uint64_t>(reported);
        }
        round_trip_seconds = std::max(round_trip_seconds, remote->round_trip_time.value_or(0.0));
    }
    packets_lost_.store(lost, std::memory_order_relaxed);

    for (const webrtc::RTCIceCandidatePairStats* pair :
         report->GetStatsOfType<webrtc::RTCIceCandidatePairStats>()) {
        if (pair->state.has_value() && *pair->state == "succeeded") {
            round_trip_seconds =
                std::max(round_trip_seconds, pair->current_round_trip_time.value_or(0.0));
        }
    }

    const std::uint64_t round_trip_ns =
        static_cast<std::uint64_t>(round_trip_seconds * static_cast<double>(kNanosecondsPerSecond));
    round_trip_time_ns_.store(round_trip_ns, std::memory_order_relaxed);

    if (observer_ != nullptr && round_trip_ns > 0) {
        observer_->on_round_trip_time(round_trip_ns);
    }

    const std::uint64_t total = packets + lost;
    if (observer_ != nullptr && total > 0) {
        observer_->on_packet_loss_detected(static_cast<double>(lost) / static_cast<double>(total));
    }
}

Result<std::unique_ptr<MediaTransport>> create_media_transport(const TransportConfig& config)
{
    auto transport = std::make_unique<WebrtcMediaTransport>(config);
    const Outcome prepared = transport->initialize();
    if (!prepared.ok()) {
        return prepared.error();
    }
    return std::unique_ptr<MediaTransport>(transport.release());
}

}  // namespace tl::transport
