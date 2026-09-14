#include <csignal>
#include <cstddef>
#include <cstdio>
#include <cstring>
#include <memory>
#include <new>

#include "telinha/app/app_options.hpp"
#include "telinha/app/machine_events.hpp"
#include "telinha/app/receiver_session.hpp"
#include "telinha/app/wizard.hpp"
#include "telinha/capture/capture_source.hpp"
#include "telinha/core/log.hpp"
#include "telinha/receive/backends.hpp"
#include "telinha/transport/media_transport.hpp"

#if TELINHA_APP_HAS_SENDER
#include "telinha/app/sender_session.hpp"
#include "telinha/audio/audio_source.hpp"
#endif

#if TL_PLATFORM_WINDOWS
#include <windows.h>

#include <shellapi.h>
#include <timeapi.h>
#endif

namespace {

std::sig_atomic_t volatile g_interrupted = 0;

extern "C" void handle_interrupt(int) noexcept
{
    g_interrupted = 1;
}

#if TL_PLATFORM_WINDOWS
bool launched_from_explorer() noexcept
{
    DWORD owners[4] = {};
    return GetConsoleProcessList(owners, 4) == 1;
}

void apply_wide_title(tl::app::AppOptions& options, int argc) noexcept
{
    if (options.title_argument <= 0) {
        return;
    }

    int count = 0;
    LPWSTR* arguments = CommandLineToArgvW(GetCommandLineW(), &count);
    if (arguments == nullptr) {
        return;
    }

    if (count == argc && options.title_argument < count) {
        const wchar_t* wide = arguments[options.title_argument];
        const int needed = WideCharToMultiByte(CP_UTF8, 0, wide, -1, nullptr, 0, nullptr, nullptr);
        if (needed > 0) {
            const std::unique_ptr<char[]> utf8(
                new (std::nothrow) char[static_cast<std::size_t>(needed)]);
            if (utf8 && WideCharToMultiByte(CP_UTF8, 0, wide, -1, utf8.get(), needed, nullptr,
                                            nullptr) == needed) {
                tl::app::set_window_title(options.receiver, utf8.get());
            }
        }
    }
    LocalFree(arguments);
}
#endif

const char* availability(bool available) noexcept
{
    return available ? "sim" : "nao";
}

#if TELINHA_APP_HAS_SENDER
constexpr std::uint32_t kAudioInputCapacity = 32;

bool container_known(const std::uint8_t* id) noexcept
{
    // Built-in devices all share the container of the computer itself.
    static constexpr std::uint8_t kThisComputer[16] = {
        0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff};
    bool empty = true;
    for (std::size_t index = 0; index < sizeof(kThisComputer); ++index) {
        empty = empty && id[index] == 0;
    }
    return !empty && std::memcmp(id, kThisComputer, sizeof(kThisComputer)) != 0;
}

char fold(char symbol) noexcept
{
    return symbol >= 'A' && symbol <= 'Z' ? static_cast<char>(symbol - 'A' + 'a') : symbol;
}

bool contains_folded(const char* text, const char* part) noexcept
{
    const std::size_t length = std::strlen(part);
    if (length < 4) {
        return false;
    }
    for (const char* start = text; *start != '\0'; ++start) {
        std::size_t matched = 0;
        while (matched < length && start[matched] != '\0' &&
               fold(start[matched]) == fold(part[matched])) {
            ++matched;
        }
        if (matched == length) {
            return true;
        }
    }
    return false;
}

std::size_t shared_prefix(const char* first, const char* second) noexcept
{
    std::size_t length = 0;
    while (first[length] != '\0' && fold(first[length]) == fold(second[length])) {
        ++length;
    }
    return length;
}

// A capture card shows up as a camera plus a separate sound input of the same physical device.
const tl::audio::AudioEndpointInfo* audio_for_device(const tl::capture::CaptureTargetInfo& device,
                                                     const tl::audio::AudioEndpointInfo* inputs,
                                                     std::uint32_t count) noexcept
{
    if (container_known(device.container_id)) {
        for (std::uint32_t index = 0; index < count; ++index) {
            if (std::memcmp(inputs[index].container_id, device.container_id,
                            sizeof(device.container_id)) == 0) {
                return &inputs[index];
            }
        }
    }
    for (std::uint32_t index = 0; index < count; ++index) {
        if (contains_folded(inputs[index].name, device.name) ||
            shared_prefix(inputs[index].name, device.name) >= 8) {
            return &inputs[index];
        }
    }
    return nullptr;
}

std::uint32_t list_audio_inputs(tl::audio::AudioEndpointInfo* inputs) noexcept
{
    std::uint32_t written = 0;
    std::uint32_t available = 0;
    const tl::Outcome listed = tl::audio::enumerate_capture_endpoints(
        tl::Span<tl::audio::AudioEndpointInfo>(inputs, kAudioInputCapacity), written, available);
    return listed.ok() ? written : 0;
}
#endif

bool lists_devices(const tl::app::AppOptions& options) noexcept
{
    return options.list_kind == tl::capture::CaptureTargetKind::None ||
           options.list_kind == tl::capture::CaptureTargetKind::Device;
}

int run_list_machine(const tl::app::AppOptions& options) noexcept
{
    static tl::capture::CaptureTargetInfo targets[64];

    const tl::capture::CaptureTargetKind kinds[3] = {tl::capture::CaptureTargetKind::Monitor,
                                                     tl::capture::CaptureTargetKind::Window,
                                                     tl::capture::CaptureTargetKind::Device};
    const char* const arrays[3] = {"monitors", "windows", "devices"};
    const char* const errors[3] = {"monitors_error", "windows_error", "devices_error"};

#if TELINHA_APP_HAS_SENDER
    static tl::audio::AudioEndpointInfo inputs[kAudioInputCapacity];
    const std::uint32_t input_count = lists_devices(options) ? list_audio_inputs(inputs) : 0;
#endif

    tl::app::MachineEvent event("targets");
    for (std::uint32_t slot = 0; slot < 3; ++slot) {
        const tl::capture::CaptureTargetKind kind = kinds[slot];
        if (options.list_kind != tl::capture::CaptureTargetKind::None &&
            options.list_kind != kind) {
            continue;
        }

        std::uint32_t written = 0;
        std::uint32_t available = 0;
        const tl::Outcome listed = tl::capture::enumerate_targets(
            kind, tl::Span<tl::capture::CaptureTargetInfo>(targets, 64), written, available);

        event.begin_array(arrays[slot]);
        for (std::uint32_t index = 0; listed.ok() && index < written; ++index) {
            const tl::capture::CaptureTargetInfo& info = targets[index];
            event.begin_object()
                .integer("index", index)
                .integer("handle", info.target.handle)
                .text("name", info.name)
                .integer("width", info.width)
                .integer("height", info.height);
            if (kind == tl::capture::CaptureTargetKind::Monitor) {
                event.number("refresh_hz", static_cast<double>(info.refresh_millihertz) / 1000.0)
                    .flag("primary", info.primary);
            } else if (kind == tl::capture::CaptureTargetKind::Window) {
                event.integer("pid", info.process_id);
            } else {
#if TELINHA_APP_HAS_SENDER
                const tl::audio::AudioEndpointInfo* audio =
                    audio_for_device(info, inputs, input_count);
                event.text("audio_id", audio != nullptr ? audio->id : "")
                    .text("audio_name", audio != nullptr ? audio->name : "");
#endif
            }
            event.end_object();
        }
        event.end_array();

        if (!listed.ok()) {
            event.text(errors[slot], tl::to_string(listed.status()));
        }
    }

#if TELINHA_APP_HAS_SENDER
    if (lists_devices(options)) {
        event.begin_array("audio_inputs");
        for (std::uint32_t index = 0; index < input_count; ++index) {
            event.begin_object()
                .text("id", inputs[index].id)
                .text("name", inputs[index].name)
                .end_object();
        }
        event.end_array();
    }
#endif
    return 0;
}

int run_list(const tl::app::AppOptions& options) noexcept
{
    if (tl::app::machine_events_enabled()) {
        return run_list_machine(options);
    }

    static tl::capture::CaptureTargetInfo targets[64];

    const tl::capture::CaptureTargetKind kinds[3] = {tl::capture::CaptureTargetKind::Monitor,
                                                     tl::capture::CaptureTargetKind::Window,
                                                     tl::capture::CaptureTargetKind::Device};
    const char* const titles[3] = {"Monitores", "Janelas", "Placas de captura"};

    for (std::uint32_t slot = 0; slot < 3; ++slot) {
        const tl::capture::CaptureTargetKind kind = kinds[slot];
        if (options.list_kind != tl::capture::CaptureTargetKind::None &&
            options.list_kind != kind) {
            continue;
        }

        std::uint32_t written = 0;
        std::uint32_t available = 0;
        const tl::Outcome listed = tl::capture::enumerate_targets(
            kind, tl::Span<tl::capture::CaptureTargetInfo>(targets, 64), written, available);

        std::printf("\n%s (%u):\n", titles[slot], available);

        if (!listed.ok()) {
            std::printf("  indisponivel: %s\n", tl::to_string(listed.status()));
            continue;
        }

        for (std::uint32_t index = 0; index < written; ++index) {
            const tl::capture::CaptureTargetInfo& info = targets[index];
            if (kind == tl::capture::CaptureTargetKind::Device) {
                std::printf("  [%u] %s\n", index, info.name);
                continue;
            }
            std::printf("  [%u] %-40s %ux%u", index, info.name, info.width, info.height);
            if (kind == tl::capture::CaptureTargetKind::Monitor) {
                std::printf(" @ %u,%u %.3f Hz%s", static_cast<unsigned>(info.x),
                            static_cast<unsigned>(info.y),
                            static_cast<double>(info.refresh_millihertz) / 1000.0,
                            info.primary ? " principal" : "");
            } else {
                std::printf(" pid %u", info.process_id);
            }
            std::printf("\n");
        }
    }

#if TELINHA_APP_HAS_SENDER
    if (lists_devices(options)) {
        static tl::audio::AudioEndpointInfo inputs[kAudioInputCapacity];
        const std::uint32_t input_count = list_audio_inputs(inputs);
        std::printf("\nEntradas de som (%u):\n", input_count);
        for (std::uint32_t index = 0; index < input_count; ++index) {
            std::printf("  %s  %s\n", inputs[index].name, inputs[index].id);
        }
    }
#endif

    std::printf("\nUse o indice: telinha send --monitor N, --window N ou --device N\n");
    return 0;
}

int run_probe() noexcept
{
    std::printf("Captura de tela\n");
    std::printf("  Desktop Duplication : %s\n",
                availability(tl::capture::backend_available(
                    tl::capture::CaptureBackend::DesktopDuplication)));
    std::printf(
        "  Graphics Capture    : %s\n",
        availability(tl::capture::backend_available(tl::capture::CaptureBackend::GraphicsCapture)));
    std::printf(
        "  Sintetico           : %s\n",
        availability(tl::capture::backend_available(tl::capture::CaptureBackend::Synthetic)));

    std::printf("\nDecodificacao de video\n");
    std::printf("  Media Foundation    : %s\n",
                availability(tl::receive::decoder_backend_available(
                    tl::receive::VideoDecoderBackend::MediaFoundation,
                    tl::transport::WireVideoCodec::H264)));
    std::printf("  Software            : %s\n", availability(tl::receive::decoder_backend_available(
                                                    tl::receive::VideoDecoderBackend::Software,
                                                    tl::transport::WireVideoCodec::H264)));

    std::printf("\nApresentacao\n");
    std::printf("  Direct3D 11         : %s\n",
                availability(tl::receive::renderer_backend_available(
                    tl::receive::RendererBackend::Direct3D11)));
    std::printf("  Sem janela          : %s\n",
                availability(tl::receive::renderer_backend_available(
                    tl::receive::RendererBackend::Headless)));
    std::printf("  WASAPI              : %s\n",
                availability(tl::receive::audio_renderer_backend_available(
                    tl::receive::AudioRendererBackend::Wasapi)));

    std::printf("\nTransporte\n");
    tl::transport::TransportConfig config;
    tl::Result<std::unique_ptr<tl::transport::MediaTransport>> media =
        tl::transport::create_media_transport(config);
    std::printf("  WebRTC              : %s\n",
                media.ok() ? "sim" : tl::to_string(media.error().status));

    std::printf("\nEnvio\n");
#if TELINHA_APP_HAS_SENDER
    std::printf("  Compilado           : sim\n");
#else
    std::printf("  Compilado           : nao, faltam os modulos de codificacao e de audio\n");
#endif

    return 0;
}

int run_receive(const tl::app::AppOptions& options) noexcept
{
    static tl::app::ReceiverSession session;

    const tl::Outcome initialized = session.initialize(options.receiver);
    if (!initialized.ok()) {
        if (tl::app::machine_events_enabled()) {
            tl::app::MachineEvent("error")
                .text("stage", "prepare")
                .text("status", tl::to_string(initialized.status()))
                .text("message", initialized.error().context);
        } else {
            std::fprintf(stderr, "telinha: nao foi possivel preparar o receptor: %s (%s)\n",
                         tl::to_string(initialized.status()), initialized.error().context);
        }
        return 2;
    }

    const tl::Outcome result = session.run();
    tl::Logger::instance().drain_to_stderr();

    if (!result.ok()) {
        if (tl::app::machine_events_enabled()) {
            tl::app::MachineEvent("error")
                .text("stage", "session")
                .text("status", tl::to_string(result.status()))
                .text("message", result.error().context);
        } else {
            std::fprintf(stderr, "telinha: receptor terminou com erro: %s (%s)\n",
                         tl::to_string(result.status()), result.error().context);
        }
        return 3;
    }
    return 0;
}

}  // namespace

