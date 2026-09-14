#include "telinha/app/receiver_session.hpp"

#include <chrono>
#include <cstdio>
#include <cstring>
#include <new>
#include <thread>

#include "telinha/app/machine_events.hpp"
#include "telinha/core/log.hpp"

namespace tl::app {
namespace {

constexpr Nanoseconds kPollInterval = 1ull * kNanosecondsPerMillisecond;
constexpr std::uint32_t kMaxDecodeSubmitsPerTick = 4;

void sleep_ns(Nanoseconds duration) noexcept
{
    std::this_thread::sleep_for(std::chrono::nanoseconds(duration));
}

constexpr Nanoseconds kKeyframeRequestGap = 500ull * kNanosecondsPerMillisecond;

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

}  // namespace

ReceiverSession::ReceiverSession() noexcept = default;

ReceiverSession::~ReceiverSession() = default;

Outcome ReceiverSession::initialize(const ReceiverOptions& options)
{
    options_ = options;

    say_step("reservando memoria");
    TL_TRY(video_queue_.reserve(options_.video_slot_bytes));
    TL_TRY(audio_queue_.reserve(options_.audio_slot_bytes));
    TL_TRY(jitter_.reserve(options_.jitter));
    TL_TRY(offset_.reserve(options_.clock));
    TL_TRY(signaling_.reserve(transport::TransportRole::Receiver));

    token_.reset(new (std::nothrow) char[kTokenCapacity]);
    remote_blob_.reset(new (std::nothrow) transport::SessionBlob());
    if (!token_ || !remote_blob_) {
        return fail(Status::OutOfMemory, "ReceiverSession::initialize");
    }

    say_step("preparando a janela");
    Result<std::unique_ptr<receive::VideoRenderer>> renderer =
        receive::create_video_renderer(options_.renderer);
    if (!renderer.ok()) {
        return Outcome{renderer.error()};
    }
    renderer_ = std::move(renderer).value();

    if (options_.render_audio) {
        Result<std::unique_ptr<receive::AudioRenderer>> audio =
            receive::create_audio_renderer(options_.audio_renderer);
        if (!audio.ok()) {
            TL_LOG_WARN("receptor: sem saida de audio (%s), seguindo so com video",
                        to_string(audio.error().status));
            options_.render_audio = false;
        } else {
            audio_renderer_ = std::move(audio).value();
        }
    }

    say_step("preparando a rede");

    for (std::uint32_t index = 0; index < options_.network.ice_server_count; ++index) {
        ice_servers_[index] = to_ice_server(options_.network.ice_servers[index]);
    }

    transport::TransportConfig config;
    config.role = transport::TransportRole::Receiver;
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

    Result<std::unique_ptr<transport::MediaTransport>> media =
        transport::create_media_transport(config);
    if (!media.ok()) {
        return Outcome{media.error()};
    }
    transport_ = std::move(media).value();

    say_step("pronto");
    return ok();
}

void ReceiverSession::request_stop() noexcept
{
    stop_.store(true, std::memory_order_relaxed);
}

void ReceiverSession::on_connection_state_changed(transport::ConnectionState state) noexcept
{
    state_.store(static_cast<std::uint32_t>(state), std::memory_order_relaxed);
    TL_LOG_INFO("receptor: conexao %s", to_string(state));
    if (machine_events_enabled()) {
        MachineEvent("state").text("state", to_string(state));
    }
}

void ReceiverSession::on_target_bitrate_changed(std::uint32_t bits_per_second,
                                                std::uint32_t framerate_hz) noexcept
{
    (void)bits_per_second;
    (void)framerate_hz;
}

void ReceiverSession::on_keyframe_requested() noexcept {}

void ReceiverSession::on_remote_video(const transport::EncodedVideoFrame& frame) noexcept
{
    if (!frame.valid()) {
        return;
    }

    const Nanoseconds arrival = now_ns();
    const std::size_t bytes = frame.bitstream.size_bytes();

    std::uint32_t slot = 0;
    std::byte* destination = video_queue_.begin_write(bytes, slot);
    if (destination == nullptr) {
        counters_.video_dropped.fetch_add(1, std::memory_order_relaxed);
        return;
    }
    std::memcpy(destination, frame.bitstream.data(), bytes);

    receive::VideoPacketHeader header;
    header.frame_index = frame.frame_index;
    header.remote_time_ns = frame.capture_time_ns;
    header.arrival_time_ns = arrival;
    header.slot = slot;
    header.byte_size = static_cast<std::uint32_t>(bytes);
    header.width = frame.width;
    header.height = frame.height;
    header.codec = frame.codec;
    header.temporal_index = frame.temporal_index;
    header.keyframe = frame.kind == transport::WireFrameKind::Key;
    video_queue_.commit(header);

    counters_.video_frames.fetch_add(1, std::memory_order_relaxed);
    counters_.video_bytes.fetch_add(bytes, std::memory_order_relaxed);
}

void ReceiverSession::on_remote_audio(const transport::PcmAudioBlock& block) noexcept
{
    if (!block.valid() || block.channels == 0) {
        return;
    }

    const Nanoseconds arrival = now_ns();
    const std::size_t bytes = block.interleaved.size_bytes();

    std::uint32_t slot = 0;
    std::byte* destination = audio_queue_.begin_write(bytes, slot);
    if (destination == nullptr) {
        counters_.audio_dropped.fetch_add(1, std::memory_order_relaxed);
        return;
    }
    std::memcpy(destination, block.interleaved.data(), bytes);

    receive::AudioPacketHeader header;
    header.remote_time_ns = block.capture_time_ns;
    header.arrival_time_ns = arrival;
    header.slot = slot;
    header.byte_size = static_cast<std::uint32_t>(bytes);
    header.sample_rate_hz = block.sample_rate_hz;
    header.frames_per_channel = block.frames_per_channel;
    header.channels = block.channels;
    audio_queue_.commit(header);

    counters_.audio_blocks.fetch_add(1, std::memory_order_relaxed);
    counters_.audio_frames.fetch_add(block.frames_per_channel, std::memory_order_relaxed);
}

void ReceiverSession::on_local_description(Span<const char> description) noexcept
{
    signaling_.on_description(description);
}

void ReceiverSession::on_local_candidate(Span<const char> candidate) noexcept
{
    signaling_.on_candidate(candidate);
}

void ReceiverSession::on_gathering_complete() noexcept
{
    signaling_.on_gathering_complete();
}

void ReceiverSession::on_round_trip_time(Nanoseconds round_trip_ns) noexcept
{
    counters_.round_trip_ns.store(round_trip_ns, std::memory_order_relaxed);
}

Outcome ReceiverSession::publish_local(const char* label)
{
    TL_TRY(transport_->create_local_description());

    const Nanoseconds deadline = now_ns() + options_.network.gather_timeout_ns;
    while (!signaling_.gathering_complete() && now_ns() < deadline &&
           !stop_.load(std::memory_order_relaxed)) {
        sleep_ns(20ull * kNanosecondsPerMillisecond);
    }

    if (!signaling_.has_description()) {
        return fail(Status::Timeout, "publish_local: sem descricao local");
    }

    if (signaling_.candidate_count() == 0) {
        TL_LOG_WARN("receptor: nenhum candidato reunido, a conexao provavelmente vai falhar");
    }
    if (signaling_.candidates_dropped() != 0) {
        TL_LOG_WARN("receptor: %u candidatos descartados por falta de espaco",
                    signaling_.candidates_dropped());
    }

    std::size_t length = 0;
    TL_TRY(signaling_.encode(token_.get(), kTokenCapacity, length));
    return publish_token(options_.signaling, "answer", label, token_.get(), length);
}

Outcome ReceiverSession::negotiate()
{
    TL_TRY(receive_remote_blob(options_.signaling, "invite", "convite de quem compartilha",
                               transport::TransportRole::Sender, token_.get(), kTokenCapacity,
                               *remote_blob_));
    if (machine_events_enabled()) {
        start_command_reader(stop_);
    }
    TL_TRY(apply_remote_blob(*transport_, *remote_blob_));
    TL_TRY(publish_local("resposta para quem compartilha"));
    return await_connection();
}

Outcome ReceiverSession::await_connection()
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

void ReceiverSession::ask_for_keyframe(const char* reason) noexcept
{
    if (!keyframe_requests_supported_) {
        return;
    }

    const Nanoseconds now = now_ns();
    if (last_keyframe_request_ns_ != 0 && now - last_keyframe_request_ns_ < kKeyframeRequestGap) {
        return;
    }
    last_keyframe_request_ns_ = now;

    const Outcome requested = transport_->request_keyframe();
    if (requested.ok()) {
        ++keyframe_requests_;
        TL_LOG_INFO("receptor: keyframe pedido (%s)", reason);
        return;
    }

    if (requested.status() == Status::NotImplemented ||
        requested.status() == Status::NotSupported) {
        keyframe_requests_supported_ = false;
        TL_LOG_WARN(
            "receptor: este transporte nao pede keyframe (%s), dependendo do keyframe periodico "
            "do emissor",
            to_string(requested.status()));
        return;
    }

    if (!keyframe_warned_) {
        keyframe_warned_ = true;
        TL_LOG_WARN(
            "receptor: pedido de keyframe recusado (%s), tentando de novo no proximo "
            "buraco",
            to_string(requested.status()));
    }
}

Outcome ReceiverSession::ensure_decoder(const receive::VideoPacketHeader& header)
{
    if (decoder_) {
        const receive::VideoDecoderInfo current = decoder_->info();
        if (current.width == header.width && current.height == header.height) {
            return ok();
        }
        decoder_->stop();
        decoder_.reset();
        TL_LOG_INFO("receptor: resolucao mudou para %ux%u, recriando o decoder", header.width,
                    header.height);
    }

    if (!header.keyframe) {
        return fail(Status::Unavailable, "ensure_decoder: aguardando keyframe");
    }

    receive::VideoDecoderConfig config = options_.video;
    config.codec = header.codec;
    config.width = header.width;
    config.height = header.height;

    const receive::VideoRendererInfo renderer_info = renderer_->info();
    Result<std::unique_ptr<receive::VideoDecoder>> created =
        receive::create_video_decoder(config, renderer_info.native_device);
    if (!created.ok()) {
        return Outcome{created.error()};
    }

    decoder_ = std::move(created).value();
    const Outcome started = decoder_->start();
    if (!started.ok()) {
        decoder_.reset();
        return started;
    }

    const receive::VideoDecoderInfo info = decoder_->info();
    TL_LOG_INFO("receptor: decoder %s %ux%u, saida %s", to_string(info.backend), info.width,
                info.height, to_string(info.output_format));
    return ok();
}

void ReceiverSession::drain_video() noexcept
{
    receive::VideoPacketHeader header;
    while (video_queue_.pop(header)) {
        offset_.observe(header.remote_time_ns, header.arrival_time_ns);

        std::uint32_t evicted = receive::kInvalidSlot;
        (void)jitter_.insert(header, evicted);
        if (evicted != receive::kInvalidSlot) {
            video_queue_.recycle(evicted);
        }
    }
}

void ReceiverSession::drain_audio(Nanoseconds local_now_ns)
{
    receive::AudioPacketHeader header;
    while (audio_queue_.pop(header)) {
        offset_.observe(header.remote_time_ns, header.arrival_time_ns);

        if (audio_renderer_) {
            receive::PcmFrame frame;
            frame.format.sample_rate_hz = header.sample_rate_hz;
            frame.format.channels = header.channels;
            frame.frames_per_channel = header.frames_per_channel;
            frame.remote_time_ns = header.remote_time_ns;

            const auto* samples =
                reinterpret_cast<const std::int16_t*>(audio_queue_.slot_data(header.slot));
            frame.interleaved =
                Span<const std::int16_t>(samples, header.byte_size / sizeof(std::int16_t));

            const Outcome submitted = audio_renderer_->submit(frame);
            if (!submitted.ok()) {
                ++audio_submit_failures_;
            } else {
                const Nanoseconds audible_at = local_now_ns + audio_renderer_->buffered_ns();
                const Nanoseconds nominal = offset_.to_local(header.remote_time_ns);
                if (audible_at > nominal) {
                    jitter_.observe_audio_delay(audible_at - nominal);
                }
            }
        }

        audio_queue_.recycle(header.slot);
    }
}

void ReceiverSession::feed_decoder(Nanoseconds local_now_ns)
{
    for (std::uint32_t iteration = 0; iteration < kMaxDecodeSubmitsPerTick; ++iteration) {
        receive::VideoPacketHeader header;
        const receive::JitterPull pulled = jitter_.pull(local_now_ns, offset_, header);

        if (pulled == receive::JitterPull::Empty || pulled == receive::JitterPull::Waiting) {
            return;
        }
        if (pulled == receive::JitterPull::Discard) {
            video_queue_.recycle(header.slot);
            continue;
        }

        const Outcome ready = ensure_decoder(header);
        if (ready.ok() && awaiting_keyframe_ && !header.keyframe) {
            counters_.video_dropped.fetch_add(1, std::memory_order_relaxed);
        } else if (ready.ok()) {
            transport::EncodedVideoFrame frame;
            frame.bitstream =
                Span<const std::byte>(video_queue_.slot_data(header.slot), header.byte_size);
            frame.capture_time_ns = header.remote_time_ns;
            frame.frame_index = header.frame_index;
            frame.width = header.width;
            frame.height = header.height;
            frame.codec = header.codec;
            frame.kind =
                header.keyframe ? transport::WireFrameKind::Key : transport::WireFrameKind::Delta;
            frame.temporal_index = header.temporal_index;

            const Outcome submitted = decoder_->submit(frame);
            if (submitted.ok()) {
                ++decode_submits_;
                awaiting_keyframe_ = false;
            } else {
                ++decode_failures_;
                ++decode_refusals_since_log_;
                awaiting_keyframe_ = true;
                ask_for_keyframe("decoder recusou um quadro");
                if (last_decode_log_ns_ == 0 ||
                    local_now_ns - last_decode_log_ns_ >= kNanosecondsPerSecond) {
                    TL_LOG_WARN(
                        "receptor: decoder recusou %llu quadros (%s, %s, 0x%08X), congelando ate o "
                        "proximo keyframe",
                        static_cast<unsigned long long>(decode_refusals_since_log_),
                        to_string(submitted.status()), submitted.error().context,
                        static_cast<unsigned>(submitted.error().platform_code));
                    decode_refusals_since_log_ = 0;
                    last_decode_log_ns_ = local_now_ns;
                }
            }
        }

        video_queue_.recycle(header.slot);
    }
}

void ReceiverSession::present_next()
{
    if (!decoder_) {
        return;
    }

    receive::DecodedVideoFrame frame;
    const Outcome polled = decoder_->poll(frame, 0);
    if (!polled.ok()) {
        if (polled.status() != Status::WouldBlock && !poll_warned_) {
            poll_warned_ = true;
            TL_LOG_WARN("receptor: decoder nao entregou quadro (%s, %s, 0x%08X)",
                        to_string(polled.status()), polled.error().context,
                        static_cast<unsigned>(polled.error().platform_code));
        }
        return;
    }

    const Outcome presented = renderer_->present(frame);
    decoder_->release();

    if (!presented.ok()) {
        TL_LOG_WARN("receptor: apresentacao falhou (%s)", to_string(presented.status()));
    }
}

void ReceiverSession::report(Nanoseconds local_now_ns)
{
    if (options_.stats_interval_ns == 0) {
        return;
    }
    if (last_report_ns_ != 0 && local_now_ns - last_report_ns_ < options_.stats_interval_ns) {
        return;
    }
    last_report_ns_ = local_now_ns;

    const receive::JitterStats& jitter = jitter_.stats();
    const LatencyHistogram::Report queue = jitter.queue_delay_ns.report();
    const receive::RendererStats& present = renderer_->stats();
    const LatencyHistogram::Report interval = present.present_interval_ns.report();
    const transport::TransportStats network = transport_->stats();

    if (machine_events_enabled()) {
        MachineEvent("stats")
            .text("role", "receiver")
            .integer("frames", counters_.video_frames.load(std::memory_order_relaxed))
            .integer("presented", present.frames_presented)
            .integer("audio_blocks", counters_.audio_blocks.load(std::memory_order_relaxed))
            .number("delay_ms", ns_to_ms(jitter.current_delay_ns))
            .number("present_interval_ms", ns_to_ms(interval.p50_ns))
            .integer("dropped", counters_.video_dropped.load(std::memory_order_relaxed))
            .integer("gaps", jitter.gaps)
            .number("loss", network.loss_ratio())
            .number("rtt_ms", ns_to_ms(counters_.round_trip_ns.load(std::memory_order_relaxed)));
        return;
    }

    std::printf(
        "recebidos %llu quadros e %llu blocos de audio | atraso de reproducao %.1f ms | fila "
        "p50 %.1f p99 %.1f ms | intervalo p50 %.1f p99 %.1f ms | 1%% pior %.1f ms | perdidos "
        "%llu na fila %llu buracos | perda de pacotes %.2f%% | rtt %.1f ms\n",
        static_cast<unsigned long long>(counters_.video_frames.load(std::memory_order_relaxed)),
        static_cast<unsigned long long>(counters_.audio_blocks.load(std::memory_order_relaxed)),
        ns_to_ms(jitter.current_delay_ns), ns_to_ms(queue.p50_ns), ns_to_ms(queue.p99_ns),
        ns_to_ms(interval.p50_ns), ns_to_ms(interval.p99_ns), interval.low_one_percent_ns / 1e6,
        static_cast<unsigned long long>(counters_.video_dropped.load(std::memory_order_relaxed)),
        static_cast<unsigned long long>(jitter.gaps), network.loss_ratio() * 100.0,
        ns_to_ms(counters_.round_trip_ns.load(std::memory_order_relaxed)));
    std::fflush(stdout);
}

Outcome ReceiverSession::run()
{
    TL_TRY(renderer_->start());
    if (audio_renderer_) {
        const Outcome audio_started = audio_renderer_->start();
        if (!audio_started.ok()) {
            TL_LOG_WARN("receptor: saida de audio nao abriu (%s), seguindo so com video",
                        to_string(audio_started.status()));
            audio_renderer_.reset();
        }
    }

    TL_TRY(transport_->start(*this));

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

    TL_LOG_INFO("receptor: conectado, aguardando video");

    ask_for_keyframe("inicio da sessao");

    bool fullscreen = renderer_->fullscreen();
    while (!stop_.load(std::memory_order_relaxed)) {
        bool close_requested = false;
        const Outcome pumped = renderer_->pump(close_requested);
        if (!pumped.ok()) {
            return pumped;
        }
        if (close_requested) {
            break;
        }

        MachineCommand command;
        while (take_machine_command(command)) {
            if (command.kind == MachineCommandKind::SetFullscreen) {
                renderer_->set_fullscreen(command.enabled);
            } else if (command.kind == MachineCommandKind::SetVolume) {
                volume_percent_ = command.volume > receive::kMaxVolumePercent
                                      ? receive::kMaxVolumePercent
                                      : command.volume;
                if (audio_renderer_) {
                    audio_renderer_->set_volume(volume_percent_);
                }
                MachineEvent("volume").integer("volume", volume_percent_);
            }
        }
        if (machine_events_enabled() && renderer_->fullscreen() != fullscreen) {
            fullscreen = renderer_->fullscreen();
            MachineEvent("fullscreen").flag("enabled", fullscreen);
        }

        const auto state =
            static_cast<transport::ConnectionState>(state_.load(std::memory_order_relaxed));
        if (state == transport::ConnectionState::Failed ||
            state == transport::ConnectionState::Closed) {
            if (machine_events_enabled()) {
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

        const Nanoseconds local_now = now_ns();

        drain_audio(local_now);
        drain_video();
        jitter_.update_delay(offset_.spread_ns());
        feed_decoder(local_now);
        present_next();
        report(local_now);

        if (jitter_.take_keyframe_request()) {
            awaiting_keyframe_ = true;
            ask_for_keyframe("buraco no video");
        }

        Logger::instance().drain_to_stderr();
        sleep_ns(kPollInterval);
    }

    if (decoder_) {
        decoder_->stop();
    }
    if (audio_renderer_) {
        audio_renderer_->stop();
    }
    renderer_->stop();
    transport_->stop();
    return ok();
}

}  // namespace tl::app
