#include "telinha/app/sender_session.hpp"

#include <chrono>
#include <cstdio>
#include <cstring>
#include <new>

#include "telinha/core/log.hpp"

namespace tl::app {
namespace {

constexpr std::uint32_t kTargetListCapacity = 128;
constexpr std::uint32_t kAudioScratchFrames = 4096;
constexpr std::uint32_t kAudioAcquireTimeoutMs = 40;

void sleep_ns(Nanoseconds duration) noexcept
{
    std::this_thread::sleep_for(std::chrono::nanoseconds(duration));
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

}  // namespace

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

void SenderSession::on_connection_state_changed(transport::ConnectionState state) noexcept
{
    state_.store(static_cast<std::uint32_t>(state), std::memory_order_relaxed);
    TL_LOG_INFO("emissor: conexao %s", to_string(state));
}

void SenderSession::on_target_bitrate_changed(std::uint32_t bits_per_second,
                                              std::uint32_t framerate_hz) noexcept
{
    (void)framerate_hz;
    pending_bitrate_.store(bits_per_second, std::memory_order_relaxed);
}

void SenderSession::on_keyframe_requested() noexcept
{
    keyframe_pending_.store(true, std::memory_order_relaxed);
}

void SenderSession::on_packet_loss_detected(double loss_ratio) noexcept
{
    if (loss_ratio > 0.1) {
        keyframe_pending_.store(true, std::memory_order_relaxed);
    }
}

void SenderSession::on_local_description(Span<const char> description) noexcept
{
    signaling_.on_description(description);
}

void SenderSession::on_local_candidate(Span<const char> candidate) noexcept
{
    signaling_.on_candidate(candidate);
}

void SenderSession::on_round_trip_time(Nanoseconds round_trip_ns) noexcept
{
    (void)round_trip_ns;
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

    TL_TRY(pipeline_.initialize(std::move(source).value(), capture_options));
    TL_TRY(pipeline_.start());

    const capture::CaptureSourceInfo info = pipeline_.info();
    TL_LOG_INFO("emissor: compartilhando %s %ux%u via %s", chosen.name, info.width, info.height,
                to_string(info.backend));
    return ok();
}

Outcome SenderSession::open_encoder(const capture::CaptureSourceInfo& info)
{
    encode::VideoEncoderConfig config;
    config.codec = encode::VideoCodec::H264;
    config.width = info.width;
    config.height = info.height;
    config.framerate_millihertz = options_.framerate_millihertz;
    config.target_bitrate_bps = options_.network.start_bitrate_bps;
    config.max_bitrate_bps = options_.network.max_bitrate_bps;
    config.coding_unit_size = encode::coding_unit_size_for(config.codec);
    config.intra_refresh = options_.intra_refresh;

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

    const encode::VideoEncoderInfo encoder_info = encoder_->info();
    TL_LOG_INFO("emissor: encoder %s %ux%u, regioes sujas %s, intra refresh %s",
                to_string(encoder_info.backend), encoder_info.width, encoder_info.height,
                encoder_info.supports_dirty_regions ? "sim" : "nao",
                encoder_info.supports_intra_refresh ? "sim" : "nao");
    return ok();
}

Outcome SenderSession::open_audio()
{
    if (options_.audio_scope == AudioScope::None) {
        return ok();
    }

    audio::AudioCaptureTarget target;
    if (options_.audio_scope == AudioScope::Process) {
        const std::uint32_t pid = options_.audio_process_id;
        if (pid == 0) {
            TL_LOG_ERROR("emissor: --audio process precisa de --audio-pid com o processo alvo");
            return fail(Status::InvalidArgument, "open_audio");
        }
        target = audio::AudioCaptureTarget::process_loopback(pid);
    } else {
        target = audio::AudioCaptureTarget::system_loopback();
    }

    audio::AudioCaptureOptions capture_options;
    capture_options.requested_format = audio::AudioFormat{48000, 2, audio::SampleFormat::Int16};

    Result<std::unique_ptr<audio::AudioSource>> created =
        audio::create_audio_source(target, capture_options);
    if (!created.ok()) {
        return Outcome{created.error()};
    }
    audio_source_ = std::move(created).value();

    const audio::AudioSourceInfo info = audio_source_->info();

    if (options_.audio_scope == AudioScope::Process &&
        info.target.scope != audio::AudioCaptureScope::ProcessLoopback) {
        audio_source_.reset();
        std::printf(
            "\nATENCAO: esta versao do Windows nao separa o audio por programa.\n"
            "Voce pediu o audio de um programa so, mas o sistema so consegue entregar o audio "
            "de tudo que estiver tocando, incluindo notificacoes, navegador e chamadas.\n"
            "A transmissao nao vai comecar assim. Escolha uma das duas:\n"
            "  --audio system   aceita transmitir o audio de todo o sistema\n"
            "  --audio none     transmite so a imagem, sem audio\n\n");
        std::fflush(stdout);
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
    return ok();
}

Outcome SenderSession::open_transport()
{
    transport::IceServer servers[kMaxIceServers];
    for (std::uint32_t index = 0; index < options_.network.ice_server_count; ++index) {
        servers[index] = to_ice_server(options_.network.ice_servers[index]);
    }

    transport::TransportConfig config;
    config.role = transport::TransportRole::Sender;
    config.candidate_policy = options_.network.candidate_policy;
    config.ice_servers =
        Span<const transport::IceServer>(servers, options_.network.ice_server_count);
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
    transport_ = std::move(created).value();
    return ok();
}

Outcome SenderSession::initialize(const SenderOptions& options)
{
    options_ = options;

    TL_TRY(open_capture());
    TL_TRY(open_encoder(pipeline_.info()));
    TL_TRY(open_audio());
    TL_TRY(open_transport());
    return ok();
}

Outcome SenderSession::negotiate()
{
    TL_TRY(transport_->create_local_description());

    const Nanoseconds deadline = now_ns() + options_.network.gather_timeout_ns;
    while (!signaling_.gathering_complete() && now_ns() < deadline &&
           !stop_.load(std::memory_order_relaxed)) {
        sleep_ns(20ull * kNanosecondsPerMillisecond);
    }

    if (!signaling_.has_description()) {
        return fail(Status::Timeout, "negotiate: sem descricao local");
    }

    SignalingPayload local;
    signaling_.snapshot(local);
    if (local.candidate_count == 0) {
        TL_LOG_WARN("emissor: nenhum candidato reunido, a conexao provavelmente vai falhar");
    }

    static thread_local char token[kTokenCapacity];
    std::size_t length = 0;
    TL_TRY(encode_signaling_token(local, token, sizeof(token), length));
    TL_TRY(publish_token(options_.signaling, "convite para quem vai assistir", token, length));

    length = 0;
    TL_TRY(consume_token(options_.signaling, "resposta de quem vai assistir", token, sizeof(token),
                         length));

    SignalingPayload remote;
    TL_TRY(decode_signaling_token(Span<const char>(token, length), remote));
    TL_TRY(transport_->set_remote_description(
        Span<const char>(remote.description, remote.description_length)));

    for (std::uint32_t index = 0; index < remote.candidate_count; ++index) {
        const Outcome added = transport_->add_remote_candidate(
            Span<const char>(remote.candidates[index], remote.candidate_length[index]));
        if (!added.ok()) {
            TL_LOG_WARN("emissor: candidato remoto recusado (%s)", to_string(added.status()));
        }
    }

    return await_connection();
}

Outcome SenderSession::await_connection()
{
    const Nanoseconds deadline = now_ns() + options_.network.connect_timeout_ns;

    while (!stop_.load(std::memory_order_relaxed)) {
        const auto state =
            static_cast<transport::ConnectionState>(state_.load(std::memory_order_relaxed));
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

void SenderSession::apply_pending_controls() noexcept
{
    const std::uint32_t bitrate = pending_bitrate_.exchange(0, std::memory_order_relaxed);
    if (bitrate != 0) {
        const Outcome applied = encoder_->set_target_bitrate(bitrate);
        if (applied.ok()) {
            counters_.bitrate_updates.fetch_add(1, std::memory_order_relaxed);
        } else {
            TL_LOG_WARN("emissor: encoder recusou %u bps (%s)", bitrate,
                        to_string(applied.status()));
        }
    }

    if (keyframe_pending_.exchange(false, std::memory_order_relaxed)) {
        encoder_->request_keyframe();
        counters_.keyframes.fetch_add(1, std::memory_order_relaxed);
    }
}

void SenderSession::pump_video()
{
    apply_pending_controls();

    capture::ClassifiedFrame classified;
    Nanoseconds capture_elapsed = 0;
    Outcome captured = ok();
    {
        const ScopedTimer timer(capture_elapsed);
        captured = pipeline_.capture_next(classified, options_.capture_timeout_ms);
    }

    if (!captured.ok()) {
        if (captured.status() != Status::Timeout) {
            TL_LOG_WARN("emissor: captura falhou (%s)", to_string(captured.status()));
        }
        return;
    }

    capture_ns_.record(capture_elapsed);
    counters_.frames_captured.fetch_add(1, std::memory_order_relaxed);

    Nanoseconds encode_elapsed = 0;
    Outcome submitted = ok();
    {
        const ScopedTimer timer(encode_elapsed);
        submitted = encoder_->submit(classified);
    }
    pipeline_.release();

    if (!submitted.ok()) {
        TL_LOG_WARN("emissor: encoder recusou o quadro (%s)", to_string(submitted.status()));
        return;
    }
    encode_ns_.record(encode_elapsed);

    transport::EncodedVideoFrame frame;
    const Outcome polled = encoder_->poll(frame, 0);
    if (!polled.ok()) {
        return;
    }

    counters_.frames_encoded.fetch_add(1, std::memory_order_relaxed);

    Nanoseconds send_elapsed = 0;
    Outcome sent = ok();
    {
        const ScopedTimer timer(send_elapsed);
        const std::lock_guard<std::mutex> guard(send_mutex_);
        sent = transport_->send_video(frame);
    }
    const std::size_t bytes = frame.bitstream.size_bytes();
    encoder_->release();

    if (!sent.ok()) {
        counters_.send_failures.fetch_add(1, std::memory_order_relaxed);
        return;
    }

    send_ns_.record(send_elapsed);
    counters_.frames_sent.fetch_add(1, std::memory_order_relaxed);
    counters_.video_bytes.fetch_add(bytes, std::memory_order_relaxed);
}

void SenderSession::audio_thread_main()
{
    audio::AudioLease lease(*audio_source_);

    while (!stop_.load(std::memory_order_relaxed)) {
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

        Outcome sent = ok();
        {
            const std::lock_guard<std::mutex> guard(send_mutex_);
            sent = transport_->send_audio(block);
        }

        if (sent.ok()) {
            counters_.audio_blocks.fetch_add(1, std::memory_order_relaxed);
            counters_.audio_bytes.fetch_add(block.interleaved.size_bytes(),
                                            std::memory_order_relaxed);
        } else {
            counters_.send_failures.fetch_add(1, std::memory_order_relaxed);
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
    const transport::TransportStats network = transport_->stats();

    std::printf(
        "enviados %llu quadros e %llu blocos de audio | captura p50 %.2f p99 %.2f ms | encode "
        "p50 %.2f p99 %.2f ms | 1%% pior %.2f ms | bitrate alvo %.1f Mbps | perda %.2f%% | fila "
        "do pacer %u ms | rtt %.1f ms | keyframes %llu\n",
        static_cast<unsigned long long>(counters_.frames_sent.load(std::memory_order_relaxed)),
        static_cast<unsigned long long>(counters_.audio_blocks.load(std::memory_order_relaxed)),
        ns_to_ms(capture.p50_ns), ns_to_ms(capture.p99_ns), ns_to_ms(encode.p50_ns),
        ns_to_ms(encode.p99_ns), encode.low_one_percent_ns / 1e6,
        static_cast<double>(network.target_bitrate_bps) / 1e6, network.loss_ratio() * 100.0,
        network.pacer_queue_ms, ns_to_ms(network.round_trip_time_ns),
        static_cast<unsigned long long>(counters_.keyframes.load(std::memory_order_relaxed)));
    std::fflush(stdout);
}

Outcome SenderSession::run()
{
    TL_TRY(transport_->start(*this));

    const Outcome negotiated = negotiate();
    if (!negotiated.ok()) {
        if (negotiated.status() == Status::Unavailable || negotiated.status() == Status::Timeout) {
            std::printf(
                "\nNao foi possivel abrir o caminho direto ate a outra maquina.\n"
                "Quando as duas pontas estao atras de NAT simetrico, o furo direto nao acontece "
                "e so um servidor TURN resolve.\n"
                "Tente de novo com --turn URL --turn-user USUARIO --turn-pass SENHA.\n\n");
            std::fflush(stdout);
        }
        return negotiated;
    }

    TL_LOG_INFO("emissor: conectado, transmitindo");

    if (audio_source_) {
        audio_thread_ = std::thread(&SenderSession::audio_thread_main, this);
    }

    while (!stop_.load(std::memory_order_relaxed)) {
        const auto state =
            static_cast<transport::ConnectionState>(state_.load(std::memory_order_relaxed));
        if (state == transport::ConnectionState::Failed ||
            state == transport::ConnectionState::Closed) {
            TL_LOG_WARN("emissor: a conexao caiu");
            break;
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
    encoder_->stop();
    pipeline_.stop();
    transport_->stop();
    return ok();
}

int run_sender(const SenderOptions& options) noexcept
{
    static SenderSession session;

    const Outcome initialized = session.initialize(options);
    if (!initialized.ok()) {
        Logger::instance().drain_to_stderr();
        std::fprintf(stderr, "telinha: nao foi possivel preparar a transmissao: %s (%s)\n",
                     to_string(initialized.status()), initialized.error().context);
        return 2;
    }

    const Outcome result = session.run();
    Logger::instance().drain_to_stderr();

    if (!result.ok()) {
        std::fprintf(stderr, "telinha: transmissao terminou com erro: %s (%s)\n",
                     to_string(result.status()), result.error().context);
        return 3;
    }
    return 0;
}

}  // namespace tl::app
