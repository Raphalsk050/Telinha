#include "telinha/app/app_options.hpp"

#include <cstdio>
#include <cstdlib>
#include <cstring>

namespace tl::app {
namespace {

bool copy_text(char* destination, std::size_t capacity, const char* source) noexcept
{
    const std::size_t length = std::strlen(source);
    if (length + 1 > capacity) {
        return false;
    }
    std::memcpy(destination, source, length + 1);
    return true;
}

bool equals(const char* a, const char* b) noexcept
{
    return std::strcmp(a, b) == 0;
}

bool parse_u32(const char* text, std::uint32_t& out) noexcept
{
    char* end = nullptr;
    const unsigned long value = std::strtoul(text, &end, 10);
    if (end == text || *end != '\0' || value > 0xFFFFFFFFul) {
        return false;
    }
    out = static_cast<std::uint32_t>(value);
    return true;
}

void set_error(char* error, int capacity, const char* format, const char* detail) noexcept
{
    if (error == nullptr || capacity <= 0) {
        return;
    }
    std::snprintf(error, static_cast<std::size_t>(capacity), format, detail);
}

void apply_defaults(AppOptions& options) noexcept
{
    NetworkOptions& sender_network = options.sender.network;
    copy_text(sender_network.ice_servers[0].url, kUrlCapacity, "stun:stun.l.google.com:19302");
    copy_text(sender_network.ice_servers[1].url, kUrlCapacity, "stun:stun.cloudflare.com:3478");
    sender_network.ice_server_count = 2;
    options.receiver.network = sender_network;

    copy_text(options.receiver.renderer.title, receive::kWindowTitleCapacity, "Telinha");
}

}  // namespace

const char* to_string(AppMode mode) noexcept
{
    switch (mode) {
        case AppMode::None: return "None";
        case AppMode::Usage: return "Usage";
        case AppMode::List: return "List";
        case AppMode::Probe: return "Probe";
        case AppMode::Send: return "Send";
        case AppMode::Receive: return "Receive";
    }
    return "Unknown";
}

const char* to_string(AudioScope scope) noexcept
{
    switch (scope) {
        case AudioScope::None: return "None";
        case AudioScope::System: return "System";
        case AudioScope::Process: return "Process";
    }
    return "Unknown";
}

void print_usage() noexcept
{
    std::printf(
        "telinha - compartilhamento de tela ponto a ponto\n"
        "\n"
        "  telinha list [monitores|janelas]\n"
        "  telinha probe\n"
        "  telinha send [opcoes]\n"
        "  telinha recv [opcoes]\n"
        "\n"
        "Alvo (send):\n"
        "  --monitor N            compartilha o monitor de indice N\n"
        "  --window N             compartilha a janela de indice N\n"
        "  --no-cursor            nao desenha o cursor\n"
        "\n"
        "Video (send):\n"
        "  --fps N                quadros por segundo, padrao 60\n"
        "  --bitrate N            alvo em kbps, padrao 8000\n"
        "  --max-bitrate N        teto em kbps, padrao 40000\n"
        "  --min-bitrate N        piso em kbps, padrao 500\n"
        "\n"
        "Audio (send):\n"
        "  --audio system         audio de todo o sistema, padrao\n"
        "  --audio process        audio apenas do programa compartilhado\n"
        "  --audio none           sem audio\n"
        "  --audio-pid N          processo explicito para --audio process\n"
        "\n"
        "Rede:\n"
        "  --stun URL             servidor STUN, repetivel\n"
        "  --turn URL             servidor TURN, ativa relay\n"
        "  --turn-user U          usuario do TURN\n"
        "  --turn-pass P          senha do TURN\n"
        "  --ice host|stun|all    politica de candidatos, padrao stun\n"
        "  --port-min N           menor porta local\n"
        "  --port-max N           maior porta local\n"
        "\n"
        "Sinalizacao:\n"
        "  --signal-in ARQUIVO    le o convite do parceiro deste arquivo\n"
        "  --signal-out ARQUIVO   escreve o proprio convite neste arquivo\n"
        "                         sem esses argumentos usa a entrada e a saida padrao\n"
        "\n"
        "Janela (recv):\n"
        "  --width N              largura inicial, padrao 1280\n"
        "  --height N             altura inicial, padrao 720\n"
        "  --fullscreen           abre em tela cheia\n"
        "  --headless             nao abre janela, so mede\n"
        "  --no-audio             nao reproduz o audio recebido\n"
        "  --jitter-ms N          atraso inicial de reproducao, padrao 60\n"
        "  --max-jitter-ms N      atraso maximo de reproducao, padrao 400\n"
        "\n"
        "Geral:\n"
        "  --log NIVEL            trace, debug, info, warn, error\n"
        "  --help                 mostra esta ajuda\n");
}

Outcome parse_command_line(int argc, const char* const* argv, AppOptions& out, char* error,
                           int error_capacity) noexcept
{
    apply_defaults(out);

    if (argc < 2) {
        out.mode = AppMode::Usage;
        return ok();
    }

    const char* command = argv[1];
    if (equals(command, "list")) {
        out.mode = AppMode::List;
        out.list_kind = capture::CaptureTargetKind::None;
    } else if (equals(command, "probe")) {
        out.mode = AppMode::Probe;
    } else if (equals(command, "send")) {
        out.mode = AppMode::Send;
    } else if (equals(command, "recv") || equals(command, "receive")) {
        out.mode = AppMode::Receive;
    } else if (equals(command, "--help") || equals(command, "-h") || equals(command, "help")) {
        out.mode = AppMode::Usage;
        return ok();
    } else {
        set_error(error, error_capacity, "comando desconhecido: %s", command);
        return fail(Status::InvalidArgument, "parse_command_line");
    }

    NetworkOptions& network =
        out.mode == AppMode::Receive ? out.receiver.network : out.sender.network;
    SignalingOptions& signaling =
        out.mode == AppMode::Receive ? out.receiver.signaling : out.sender.signaling;

    std::uint32_t turn_index = 0;
    bool turn_present = false;

    for (int index = 2; index < argc; ++index) {
        const char* argument = argv[index];
        const bool has_value = index + 1 < argc;
        const char* value = has_value ? argv[index + 1] : "";

        if (out.mode == AppMode::List && argument[0] != '-') {
            if (equals(argument, "monitores") || equals(argument, "monitors")) {
                out.list_kind = capture::CaptureTargetKind::Monitor;
            } else if (equals(argument, "janelas") || equals(argument, "windows")) {
                out.list_kind = capture::CaptureTargetKind::Window;
            } else {
                set_error(error, error_capacity, "alvo desconhecido para list: %s", argument);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            continue;
        }

        if (equals(argument, "--help") || equals(argument, "-h")) {
            out.mode = AppMode::Usage;
            return ok();
        }

        if (equals(argument, "--no-cursor")) {
            out.sender.include_cursor = false;
            continue;
        }
        if (equals(argument, "--fullscreen")) {
            out.receiver.renderer.start_fullscreen = true;
            continue;
        }
        if (equals(argument, "--headless")) {
            out.receiver.renderer.backend = receive::RendererBackend::Headless;
            out.receiver.audio_renderer.backend = receive::AudioRendererBackend::Headless;
            continue;
        }
        if (equals(argument, "--no-audio")) {
            out.receiver.render_audio = false;
            continue;
        }

        if (!has_value) {
            set_error(error, error_capacity, "falta o valor de %s", argument);
            return fail(Status::InvalidArgument, "parse_command_line");
        }
        ++index;

        std::uint32_t number = 0;

        if (equals(argument, "--monitor")) {
            if (!parse_u32(value, number)) {
                set_error(error, error_capacity, "indice invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            out.sender.target_kind = capture::CaptureTargetKind::Monitor;
            out.sender.target_index = static_cast<std::int32_t>(number);
        } else if (equals(argument, "--window")) {
            if (!parse_u32(value, number)) {
                set_error(error, error_capacity, "indice invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            out.sender.target_kind = capture::CaptureTargetKind::Window;
            out.sender.target_index = static_cast<std::int32_t>(number);
        } else if (equals(argument, "--fps")) {
            if (!parse_u32(value, number) || number == 0 || number > 240) {
                set_error(error, error_capacity, "fps invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            out.sender.framerate_millihertz = number * 1000u;
        } else if (equals(argument, "--bitrate")) {
            if (!parse_u32(value, number) || number == 0) {
                set_error(error, error_capacity, "bitrate invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            network.start_bitrate_bps = number * 1000u;
        } else if (equals(argument, "--max-bitrate")) {
            if (!parse_u32(value, number) || number == 0) {
                set_error(error, error_capacity, "bitrate invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            network.max_bitrate_bps = number * 1000u;
        } else if (equals(argument, "--min-bitrate")) {
            if (!parse_u32(value, number) || number == 0) {
                set_error(error, error_capacity, "bitrate invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            network.min_bitrate_bps = number * 1000u;
        } else if (equals(argument, "--audio")) {
            if (equals(value, "system") || equals(value, "sistema")) {
                out.sender.audio_scope = AudioScope::System;
            } else if (equals(value, "process") || equals(value, "processo")) {
                out.sender.audio_scope = AudioScope::Process;
            } else if (equals(value, "none") || equals(value, "nenhum")) {
                out.sender.audio_scope = AudioScope::None;
            } else {
                set_error(error, error_capacity, "escopo de audio invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
        } else if (equals(argument, "--audio-pid")) {
            if (!parse_u32(value, number)) {
                set_error(error, error_capacity, "pid invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            out.sender.audio_process_id = number;
        } else if (equals(argument, "--stun")) {
            if (network.ice_server_count >= kMaxIceServers) {
                set_error(error, error_capacity, "servidores ICE demais: %s", value);
                return fail(Status::OutOfRange, "parse_command_line");
            }
            if (!copy_text(network.ice_servers[network.ice_server_count].url, kUrlCapacity,
                           value)) {
                set_error(error, error_capacity, "url longa demais: %s", value);
                return fail(Status::OutOfRange, "parse_command_line");
            }
            ++network.ice_server_count;
        } else if (equals(argument, "--turn")) {
            if (network.ice_server_count >= kMaxIceServers) {
                set_error(error, error_capacity, "servidores ICE demais: %s", value);
                return fail(Status::OutOfRange, "parse_command_line");
            }
            turn_index = network.ice_server_count;
            turn_present = true;
            if (!copy_text(network.ice_servers[turn_index].url, kUrlCapacity, value)) {
                set_error(error, error_capacity, "url longa demais: %s", value);
                return fail(Status::OutOfRange, "parse_command_line");
            }
            ++network.ice_server_count;
            network.candidate_policy = transport::CandidatePolicy::All;
        } else if (equals(argument, "--turn-user")) {
            if (!turn_present) {
                set_error(error, error_capacity, "--turn-user exige --turn antes%s", "");
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            if (!copy_text(network.ice_servers[turn_index].username, kCredentialCapacity, value)) {
                set_error(error, error_capacity, "usuario longo demais: %s", value);
                return fail(Status::OutOfRange, "parse_command_line");
            }
        } else if (equals(argument, "--turn-pass")) {
            if (!turn_present) {
                set_error(error, error_capacity, "--turn-pass exige --turn antes%s", "");
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            if (!copy_text(network.ice_servers[turn_index].credential, kCredentialCapacity,
                           value)) {
                set_error(error, error_capacity, "senha longa demais: %s", value);
                return fail(Status::OutOfRange, "parse_command_line");
            }
        } else if (equals(argument, "--ice")) {
            if (equals(value, "host")) {
                network.candidate_policy = transport::CandidatePolicy::HostOnly;
            } else if (equals(value, "stun")) {
                network.candidate_policy = transport::CandidatePolicy::HostAndServerReflexive;
            } else if (equals(value, "all")) {
                network.candidate_policy = transport::CandidatePolicy::All;
            } else {
                set_error(error, error_capacity, "politica ICE invalida: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
        } else if (equals(argument, "--port-min")) {
            if (!parse_u32(value, number) || number > 0xFFFFu) {
                set_error(error, error_capacity, "porta invalida: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            network.local_port_min = static_cast<std::uint16_t>(number);
        } else if (equals(argument, "--port-max")) {
            if (!parse_u32(value, number) || number > 0xFFFFu) {
                set_error(error, error_capacity, "porta invalida: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            network.local_port_max = static_cast<std::uint16_t>(number);
        } else if (equals(argument, "--signal-in")) {
            if (!copy_text(signaling.in_path, kPathCapacity, value)) {
                set_error(error, error_capacity, "caminho longo demais: %s", value);
                return fail(Status::OutOfRange, "parse_command_line");
            }
        } else if (equals(argument, "--signal-out")) {
            if (!copy_text(signaling.out_path, kPathCapacity, value)) {
                set_error(error, error_capacity, "caminho longo demais: %s", value);
                return fail(Status::OutOfRange, "parse_command_line");
            }
        } else if (equals(argument, "--width")) {
            if (!parse_u32(value, number) || number == 0) {
                set_error(error, error_capacity, "largura invalida: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            out.receiver.renderer.width = number;
        } else if (equals(argument, "--height")) {
            if (!parse_u32(value, number) || number == 0) {
                set_error(error, error_capacity, "altura invalida: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            out.receiver.renderer.height = number;
        } else if (equals(argument, "--jitter-ms")) {
            if (!parse_u32(value, number)) {
                set_error(error, error_capacity, "atraso invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            out.receiver.jitter.initial_delay_ns = number * kNanosecondsPerMillisecond;
        } else if (equals(argument, "--max-jitter-ms")) {
            if (!parse_u32(value, number)) {
                set_error(error, error_capacity, "atraso invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
            out.receiver.jitter.max_delay_ns = number * kNanosecondsPerMillisecond;
        } else if (equals(argument, "--log")) {
            if (equals(value, "trace")) {
                out.log_level = LogLevel::Trace;
            } else if (equals(value, "debug")) {
                out.log_level = LogLevel::Debug;
            } else if (equals(value, "info")) {
                out.log_level = LogLevel::Info;
            } else if (equals(value, "warn")) {
                out.log_level = LogLevel::Warn;
            } else if (equals(value, "error")) {
                out.log_level = LogLevel::Error;
            } else {
                set_error(error, error_capacity, "nivel de log invalido: %s", value);
                return fail(Status::InvalidArgument, "parse_command_line");
            }
        } else {
            set_error(error, error_capacity, "argumento desconhecido: %s", argument);
            return fail(Status::InvalidArgument, "parse_command_line");
        }
    }

    if (network.min_bitrate_bps > network.start_bitrate_bps ||
        network.start_bitrate_bps > network.max_bitrate_bps) {
        set_error(error, error_capacity, "faixa de bitrate incoerente%s", "");
        return fail(Status::InvalidArgument, "parse_command_line");
    }
    if (network.local_port_min != 0 && network.local_port_max < network.local_port_min) {
        set_error(error, error_capacity, "faixa de portas incoerente%s", "");
        return fail(Status::InvalidArgument, "parse_command_line");
    }
    if (out.receiver.jitter.max_delay_ns < out.receiver.jitter.min_delay_ns) {
        set_error(error, error_capacity, "faixa de atraso incoerente%s", "");
        return fail(Status::InvalidArgument, "parse_command_line");
    }

    return ok();
}

}  // namespace tl::app
