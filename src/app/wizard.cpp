#include "telinha/app/wizard.hpp"

#include <cstdio>
#include <cstdlib>
#include <cstring>

#include "telinha/app/receiver_session.hpp"
#include "telinha/capture/capture_source.hpp"
#include "telinha/core/log.hpp"

#if TELINHA_APP_HAS_SENDER
#include "telinha/app/sender_session.hpp"
#endif

namespace tl::app {
namespace {

constexpr std::uint32_t kListCapacity = 64;

struct WizardTarget {
    capture::CaptureTargetKind kind = capture::CaptureTargetKind::None;
    std::int32_t index = 0;
};

bool read_line(char* out, std::size_t capacity) noexcept
{
    if (std::fgets(out, static_cast<int>(capacity), stdin) == nullptr) {
        return false;
    }
    std::size_t length = std::strlen(out);
    while (length > 0 && (out[length - 1] == '\n' || out[length - 1] == '\r' ||
                          out[length - 1] == ' ' || out[length - 1] == '\t')) {
        out[--length] = '\0';
    }
    return true;
}

bool read_choice(const char* prompt, std::uint32_t low, std::uint32_t high,
                 std::uint32_t& out) noexcept
{
    char line[64];
    for (int attempt = 0; attempt < 5; ++attempt) {
        std::printf("%s", prompt);
        std::fflush(stdout);

        if (!read_line(line, sizeof(line))) {
            return false;
        }
        if (line[0] == '\0') {
            continue;
        }

        char* end = nullptr;
        const unsigned long value = std::strtoul(line, &end, 10);
        if (end != line && *end == '\0' && value >= low && value <= high) {
            out = static_cast<std::uint32_t>(value);
            return true;
        }
        std::printf("Digite um numero entre %u e %u.\n", low, high);
    }
    return false;
}

std::uint32_t print_targets(capture::CaptureTargetKind kind, const char* heading,
                            std::uint32_t first_label, WizardTarget* map,
                            std::uint32_t map_capacity) noexcept
{
    static capture::CaptureTargetInfo targets[kListCapacity];

    std::uint32_t written = 0;
    std::uint32_t available = 0;
    const Outcome listed = capture::enumerate_targets(
        kind, Span<capture::CaptureTargetInfo>(targets, kListCapacity), written, available);

    if (!listed.ok() || written == 0) {
        return 0;
    }

    std::printf("\n%s\n", heading);
    std::uint32_t added = 0;
    for (std::uint32_t index = 0; index < written && first_label + added <= map_capacity; ++index) {
        const capture::CaptureTargetInfo& info = targets[index];
        const std::uint32_t label = first_label + added;

        std::printf("  %2u) %-44s %ux%u", label, info.name, info.width, info.height);
        if (kind == capture::CaptureTargetKind::Monitor && info.primary) {
            std::printf("  principal");
        }
        std::printf("\n");

        map[label - 1].kind = kind;
        map[label - 1].index = static_cast<std::int32_t>(index);
        ++added;
    }
    return added;
}

int run_share(AppOptions& options) noexcept
{
#if !TELINHA_APP_HAS_SENDER
    (void)options;
    std::printf("\nEste executavel foi compilado sem o lado que transmite.\n");
    return 4;
#else
    static WizardTarget map[kListCapacity];

    std::uint32_t count =
        print_targets(capture::CaptureTargetKind::Monitor, "Telas:", 1, map, kListCapacity);
    count += print_targets(capture::CaptureTargetKind::Window, "Programas abertos:", count + 1, map,
                           kListCapacity);

    if (count == 0) {
        std::printf(
            "\nNao consegui listar nada para compartilhar nesta maquina.\n"
            "Rode telinha probe para ver o que falta.\n");
        return 5;
    }

    std::uint32_t choice = 0;
    if (!read_choice("\nO que voce quer compartilhar? Digite o numero: ", 1, count, choice)) {
        return 6;
    }

    const WizardTarget& chosen = map[choice - 1];
    options.sender.target_kind = chosen.kind;
    options.sender.target_index = chosen.index;

    std::printf(
        "\nE o som?\n"
        "   1) O som do computador inteiro\n"
        "   2) So o som do programa que eu escolhi\n"
        "   3) Sem som, so a imagem\n");

    std::uint32_t audio = 1;
    if (!read_choice("Digite o numero: ", 1, 3, audio)) {
        return 6;
    }

    if (audio == 1) {
        options.sender.audio_scope = AudioScope::System;
    } else if (audio == 2) {
        if (chosen.kind != capture::CaptureTargetKind::Window) {
            std::printf(
                "\nVoce escolheu compartilhar uma tela inteira, entao nao existe um "
                "programa unico de onde tirar o som. Vou mandar o som do computador "
                "inteiro.\n");
            options.sender.audio_scope = AudioScope::System;
        } else {
            options.sender.audio_scope = AudioScope::Process;
        }
    } else {
        options.sender.audio_scope = AudioScope::None;
    }

    options.sender.signaling.use_clipboard = true;

    std::printf(
        "\nPreparando. Em instantes vou te dar um codigo para mandar para a outra "
        "pessoa.\n");
    std::fflush(stdout);

    return run_sender(options.sender);
#endif
}

int run_watch(AppOptions& options) noexcept
{
    options.receiver.signaling.use_clipboard = true;

    std::printf("\nVoce vai assistir. Peca o codigo para quem vai compartilhar a tela.\n");
    std::fflush(stdout);

    static ReceiverSession session;

    const Outcome initialized = session.initialize(options.receiver);
    if (!initialized.ok()) {
        Logger::instance().drain_to_stderr();
        std::printf(
            "\nNao consegui preparar o receptor nesta maquina: %s\n"
            "Rode telinha probe para ver o que falta.\n",
            to_string(initialized.status()));
        return 2;
    }

    const Outcome result = session.run();
    Logger::instance().drain_to_stderr();

    if (!result.ok()) {
        std::printf("\nA transmissao terminou com erro: %s\n", to_string(result.status()));
        return 3;
    }
    return 0;
}

}  // namespace

int run_wizard(AppOptions& options) noexcept
{
    std::printf(
        "\n  Telinha\n"
        "  compartilhamento de tela direto entre dois computadores\n"
        "\n"
        "   1) Compartilhar a minha tela\n"
        "   2) Assistir a tela de outra pessoa\n");

    std::uint32_t choice = 0;
    if (!read_choice("\nDigite o numero e tecle enter: ", 1, 2, choice)) {
        std::printf("\nNao entendi a escolha, saindo.\n");
        return 1;
    }

    return choice == 1 ? run_share(options) : run_watch(options);
}

}  // namespace tl::app
