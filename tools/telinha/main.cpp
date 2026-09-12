#include <csignal>
#include <cstdio>

#include "telinha/app/app_options.hpp"
#include "telinha/app/receiver_session.hpp"
#include "telinha/app/wizard.hpp"
#include "telinha/capture/capture_source.hpp"
#include "telinha/core/log.hpp"
#include "telinha/receive/backends.hpp"
#include "telinha/transport/media_transport.hpp"

#if TELINHA_APP_HAS_SENDER
#include "telinha/app/sender_session.hpp"
#endif

#if TL_PLATFORM_WINDOWS
#include <windows.h>

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
#endif

const char* availability(bool available) noexcept
{
    return available ? "sim" : "nao";
}

int run_list(const tl::app::AppOptions& options) noexcept
{
    static tl::capture::CaptureTargetInfo targets[64];

    const tl::capture::CaptureTargetKind kinds[2] = {tl::capture::CaptureTargetKind::Monitor,
                                                     tl::capture::CaptureTargetKind::Window};

    for (const tl::capture::CaptureTargetKind kind : kinds) {
        if (options.list_kind != tl::capture::CaptureTargetKind::None &&
            options.list_kind != kind) {
            continue;
        }

        std::uint32_t written = 0;
        std::uint32_t available = 0;
        const tl::Outcome listed = tl::capture::enumerate_targets(
            kind, tl::Span<tl::capture::CaptureTargetInfo>(targets, 64), written, available);

        std::printf("\n%s (%u):\n",
                    kind == tl::capture::CaptureTargetKind::Monitor ? "Monitores" : "Janelas",
                    available);

        if (!listed.ok()) {
            std::printf("  indisponivel: %s\n", tl::to_string(listed.status()));
            continue;
        }

        for (std::uint32_t index = 0; index < written; ++index) {
            const tl::capture::CaptureTargetInfo& info = targets[index];
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

    std::printf("\nUse o indice: telinha send --monitor N  ou  telinha send --window N\n");
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
        std::fprintf(stderr, "telinha: nao foi possivel preparar o receptor: %s (%s)\n",
                     tl::to_string(initialized.status()), initialized.error().context);
        return 2;
    }

    const tl::Outcome result = session.run();
    tl::Logger::instance().drain_to_stderr();

    if (!result.ok()) {
        std::fprintf(stderr, "telinha: receptor terminou com erro: %s (%s)\n",
                     tl::to_string(result.status()), result.error().context);
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

    tl::Logger::instance().set_min_level(options.log_level);

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
    if (launched_from_explorer()) {
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
