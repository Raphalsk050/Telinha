#include "telinha/app/sender_session.hpp"

#include <chrono>
#include <cstdio>
#include <cstring>
#include <new>
#include <utility>

#include "telinha/app/machine_events.hpp"
#include "telinha/core/log.hpp"

namespace tl::app {
namespace {

constexpr std::uint32_t kTargetListCapacity = 128;
constexpr std::uint32_t kAudioScratchFrames = 4096;
constexpr std::uint32_t kAudioAcquireTimeoutMs = 40;
constexpr std::uint32_t kMaxBitrateKbps = 4'000'000;
constexpr std::uint32_t kEncoderFullWaitMs = 50;
constexpr std::uint32_t kMaxPacketsPerDrain = 32;

void sleep_ns(Nanoseconds duration) noexcept
{
    std::this_thread::sleep_for(std::chrono::nanoseconds(duration));
}

void say_step(const char* text) noexcept
{
    if (machine_events_enabled()) {
        MachineEvent("step").text("text", text);
        return;
    }
    std::printf("  %s\n", text);
    std::fflush(stdout);
}

transport::IceServer to_ice_server(const IceServerConfig& config) noexcept
{
    transport::IceServer server;
    server.url = config.url;
    server.username = config.username[0] == '\0' ? nullptr : config.username;
    server.credential = config.credential[0] == '\0' ? nullptr : config.credential;
    return server;
}

std::int16_t to_int16(float sample) noexcept
{
    const float scaled = sample * 32767.0f;
    if (scaled >= 32767.0f) {
        return 32767;
    }
    if (scaled <= -32768.0f) {
        return -32768;
    }
    return static_cast<std::int16_t>(scaled);
}

void fit_within(std::uint32_t max_width, std::uint32_t max_height, std::uint32_t& width,
                std::uint32_t& height) noexcept
{
    if (width == 0 || height == 0) {
        return;
    }

    const std::uint32_t box_long = max_width > max_height ? max_width : max_height;
    const std::uint32_t box_short = max_width > max_height ? max_height : max_width;
    const std::uint32_t source_long = width > height ? width : height;
    const std::uint32_t source_short = width > height ? height : width;

    double scale = 1.0;
    if (box_long != 0 && source_long > box_long) {
        scale = static_cast<double>(box_long) / source_long;
    }
    if (box_short != 0 && source_short > box_short) {
        const double short_scale = static_cast<double>(box_short) / source_short;
        scale = short_scale < scale ? short_scale : scale;
    }
    if (scale >= 1.0) {
        return;
    }

    const auto scaled_width = static_cast<std::uint32_t>(width * scale) & ~1u;
    const auto scaled_height = static_cast<std::uint32_t>(height * scale) & ~1u;
    width = scaled_width < 2 ? 2 : scaled_width;
    height = scaled_height < 2 ? 2 : scaled_height;
}

}  // namespace

void SenderSession::PeerObserver::on_connection_state_changed(
    transport::ConnectionState state) noexcept
{
    if (!session_->multi_) {
        peer_->state.store(static_cast<std::uint32_t>(state), std::memory_order_relaxed);
        TL_LOG_INFO("emissor: conexao %s", to_string(state));
        if (machine_events_enabled()) {
            MachineEvent("state").text("state", to_string(state));
        }
        return;
    }

    if (!peer_->detached.load(std::memory_order_relaxed)) {
        TL_LOG_INFO("emissor: espectador %s conexao %s", peer_->id, to_string(state));
        MachineEvent("peer_state").text("peer", peer_->id).text("state", to_string(state));
        if (state == transport::ConnectionState::Connected) {
            session_->keyframe_pending_.store(true, std::memory_order_relaxed);
        }
    }
    peer_->state.store(static_cast<std::uint32_t>(state), std::memory_order_relaxed);
}

void SenderSession::PeerObserver::on_target_bitrate_changed(std::uint32_t bits_per_second,
                                                            std::uint32_t framerate_hz) noexcept
{
    (void)framerate_hz;
    peer_->target_bitrate_bps.store(bits_per_second, std::memory_order_relaxed);
    if (!session_->multi_) {
        session_->pending_bitrate_.store(bits_per_second, std::memory_order_relaxed);
    }
}

void SenderSession::PeerObserver::on_keyframe_requested() noexcept
{
    session_->keyframe_pending_.store(true, std::memory_order_relaxed);
}

void SenderSession::PeerObserver::on_packet_loss_detected(double loss_ratio) noexcept
{
    if (loss_ratio > 0.1) {
        session_->keyframe_pending_.store(true, std::memory_order_relaxed);
    }
}

void SenderSession::PeerObserver::on_local_description(Span<const char> description) noexcept
{
    peer_->signaling.on_description(description);
}

void SenderSession::PeerObserver::on_local_candidate(Span<const char> candidate) noexcept
{
    peer_->signaling.on_candidate(candidate);
}

void SenderSession::PeerObserver::on_gathering_complete() noexcept
{
    peer_->signaling.on_gathering_complete();
}

void SenderSession::PeerObserver::on_round_trip_time(Nanoseconds round_trip_ns) noexcept
{
    (void)round_trip_ns;
}

SenderSession::SenderSession() noexcept = default;

SenderSession::~SenderSession()
{
    stop_.store(true, std::memory_order_relaxed);
    if (audio_thread_.joinable()) {
        audio_thread_.join();
    }
}

void SenderSession::request_stop() noexcept
{
    stop_.store(true, std::memory_order_relaxed);
}

Outcome SenderSession::resolve_target(capture::CaptureTargetInfo& out)
{
    if (options_.target.valid()) {
        out = capture::CaptureTargetInfo{};
        out.target = options_.target;
        return ok();
    }

    static capture::CaptureTargetInfo targets[kTargetListCapacity];
    std::uint32_t written = 0;
    std::uint32_t available = 0;

    TL_TRY(capture::enumerate_targets(
        options_.target_kind, Span<capture::CaptureTargetInfo>(targets, kTargetListCapacity),
        written, available));

    if (options_.target_index < 0 || static_cast<std::uint32_t>(options_.target_index) >= written) {
        TL_LOG_ERROR("emissor: indice %d fora da lista de %u alvos, rode telinha list",
                     options_.target_index, written);
        return fail(Status::OutOfRange, "resolve_target");
    }

    out = targets[static_cast<std::uint32_t>(options_.target_index)];
    return ok();
}

Outcome SenderSession::open_capture()
{
    say_step("abrindo a captura");

    capture::CaptureTargetInfo chosen;
    TL_TRY(resolve_target(chosen));

    capture::CaptureOptions capture_options;
    capture_options.include_cursor = options_.include_cursor;
    capture_options.tile_size = options_.tile_size;

    Result<std::unique_ptr<capture::CaptureSource>> source =
        capture::create_capture_source(chosen.target, capture_options);
    if (!source.ok()) {
        return Outcome{source.error()};
    }

    pipeline_.reset(new (std::nothrow) capture::CapturePipeline());
    if (!pipeline_) {
        return fail(Status::OutOfMemory, "open_capture");
    }
    TL_TRY(pipeline_->initialize(std::move(source).value(), capture_options));
    TL_TRY(pipeline_->start());

    const capture::CaptureSourceInfo info = pipeline_->info();
    TL_LOG_INFO("emissor: compartilhando %s %ux%u via %s", chosen.name, info.width, info.height,
                to_string(info.backend));
    return ok();
}

Outcome SenderSession::open_encoder(const capture::CaptureSourceInfo& info)
{
    say_step("preparando o encoder");

    std::uint32_t width = info.width;
    std::uint32_t height = info.height;
    fit_within(max_width_, max_height_, width, height);

    encode::VideoEncoderConfig config;
    config.codec = encode::VideoCodec::H264;
    config.width = width;
    config.height = height;
    config.framerate_millihertz = frame_rate_millihertz();
    config.target_bitrate_bps = options_.network.start_bitrate_bps;
    config.max_bitrate_bps = options_.network.max_bitrate_bps;
    if (max_bitrate_bps_ != 0) {
        config.max_bitrate_bps = max_bitrate_bps_;
        if (config.target_bitrate_bps > max_bitrate_bps_) {
            config.target_bitrate_bps = max_bitrate_bps_;
        }
    }
    config.coding_unit_size = encode::coding_unit_size_for(config.codec);
    config.intra_refresh = options_.intra_refresh;
    config.region_of_interest = width == info.width && height == info.height;

    if (!config.valid()) {
        return fail(Status::InvalidArgument, "open_encoder: configuracao invalida");
    }

    Result<std::unique_ptr<encode::VideoEncoder>> created =
        encode::create_video_encoder(config, info.native_device);
    if (!created.ok()) {
        return Outcome{created.error()};
    }

    encoder_ = std::move(created).value();
    TL_TRY(encoder_->start());
    applied_bitrate_bps_ = config.target_bitrate_bps;

    const encode::VideoEncoderInfo encoder_info = encoder_->info();
    TL_LOG_INFO("emissor: encoder %s %ux%u, regioes sujas %s, intra refresh %s",
                to_string(encoder_info.backend), encoder_info.width, encoder_info.height,
                encoder_info.supports_dirty_regions ? "sim" : "nao",
                encoder_info.supports_intra_refresh ? "sim" : "nao");
    return ok();
}

Outcome SenderSession::open_video()
{
    TL_TRY(open_capture());
    return open_encoder(pipeline_->info());
}

void SenderSession::close_video() noexcept
{
    if (encoder_) {
        encoder_->stop();
        encoder_.reset();
    }
    if (pipeline_) {
        pipeline_->stop();
        pipeline_.reset();
    }
}

Outcome SenderSession::open_audio()
{
    if (options_.audio_scope == AudioScope::None) {
        return ok();
    }

    say_step("preparando o audio");

    audio::AudioCaptureTarget target;
    if (options_.audio_scope == AudioScope::Device) {
        if (options_.audio_device_id[0] == '\0') {
            TL_LOG_ERROR("emissor: --audio device precisa de --audio-device com a entrada de som");
            return fail(Status::InvalidArgument, "open_audio");
        }
        target = audio::AudioCaptureTarget::capture_device(options_.audio_device_id);
    } else if (options_.audio_scope == AudioScope::Process) {
        const std::uint32_t pid = options_.audio_process_id;
        if (pid == 0) {
            TL_LOG_ERROR("emissor: --audio process precisa de --audio-pid com o processo alvo");
            return fail(Status::InvalidArgument, "open_audio");
        }
        target = audio::AudioCaptureTarget::process_loopback(
            pid, audio::ProcessLoopbackMode::IncludeProcessTree);
    } else if (options_.audio_exclude_process_id != 0 && audio::process_loopback_available()) {
        target =
            audio::AudioCaptureTarget::everything_except_process(options_.audio_exclude_process_id);
    } else {
        if (options_.audio_exclude_process_id != 0) {
            TL_LOG_WARN(
                "emissor: este Windows nao separa o audio por programa, o som do processo %u vai "
                "junto",
                options_.audio_exclude_process_id);
        }
        target = audio::AudioCaptureTarget::system_loopback();
    }

    audio::AudioCaptureOptions capture_options;
    capture_options.requested_format = audio::AudioFormat{48000, 2, audio::SampleFormat::Int16};

    Result<std::unique_ptr<audio::AudioSource>> created =
        audio::create_audio_source(target, capture_options);
    if (!created.ok() &&
        target.process_loopback_mode == audio::ProcessLoopbackMode::ExcludeProcessTree) {
        TL_LOG_WARN(
            "emissor: nao consegui deixar o processo %u fora do audio (%s), usando o som de todo "
            "o sistema",
            options_.audio_exclude_process_id, to_string(created.status()));
        created = audio::create_audio_source(audio::AudioCaptureTarget::system_loopback(),
                                             capture_options);
    }
    if (!created.ok()) {
        return Outcome{created.error()};
    }
    audio_source_ = std::move(created).value();

    const audio::AudioSourceInfo info = audio_source_->info();

    if (options_.audio_scope == AudioScope::Process &&
        info.target.scope != audio::AudioCaptureScope::ProcessLoopback) {
        audio_source_.reset();
        if (machine_events_enabled()) {
            MachineEvent("error")
                .text("stage", "audio")
                .text("status", to_string(Status::NotSupported))
                .text("message", "esta versao do Windows nao separa o audio por programa");
        } else {
            std::printf(
                "\nATENCAO: esta versao do Windows nao separa o audio por programa.\n"
                "Voce pediu o audio de um programa so, mas o sistema so consegue entregar o "
                "audio de tudo que estiver tocando, incluindo notificacoes, navegador e "
                "chamadas.\n"
                "A transmissao nao vai comecar assim. Escolha uma das duas:\n"
                "  --audio system   aceita transmitir o audio de todo o sistema\n"
                "  --audio none     transmite so a imagem, sem audio\n\n");
            std::fflush(stdout);
        }
        return fail(Status::NotSupported, "open_audio: loopback por processo indisponivel");
    }

    audio_scratch_frames_ = kAudioScratchFrames;
    audio_scratch_.reset(new (std::nothrow)
                             std::int16_t[audio_scratch_frames_ * info.format.channels]);
    if (!audio_scratch_) {
        return fail(Status::OutOfMemory, "open_audio");
    }

    TL_TRY(audio_source_->start());
    TL_LOG_INFO("emissor: audio %s %u Hz %u canais", to_string(info.target.scope),
                info.format.sample_rate, info.format.channels);
    if (info.target.scope == audio::AudioCaptureScope::ProcessLoopback &&
        info.target.process_loopback_mode == audio::ProcessLoopbackMode::ExcludeProcessTree) {
        TL_LOG_INFO("emissor: o som do processo %u e dos filhos fica de fora",
                    info.target.process_id);
    }
    return ok();
}

Outcome SenderSession::create_peer(std::unique_ptr<Peer>& out)
{
    out.reset(new (std::nothrow) Peer(*this));
    if (!out) {
        return fail(Status::OutOfMemory, "SenderSession::initialize");
    }

    TL_TRY(out->signaling.reserve(transport::TransportRole::Sender));

    out->token.reset(new (std::nothrow) char[kTokenCapacity]);
    out->remote_blob.reset(new (std::nothrow) transport::SessionBlob());
    if (!out->token || !out->remote_blob) {
        return fail(Status::OutOfMemory, "SenderSession::initialize");
    }
    return ok();
}

Outcome SenderSession::open_transport(Peer& peer)
{
    transport::TransportConfig config;
    config.role = transport::TransportRole::Sender;
    config.candidate_policy = options_.network.candidate_policy;
    config.ice_servers =
        Span<const transport::IceServer>(ice_servers_, options_.network.ice_server_count);
    config.local_port_min = options_.network.local_port_min;
    config.local_port_max = options_.network.local_port_max;
    config.start_bitrate_bps = options_.network.start_bitrate_bps;
    config.min_bitrate_bps = options_.network.min_bitrate_bps;
    config.max_bitrate_bps = options_.network.max_bitrate_bps;
    config.enable_forward_error_correction = options_.network.enable_forward_error_correction;
    config.enable_retransmission = options_.network.enable_retransmission;

    Result<std::unique_ptr<transport::MediaTransport>> created =
        transport::create_media_transport(config);
    if (!created.ok()) {
        return Outcome{created.error()};
    }
    peer.media = std::move(created).value();
    return ok();
}

Outcome SenderSession::initialize(const SenderOptions& options)
{
    options_ = options;
    multi_ = options.multi_peer;

    for (std::uint32_t index = 0; index < options_.network.ice_server_count; ++index) {
        ice_servers_[index] = to_ice_server(options_.network.ice_servers[index]);
    }

    if (!multi_) {
        TL_TRY(create_peer(peers_[0]));
    }

    TL_TRY(open_video());
    TL_TRY(open_audio());

    if (!multi_) {
        say_step("preparando a rede");
        TL_TRY(open_transport(*peers_[0]));
    }
    return ok();
}

Outcome SenderSession::negotiate()
{
    Peer& peer = *peers_[0];
    TL_TRY(peer.media->create_local_description());

    const Nanoseconds deadline = now_ns() + options_.network.gather_timeout_ns;
    while (!peer.signaling.gathering_complete() && now_ns() < deadline &&
           !stop_.load(std::memory_order_relaxed)) {
        sleep_ns(20ull * kNanosecondsPerMillisecond);
    }

    if (!peer.signaling.has_description()) {
        return fail(Status::Timeout, "negotiate: sem descricao local");
    }

    if (peer.signaling.candidate_count() == 0) {
        TL_LOG_WARN("emissor: nenhum candidato reunido, a conexao provavelmente vai falhar");
    }

    std::size_t length = 0;
    TL_TRY(peer.signaling.encode(peer.token.get(), kTokenCapacity, length));
    TL_TRY(publish_token(options_.signaling, "invite", "convite para quem vai assistir",
                         peer.token.get(), length));

    TL_TRY(receive_remote_blob(options_.signaling, "answer", "resposta de quem vai assistir",
                               transport::TransportRole::Receiver, peer.token.get(), kTokenCapacity,
                               *peer.remote_blob));
    if (machine_events_enabled()) {
        start_command_reader(stop_);
    }
    TL_TRY(apply_remote_blob(*peer.media, *peer.remote_blob));

    return await_connection();
}

Outcome SenderSession::await_connection()
{
    const Peer& peer = *peers_[0];
    const Nanoseconds deadline = now_ns() + options_.network.connect_timeout_ns;

    while (!stop_.load(std::memory_order_relaxed)) {
        const auto state =
            static_cast<transport::ConnectionState>(peer.state.load(std::memory_order_relaxed));
        if (state == transport::ConnectionState::Connected) {
            return ok();
        }
        if (state == transport::ConnectionState::Failed ||
            state == transport::ConnectionState::Closed) {
            return fail(Status::Unavailable, "await_connection: ICE falhou");
        }
        if (now_ns() >= deadline) {
            return fail(Status::Timeout, "await_connection");
        }
        sleep_ns(20ull * kNanosecondsPerMillisecond);
    }
    return fail(Status::Unavailable, "await_connection: interrompido");
}

Outcome SenderSession::switch_target(const capture::CaptureTarget& target)
{
    const capture::CaptureTarget previous = options_.target;

    close_video();
    options_.target = target;
    const Outcome opened = open_video();
    if (opened.ok()) {
        return ok();
    }

    TL_LOG_WARN("emissor: nao consegui trocar de alvo (%s), voltando ao anterior",
                to_string(opened.status()));
    close_video();
    options_.target = previous;
    const Outcome restored = open_video();
    if (!restored.ok()) {
        stop_.store(true, std::memory_order_relaxed);
        return restored;
    }
    return opened;
}

Outcome SenderSession::apply_quality(const MachineCommand& command)
{
    const bool reopen = command.max_fps != max_fps_ || command.max_width != max_width_ ||
                        command.max_height != max_height_;

    max_fps_ = command.max_fps;
    max_width_ = command.max_width;
    max_height_ = command.max_height;
    max_bitrate_bps_ = command.max_bitrate_kbps > kMaxBitrateKbps
                           ? kMaxBitrateKbps * 1000u
                           : command.max_bitrate_kbps * 1000u;
    next_frame_ns_ = 0;

    if (reopen) {
        if (encoder_) {
            encoder_->stop();
            encoder_.reset();
        }
        const Outcome opened = open_encoder(pipeline_->info());
        if (!opened.ok()) {
            stop_.store(true, std::memory_order_relaxed);
        }
        return opened;
    }

    std::uint32_t target =
        multi_ ? lowest_peer_bitrate() : peers_[0]->media->stats().target_bitrate_bps;
    if (target == 0) {
        target = options_.network.start_bitrate_bps;
    }
    if (max_bitrate_bps_ != 0 && target > max_bitrate_bps_) {
        target = max_bitrate_bps_;
    }
    applied_bitrate_bps_ = target;
    return encoder_->set_target_bitrate(target);
}

void SenderSession::handle_command(const MachineCommand& command)
{
    if (multi_) {
        if (command.kind == MachineCommandKind::AddPeer) {
            add_peer(command);
            return;
        }
        if (command.kind == MachineCommandKind::PeerAnswer ||
            command.kind == MachineCommandKind::PeerSdpAnswer) {
            answer_peer(command);
            return;
        }
        if (command.kind == MachineCommandKind::RemovePeer) {
            remove_peer(command);
            return;
        }
    }

    if (command.kind == MachineCommandKind::SwitchTarget) {
        capture::CaptureTarget target = capture::CaptureTarget::monitor(command.handle);
        if (command.window) {
            target = capture::CaptureTarget::window(command.handle);
        } else if (command.device) {
            target = capture::CaptureTarget::device(command.handle);
        }
        const Outcome switched = switch_target(target);
        if (!switched.ok()) {
            MachineEvent("command_failed")
                .text("command", "switch_target")
                .text("status", to_string(switched.status()))
                .text("message", switched.error().context);
            return;
        }

        const encode::VideoEncoderInfo encoder_info = encoder_->info();
        MachineEvent("target")
            .text("kind", command.window ? "window" : (command.device ? "device" : "monitor"))
            .integer("handle", command.handle)
            .integer("width", encoder_info.width)
            .integer("height", encoder_info.height);
        return;
    }

    if (command.kind == MachineCommandKind::SetQuality) {
        const Outcome applied = apply_quality(command);
        if (!applied.ok()) {
            MachineEvent("command_failed")
                .text("command", "set_quality")
                .text("status", to_string(applied.status()))
                .text("message", applied.error().context);
            return;
        }

        const encode::VideoEncoderInfo encoder_info = encoder_->info();
        MachineEvent("quality")
            .integer("max_fps", max_fps_)
            .integer("max_width", max_width_)
            .integer("max_height", max_height_)
            .integer("max_bitrate_kbps", max_bitrate_bps_ / 1000u)
            .integer("width", encoder_info.width)
            .integer("height", encoder_info.height);
        return;
    }

    if (command.kind == MachineCommandKind::SetAudio) {
        const Outcome switched = switch_audio(command);
        if (!switched.ok()) {
            MachineEvent("command_failed")
                .text("command", "set_audio")
                .text("status", to_string(switched.status()))
                .text("message", switched.error().context);
            return;
        }
        MachineEvent("audio")
            .text("scope", command.scope)
            .integer("pid", command.pid)
            .text("device", options_.audio_device_id);
    }
}

Outcome SenderSession::switch_audio(const MachineCommand& command)
{
    AudioScope scope = AudioScope::None;
    if (std::strcmp(command.scope, "system") == 0) {
        scope = AudioScope::System;
    } else if (std::strcmp(command.scope, "process") == 0) {
        scope = AudioScope::Process;
    } else if (std::strcmp(command.scope, "device") == 0) {
        if (command.device_id[0] == '\0') {
            return fail(Status::InvalidArgument, "switch_audio: falta a entrada de som");
        }
        scope = AudioScope::Device;
    } else if (std::strcmp(command.scope, "none") != 0) {
        return fail(Status::InvalidArgument, "switch_audio: escopo desconhecido");
    }

    audio_stop_.store(true, std::memory_order_relaxed);
    if (audio_thread_.joinable()) {
        audio_thread_.join();
    }
    if (audio_source_) {
        audio_source_->stop();
        audio_source_.reset();
    }
    audio_stop_.store(false, std::memory_order_relaxed);

    const AudioScope previous_scope = options_.audio_scope;
    const std::uint32_t previous_pid = options_.audio_process_id;
    char previous_device[audio::kAudioDeviceIdCapacity];
    std::memcpy(previous_device, options_.audio_device_id, sizeof(previous_device));
    options_.audio_scope = scope;
    options_.audio_process_id = command.pid;
    if (scope == AudioScope::Device) {
        std::snprintf(options_.audio_device_id, sizeof(options_.audio_device_id), "%s",
                      command.device_id);
    }

    const Outcome opened = open_audio();
    if (!opened.ok()) {
        TL_LOG_WARN("emissor: nao consegui trocar o audio (%s), voltando ao anterior",
                    to_string(opened.status()));
        audio_source_.reset();
        options_.audio_scope = previous_scope;
        options_.audio_process_id = previous_pid;
        std::memcpy(options_.audio_device_id, previous_device, sizeof(previous_device));
        if (!open_audio().ok()) {
            audio_source_.reset();
        }
    }
    if (audio_source_) {
        audio_thread_ = std::thread(&SenderSession::audio_thread_main, this);
    }
    return opened;
}

int SenderSession::find_peer(const char* id) const noexcept
{
    for (std::uint32_t slot = 0; slot < kMaxSenderPeers; ++slot) {
        if (peers_[slot] && std::strcmp(peers_[slot]->id, id) == 0) {
            return static_cast<int>(slot);
        }
    }
    return -1;
}

bool SenderSession::peer_connected(const Peer& peer) noexcept
{
    return static_cast<transport::ConnectionState>(peer.state.load(std::memory_order_relaxed)) ==
           transport::ConnectionState::Connected;
}

bool SenderSession::peer_sendable(const Peer& peer) const noexcept
{
    return !multi_ || peer_connected(peer);
}

std::uint32_t SenderSession::connected_peers() const noexcept
{
    std::uint32_t count = 0;
    for (const std::unique_ptr<Peer>& peer : peers_) {
        if (peer && peer_connected(*peer)) {
            ++count;
        }
    }
    return count;
}

std::uint32_t SenderSession::lowest_peer_bitrate() const noexcept
{
    std::uint32_t lowest = 0;
    for (const std::unique_ptr<Peer>& peer : peers_) {
        if (!peer || !peer_connected(*peer)) {
            continue;
        }
        const std::uint32_t target = peer->target_bitrate_bps.load(std::memory_order_relaxed);
        if (target != 0 && (lowest == 0 || target < lowest)) {
            lowest = target;
        }
    }
    return lowest;
}

std::unique_ptr<SenderSession::Peer> SenderSession::detach_peer(std::uint32_t slot) noexcept
{
    std::unique_ptr<Peer> peer;
    {
        const std::lock_guard<std::mutex> guard(send_mutex_);
        peer = std::move(peers_[slot]);
    }
    return peer;
}

void SenderSession::destroy_peer(std::unique_ptr<Peer> peer) noexcept
{
    if (!peer) {
        return;
    }
    peer->detached.store(true, std::memory_order_relaxed);
    if (peer->media) {
        peer->media->stop();
        peer->media.reset();
    }
}

void SenderSession::report_peer_failure(const char* id, const Outcome& reason)
{
    TL_LOG_WARN("emissor: espectador %s falhou (%s, %s)", id, to_string(reason.status()),
                reason.error().context);
    MachineEvent("peer_failed")
        .text("peer", id)
        .text("status", to_string(reason.status()))
        .text("message", reason.error().context);
    MachineEvent("peer_removed").text("peer", id);
}

void SenderSession::fail_peer(std::uint32_t slot, const Outcome& reason)
{
    char id[kPeerIdCapacity];
    std::memcpy(id, peers_[slot]->id, sizeof(id));
    destroy_peer(detach_peer(slot));
    report_peer_failure(id, reason);
}

Outcome SenderSession::start_peer(Peer& peer)
{
    TL_TRY(peer.media->start(peer.observer));
    TL_TRY(peer.media->create_local_description());
    peer.phase = PeerPhase::Gathering;
    peer.deadline_ns = now_ns() + options_.network.gather_timeout_ns;
    return ok();
}

void SenderSession::add_peer(const MachineCommand& command)
{
    std::uint32_t slot = kMaxSenderPeers;
    const int existing = find_peer(command.peer);
    if (existing >= 0) {
        slot = static_cast<std::uint32_t>(existing);
        TL_LOG_INFO("emissor: espectador %s trocado por um convite novo", command.peer);
        destroy_peer(detach_peer(slot));
    } else {
        for (std::uint32_t index = 0; index < kMaxSenderPeers; ++index) {
            if (!peers_[index]) {
                slot = index;
                break;
            }
        }
    }

    if (slot == kMaxSenderPeers) {
        report_peer_failure(command.peer,
                            fail(Status::OutOfRange, "add_peer: limite de espectadores atingido"));
        return;
    }

    std::unique_ptr<Peer> peer;
    Outcome created = create_peer(peer);
    if (created.ok()) {
        std::memcpy(peer->id, command.peer, sizeof(peer->id));
        peer->format = command.format;
        created = open_transport(*peer);
    }
    if (created.ok()) {
        created = start_peer(*peer);
    }
    if (!created.ok()) {
        destroy_peer(std::move(peer));
        report_peer_failure(command.peer, created);
        return;
    }

    TL_LOG_INFO("emissor: preparando convite para %s", command.peer);
    const std::lock_guard<std::mutex> guard(send_mutex_);
    peers_[slot] = std::move(peer);
}

void SenderSession::answer_peer(const MachineCommand& command)
{
    const int found = find_peer(command.peer);
    if (found < 0) {
        TL_LOG_WARN("emissor: resposta de um espectador desconhecido (%s)", command.peer);
        return;
    }
    Peer& peer = *peers_[static_cast<std::uint32_t>(found)];

    Outcome accepted = ok();
    if (peer.phase == PeerPhase::Gathering) {
        accepted = fail(Status::Unavailable, "peer_answer: o convite ainda nao saiu");
    } else if (peer.phase != PeerPhase::AwaitingAnswer) {
        accepted = fail(Status::AlreadyExists, "peer_answer: este espectador ja respondeu");
    } else if (command.kind == MachineCommandKind::PeerSdpAnswer) {
        accepted = peer.media->set_remote_description(
            Span<const char>(command.payload, command.payload_length));
    } else {
        accepted = transport::decode_session_blob(
            Span<const char>(command.payload, command.payload_length), *peer.remote_blob);
        if (accepted.ok() && peer.remote_blob->role() != transport::TransportRole::Receiver) {
            accepted = fail(Status::InvalidArgument,
                            "peer_answer: o codigo veio da mesma ponta, e o seu proprio");
        }
        if (accepted.ok()) {
            accepted = apply_remote_blob(*peer.media, *peer.remote_blob);
        }
    }

    if (!accepted.ok()) {
        TL_LOG_WARN("emissor: codigo de %s recusado (%s)", peer.id, accepted.error().context);
        MachineEvent("code_rejected")
            .text("peer", peer.id)
            .text("message", accepted.error().context);
        return;
    }

    peer.phase = PeerPhase::Connecting;
    peer.deadline_ns = now_ns() + options_.network.connect_timeout_ns;
}

void SenderSession::remove_peer(const MachineCommand& command)
{
    const int found = find_peer(command.peer);
    if (found < 0) {
        return;
    }
    destroy_peer(detach_peer(static_cast<std::uint32_t>(found)));
    TL_LOG_INFO("emissor: espectador %s removido", command.peer);
    MachineEvent("peer_removed").text("peer", command.peer);
}

void SenderSession::publish_invite(std::uint32_t slot)
{
    Peer& peer = *peers_[slot];
    if (!peer.signaling.has_description()) {
        fail_peer(slot, fail(Status::Timeout, "negotiate: sem descricao local"));
        return;
    }
    if (peer.signaling.candidate_count() == 0) {
        TL_LOG_WARN("emissor: nenhum candidato reunido para %s, a conexao provavelmente vai falhar",
                    peer.id);
    }

    std::size_t length = 0;
    const Outcome encoded = peer.format == PeerFormat::Sdp
                                ? peer.media->local_session_description(
                                      Span<char>(peer.token.get(), kTokenCapacity), length)
                                : peer.signaling.encode(peer.token.get(), kTokenCapacity, length);
    if (!encoded.ok()) {
        fail_peer(slot, encoded);
        return;
    }

    peer.phase = PeerPhase::AwaitingAnswer;
    peer.deadline_ns = 0;
    if (peer.format == PeerFormat::Sdp) {
        MachineEvent("offer").text("peer", peer.id).text("sdp", peer.token.get(), length);
        return;
    }
    MachineEvent("code")
        .text("kind", "invite")
        .text("peer", peer.id)
        .text("code", peer.token.get(), length);
}

void SenderSession::service_peers()
{
    const Nanoseconds now = now_ns();
    for (std::uint32_t slot = 0; slot < kMaxSenderPeers; ++slot) {
        if (!peers_[slot]) {
            continue;
        }
        Peer& peer = *peers_[slot];
        const auto state =
            static_cast<transport::ConnectionState>(peer.state.load(std::memory_order_relaxed));

        if (state == transport::ConnectionState::Failed ||
            state == transport::ConnectionState::Closed) {
            fail_peer(slot, peer.phase == PeerPhase::Connected
                                ? fail(Status::Unavailable, "a conexao caiu e nao volta sozinha")
                                : fail(Status::Unavailable, "await_connection: ICE falhou"));
            continue;
        }

        switch (peer.phase) {
            case PeerPhase::Gathering:
                if (peer.signaling.gathering_complete() || now >= peer.deadline_ns) {
                    publish_invite(slot);
                }
                break;
            case PeerPhase::Connecting:
                if (state == transport::ConnectionState::Connected) {
                    peer.phase = PeerPhase::Connected;
                } else if (now >= peer.deadline_ns) {
                    fail_peer(slot, fail(Status::Timeout, "await_connection: tempo esgotado"));
                }
                break;
            case PeerPhase::AwaitingAnswer:
            case PeerPhase::Connected: break;
        }
    }
}

void SenderSession::apply_peer_bitrate() noexcept
{
    std::uint32_t bitrate = lowest_peer_bitrate();
    if (bitrate == 0) {
        return;
    }
    if (max_bitrate_bps_ != 0 && bitrate > max_bitrate_bps_) {
        bitrate = max_bitrate_bps_;
    }
    if (bitrate == applied_bitrate_bps_) {
        return;
    }

    applied_bitrate_bps_ = bitrate;
    const Outcome applied = encoder_->set_target_bitrate(bitrate);
    if (applied.ok()) {
        counters_.bitrate_updates.fetch_add(1, std::memory_order_relaxed);
    } else {
        TL_LOG_WARN("emissor: encoder recusou %u bps (%s)", bitrate, to_string(applied.status()));
    }
}

void SenderSession::apply_pending_controls() noexcept
{
    if (multi_) {
        apply_peer_bitrate();
    } else {
        std::uint32_t bitrate = pending_bitrate_.exchange(0, std::memory_order_relaxed);
        if (bitrate != 0) {
            if (max_bitrate_bps_ != 0 && bitrate > max_bitrate_bps_) {
                bitrate = max_bitrate_bps_;
            }
            const Outcome applied = encoder_->set_target_bitrate(bitrate);
            if (applied.ok()) {
                counters_.bitrate_updates.fetch_add(1, std::memory_order_relaxed);
            } else {
                TL_LOG_WARN("emissor: encoder recusou %u bps (%s)", bitrate,
                            to_string(applied.status()));
            }
        }
    }

    if (keyframe_pending_.exchange(false, std::memory_order_relaxed)) {
        encoder_->request_keyframe();
        counters_.keyframes.fetch_add(1, std::memory_order_relaxed);
    }
}

void SenderSession::send_encoded(const transport::EncodedVideoFrame& frame)
{
    counters_.frames_encoded.fetch_add(1, std::memory_order_relaxed);

    Nanoseconds send_elapsed = 0;
    std::uint32_t delivered = 0;
    std::uint32_t refused = 0;
    {
        const ScopedTimer timer(send_elapsed);
        const std::lock_guard<std::mutex> guard(send_mutex_);
        for (const std::unique_ptr<Peer>& peer : peers_) {
            if (!peer || !peer_sendable(*peer)) {
                continue;
            }
            if (peer->media->send_video(frame).ok()) {
                ++delivered;
            } else {
                ++refused;
            }
        }
    }
    const std::size_t bytes = frame.bitstream.size_bytes();
    encoder_->release();

    if (refused != 0) {
        counters_.send_failures.fetch_add(refused, std::memory_order_relaxed);
    }
    if (delivered == 0) {
        return;
    }

    send_ns_.record(send_elapsed);
    counters_.frames_sent.fetch_add(1, std::memory_order_relaxed);
    counters_.video_bytes.fetch_add(bytes, std::memory_order_relaxed);
}

void SenderSession::drain_encoder(std::uint32_t first_timeout_ms)
{
    std::uint32_t timeout_ms = first_timeout_ms;
    for (std::uint32_t packet = 0; packet < kMaxPacketsPerDrain; ++packet) {
        transport::EncodedVideoFrame frame;
        const Outcome polled = encoder_->poll(frame, timeout_ms);
        if (!polled.ok()) {
            return;
        }
        timeout_ms = 0;
        send_encoded(frame);
    }
}

void SenderSession::note_encoder_refusal(const Outcome& refused) noexcept
{
    ++encoder_refusals_;
    const Nanoseconds now = now_ns();
    if (last_refusal_log_ns_ != 0 && now - last_refusal_log_ns_ < kNanosecondsPerSecond) {
        return;
    }
    if (refused.status() == Status::Full) {
        TL_LOG_WARN("emissor: encoder ocupado, %u quadros descartados", encoder_refusals_);
    } else {
        TL_LOG_WARN("emissor: encoder recusou %u quadros (%s, %s, 0x%08X)", encoder_refusals_,
                    to_string(refused.status()), refused.error().context,
                    static_cast<unsigned>(refused.error().platform_code));
    }
    encoder_refusals_ = 0;
    last_refusal_log_ns_ = now;
}

std::uint32_t SenderSession::frame_rate_millihertz() const noexcept
{
    constexpr std::uint32_t kAutomaticMillihertz = 60000;
    constexpr std::uint32_t kHighestExplicitFps = 1000;

    if (max_fps_ != 0) {
        return (max_fps_ > kHighestExplicitFps ? kHighestExplicitFps : max_fps_) * 1000u;
    }

    std::uint32_t limit =
        options_.framerate_millihertz != 0 && options_.framerate_millihertz < kAutomaticMillihertz
            ? options_.framerate_millihertz
            : kAutomaticMillihertz;
    const std::uint32_t refresh = pipeline_ ? pipeline_->info().refresh_millihertz : 0;
    if (refresh >= 1000u && refresh < limit) {
        limit = refresh;
    }
    return limit;
}

void SenderSession::pump_video()
{
    apply_pending_controls();
    drain_encoder(0);

    const Nanoseconds before_capture = now_ns();
    if (before_capture < next_frame_ns_) {
        const Nanoseconds remaining = next_frame_ns_ - before_capture;
        sleep_ns(remaining < kNanosecondsPerMillisecond ? remaining : kNanosecondsPerMillisecond);
        return;
    }

    capture::ClassifiedFrame classified;
    Nanoseconds capture_elapsed = 0;
    Outcome captured = ok();
    {
        const ScopedTimer timer(capture_elapsed);
        captured = pipeline_->capture_next(classified, options_.capture_timeout_ms);
    }

    if (!captured.ok()) {
        if (captured.status() != Status::Timeout) {
            TL_LOG_WARN("emissor: captura falhou (%s)", to_string(captured.status()));
        }
        return;
    }

    next_frame_ns_ = now_ns() + kNanosecondsPerSecond * 1000u / frame_rate_millihertz();
    capture_ns_.record(capture_elapsed);
    counters_.frames_captured.fetch_add(1, std::memory_order_relaxed);

    if (multi_ && connected_peers() == 0) {
        pipeline_->keep_dirty();
        pipeline_->release();
        return;
    }

    Nanoseconds encode_elapsed = 0;
    Outcome submitted = ok();
    {
        const ScopedTimer timer(encode_elapsed);
        submitted = encoder_->submit(classified);
    }
    if (!submitted.ok() && submitted.status() == Status::Full) {
        drain_encoder(kEncoderFullWaitMs);
        const ScopedTimer timer(encode_elapsed);
        submitted = encoder_->submit(classified);
    }
    if (!submitted.ok()) {
        pipeline_->keep_dirty();
    }
    pipeline_->release();

    if (!submitted.ok()) {
        note_encoder_refusal(submitted);
        return;
    }
    encode_ns_.record(encode_elapsed);
    drain_encoder(0);
}

void SenderSession::audio_thread_main()
{
    audio::AudioLease lease(*audio_source_);

    while (!stop_.load(std::memory_order_relaxed) && !audio_stop_.load(std::memory_order_relaxed)) {
        audio::CapturedAudio captured;
        const Outcome acquired = lease.acquire(captured, kAudioAcquireTimeoutMs);
        if (!acquired.ok()) {
            if (acquired.status() != Status::Timeout) {
                TL_LOG_WARN("emissor: captura de audio falhou (%s)", to_string(acquired.status()));
            }
            continue;
        }
        if (!captured.valid() || captured.frame_count == 0) {
            continue;
        }

        const std::uint16_t channels = captured.format.channels;
        const std::uint32_t frames = captured.frame_count > audio_scratch_frames_
                                         ? static_cast<std::uint32_t>(audio_scratch_frames_)
                                         : captured.frame_count;
        const std::size_t sample_count = static_cast<std::size_t>(frames) * channels;

        const std::int16_t* interleaved = nullptr;
        if (captured.format.sample_format == audio::SampleFormat::Int16) {
            interleaved = reinterpret_cast<const std::int16_t*>(captured.samples.data());
        } else if (captured.format.sample_format == audio::SampleFormat::Float32) {
            const auto* source = reinterpret_cast<const float*>(captured.samples.data());
            for (std::size_t index = 0; index < sample_count; ++index) {
                audio_scratch_[index] = to_int16(source[index]);
            }
            interleaved = audio_scratch_.get();
        } else {
            continue;
        }

        transport::PcmAudioBlock block;
        block.interleaved = Span<const std::int16_t>(interleaved, sample_count);
        block.capture_time_ns =
            captured.timestamp_valid ? captured.device_time_ns : captured.acquire_time_ns;
        block.sample_rate_hz = captured.format.sample_rate;
        block.frames_per_channel = frames;
        block.channels = channels;

        std::uint32_t delivered = 0;
        std::uint32_t refused = 0;
        {
            const std::lock_guard<std::mutex> guard(send_mutex_);
            for (const std::unique_ptr<Peer>& peer : peers_) {
                if (!peer || !peer_sendable(*peer)) {
                    continue;
                }
                if (peer->media->send_audio(block).ok()) {
                    ++delivered;
                } else {
                    ++refused;
                }
            }
        }

        if (delivered != 0) {
            counters_.audio_blocks.fetch_add(1, std::memory_order_relaxed);
            counters_.audio_bytes.fetch_add(block.interleaved.size_bytes(),
                                            std::memory_order_relaxed);
        }
        if (refused != 0) {
            counters_.send_failures.fetch_add(refused, std::memory_order_relaxed);
        }
    }
}

void SenderSession::report(Nanoseconds local_now_ns)
{
    if (options_.stats_interval_ns == 0) {
        return;
    }
    if (last_report_ns_ != 0 && local_now_ns - last_report_ns_ < options_.stats_interval_ns) {
        return;
    }
    last_report_ns_ = local_now_ns;

    const LatencyHistogram::Report capture = capture_ns_.report();
    const LatencyHistogram::Report encode = encode_ns_.report();

    transport::TransportStats network;
    double loss = 0.0;
    std::uint32_t viewers = 0;
    if (multi_) {
        for (const std::unique_ptr<Peer>& peer : peers_) {
            if (!peer || !peer_connected(*peer)) {
                continue;
            }
            const transport::TransportStats peer_stats = peer->media->stats();
            ++viewers;
            if (peer_stats.loss_ratio() > loss) {
                loss = peer_stats.loss_ratio();
            }
            if (peer_stats.round_trip_time_ns > network.round_trip_time_ns) {
                network.round_trip_time_ns = peer_stats.round_trip_time_ns;
            }
            if (peer_stats.target_bitrate_bps != 0 &&
                (network.target_bitrate_bps == 0 ||
                 peer_stats.target_bitrate_bps < network.target_bitrate_bps)) {
                network.target_bitrate_bps = peer_stats.target_bitrate_bps;
            }
        }
    } else {
        network = peers_[0]->media->stats();
        loss = network.loss_ratio();
    }

    if (machine_events_enabled()) {
        const encode::VideoEncoderInfo encoder_info = encoder_->info();
        MachineEvent event("stats");
        event.text("role", "sender")
            .integer("frames", counters_.frames_sent.load(std::memory_order_relaxed))
            .integer("audio_blocks", counters_.audio_blocks.load(std::memory_order_relaxed))
            .integer("width", encoder_info.width)
            .integer("height", encoder_info.height)
            .number("capture_ms", ns_to_ms(capture.p50_ns))
            .number("encode_ms", ns_to_ms(encode.p50_ns))
            .integer("bitrate_bps", network.target_bitrate_bps)
            .number("loss", loss)
            .number("rtt_ms", ns_to_ms(network.round_trip_time_ns))
            .integer("keyframes", counters_.keyframes.load(std::memory_order_relaxed));
        if (multi_) {
            event.integer("viewers", viewers);
        }
        return;
    }

    std::printf(
        "enviados %llu quadros e %llu blocos de audio | captura p50 %.2f p99 %.2f ms | encode "
        "p50 %.2f p99 %.2f ms | 1%% pior %.2f ms | bitrate alvo %.1f Mbps | perda %.2f%% | fila "
        "do pacer %u ms | rtt %.1f ms | keyframes %llu\n",
        static_cast<unsigned long long>(counters_.frames_sent.load(std::memory_order_relaxed)),
        static_cast<unsigned long long>(counters_.audio_blocks.load(std::memory_order_relaxed)),
        ns_to_ms(capture.p50_ns), ns_to_ms(capture.p99_ns), ns_to_ms(encode.p50_ns),
        ns_to_ms(encode.p99_ns), encode.low_one_percent_ns / 1e6,
        static_cast<double>(network.target_bitrate_bps) / 1e6, loss * 100.0, network.pacer_queue_ms,
        ns_to_ms(network.round_trip_time_ns),
        static_cast<unsigned long long>(counters_.keyframes.load(std::memory_order_relaxed)));
    std::fflush(stdout);
}

Outcome SenderSession::run_multi()
{
    const encode::VideoEncoderInfo encoder_info = encoder_->info();
    MachineEvent("ready")
        .integer("width", encoder_info.width)
        .integer("height", encoder_info.height);
    start_command_reader(stop_);
    TL_LOG_INFO("emissor: pronto para ate %u espectadores", kMaxSenderPeers);

    if (audio_source_) {
        audio_thread_ = std::thread(&SenderSession::audio_thread_main, this);
    }

    while (!stop_.load(std::memory_order_relaxed)) {
        MachineCommand command;
        while (!stop_.load(std::memory_order_relaxed) && take_machine_command(command)) {
            handle_command(command);
        }
        if (stop_.load(std::memory_order_relaxed)) {
            break;
        }

        service_peers();
        pump_video();
        report(now_ns());
        Logger::instance().drain_to_stderr();
    }

    stop_.store(true, std::memory_order_relaxed);
    if (audio_thread_.joinable()) {
        audio_thread_.join();
    }

    if (audio_source_) {
        audio_source_->stop();
    }
    close_video();
    for (std::uint32_t slot = 0; slot < kMaxSenderPeers; ++slot) {
        destroy_peer(detach_peer(slot));
    }
    return ok();
}

Outcome SenderSession::run()
{
    if (multi_) {
        return run_multi();
    }

    Peer& peer = *peers_[0];
    TL_TRY(peer.media->start(peer.observer));

    const Outcome negotiated = negotiate();
    if (!negotiated.ok()) {
        if (negotiated.status() == Status::Unavailable || negotiated.status() == Status::Timeout) {
            if (machine_events_enabled()) {
                MachineEvent("error")
                    .text("stage", "network")
                    .text("status", to_string(negotiated.status()))
                    .text("message", negotiated.error().context);
            } else {
                std::printf(
                    "\nNao foi possivel abrir o caminho direto ate a outra maquina.\n"
                    "Quando as duas pontas estao atras de NAT simetrico, o furo direto nao "
                    "acontece e so um servidor TURN resolve.\n"
                    "Tente de novo com --turn URL --turn-user USUARIO --turn-pass SENHA.\n\n");
                std::fflush(stdout);
            }
        }
        return negotiated;
    }

    TL_LOG_INFO("emissor: conectado, transmitindo");

    if (audio_source_) {
        audio_thread_ = std::thread(&SenderSession::audio_thread_main, this);
    }

    const bool machine = machine_events_enabled();
    while (!stop_.load(std::memory_order_relaxed)) {
        const auto state =
            static_cast<transport::ConnectionState>(peer.state.load(std::memory_order_relaxed));
        if (state == transport::ConnectionState::Failed ||
            state == transport::ConnectionState::Closed) {
            if (machine) {
                MachineEvent("error")
                    .text("stage", "connection_lost")
                    .text("status", to_string(state))
                    .text("message", "a conexao caiu e nao volta sozinha");
            } else {
                std::printf(
                    "\nA conexao caiu e nao volta sozinha.\n"
                    "Os enderecos foram combinados uma vez so, no inicio, entao trocar de cabo "
                    "para Wi-Fi ou de rede derruba de vez.\n"
                    "Abram o Telinha de novo nos dois lados e troquem um codigo novo.\n\n");
                std::fflush(stdout);
            }
            break;
        }

        if (machine) {
            MachineCommand command;
            while (!stop_.load(std::memory_order_relaxed) && take_machine_command(command)) {
                handle_command(command);
            }
            if (stop_.load(std::memory_order_relaxed)) {
                break;
            }
        }

        pump_video();
        report(now_ns());
        Logger::instance().drain_to_stderr();
    }

    stop_.store(true, std::memory_order_relaxed);
    if (audio_thread_.joinable()) {
        audio_thread_.join();
    }

    if (audio_source_) {
        audio_source_->stop();
    }
    close_video();
    peer.media->stop();
    return ok();
}

int run_sender(const SenderOptions& options) noexcept
{
    static SenderSession session;

    const Outcome initialized = session.initialize(options);
    if (!initialized.ok()) {
        Logger::instance().drain_to_stderr();
        if (machine_events_enabled()) {
            MachineEvent("error")
                .text("stage", "prepare")
                .text("status", to_string(initialized.status()))
                .text("message", initialized.error().context);
        } else {
            std::fprintf(stderr, "telinha: nao foi possivel preparar a transmissao: %s (%s)\n",
                         to_string(initialized.status()), initialized.error().context);
        }
        return 2;
    }

    const Outcome result = session.run();
    Logger::instance().drain_to_stderr();

    if (!result.ok()) {
        if (machine_events_enabled()) {
            MachineEvent("error")
                .text("stage", "session")
                .text("status", to_string(result.status()))
                .text("message", result.error().context);
        } else {
            std::fprintf(stderr, "telinha: transmissao terminou com erro: %s (%s)\n",
                         to_string(result.status()), result.error().context);
        }
        return 3;
    }
    return 0;
}

}  // namespace tl::app