int main(int argc, char** argv)
{
    std::signal(SIGINT, handle_interrupt);
#if defined(SIGTERM)
    std::signal(SIGTERM, handle_interrupt);
#endif

#if TL_PLATFORM_WINDOWS
    timeBeginPeriod(1);
#endif

    static tl::app::AppOptions options;
    char error[tl::app::kErrorCapacity] = {};

    const tl::Outcome parsed =
        tl::app::parse_command_line(argc, argv, options, error, static_cast<int>(sizeof(error)));
    if (!parsed.ok()) {
        std::fprintf(stderr, "telinha: %s\n\n", error);
        tl::app::print_usage();
#if TL_PLATFORM_WINDOWS
        timeEndPeriod(1);
#endif
        return 1;
    }

#if TL_PLATFORM_WINDOWS
    apply_wide_title(options, argc);
#endif

    tl::Logger::instance().set_min_level(options.log_level);
    if (options.machine_output) {
        tl::app::enable_machine_events();
    }

    int status = 0;
    switch (options.mode) {
        case tl::app::AppMode::Usage: tl::app::print_usage(); break;
        case tl::app::AppMode::None:
        case tl::app::AppMode::Wizard: status = tl::app::run_wizard(options); break;
        case tl::app::AppMode::List: status = run_list(options); break;
        case tl::app::AppMode::Probe: status = run_probe(); break;
        case tl::app::AppMode::Receive: status = run_receive(options); break;
        case tl::app::AppMode::Send:
#if TELINHA_APP_HAS_SENDER
            status = tl::app::run_sender(options.sender);
#else
            std::fprintf(stderr,
                         "telinha: este executavel foi compilado sem o lado emissor, porque os "
                         "modulos de codificacao e de audio ainda nao estao no build\n");
            status = 4;
#endif
            break;
    }

    tl::Logger::instance().drain_to_stderr();

#if TL_PLATFORM_WINDOWS
    timeEndPeriod(1);
    if (!options.machine_output && launched_from_explorer()) {
        std::printf("\nTecle enter para fechar.\n");
        std::fflush(stdout);
        int character = std::getchar();
        while (character != '\n' && character != EOF) {
            character = std::getchar();
        }
    }
#endif
    return status;
}
