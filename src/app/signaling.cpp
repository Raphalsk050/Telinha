#include "telinha/app/signaling.hpp"

#include <cstdio>
#include <cstring>
#include <new>

#include "telinha/app/clipboard.hpp"
#include "telinha/core/log.hpp"

namespace tl::app {
namespace {

constexpr char kTokenPrefix[] = "VExT";
constexpr std::size_t kTokenPrefixLength = sizeof(kTokenPrefix) - 1;
constexpr int kClipboardAttempts = 3;

bool looks_like_token(const char* text, std::size_t length) noexcept
{
    return length > kTokenPrefixLength && std::memcmp(text, kTokenPrefix, kTokenPrefixLength) == 0;
}

std::size_t strip_edges(char* text, std::size_t length) noexcept
{
    std::size_t begin = 0;
    while (begin < length && (text[begin] == ' ' || text[begin] == '\n' || text[begin] == '\r' ||
                              text[begin] == '\t')) {
        ++begin;
    }
    std::size_t end = length;
    while (end > begin && (text[end - 1] == ' ' || text[end - 1] == '\n' || text[end - 1] == '\r' ||
                           text[end - 1] == '\t')) {
        --end;
    }
    const std::size_t useful = end - begin;
    if (begin != 0) {
        std::memmove(text, text + begin, useful);
    }
    text[useful] = '\0';
    return useful;
}

void wait_for_enter() noexcept
{
    int character = std::getchar();
    while (character != '\n' && character != EOF) {
        character = std::getchar();
    }
}

std::size_t trimmed_length(Span<const char> text) noexcept
{
    std::size_t length = text.size();
    while (length > 0 && text[length - 1] == '\0') {
        --length;
    }
    return length;
}

}  // namespace

Outcome SignalingCollector::reserve(transport::TransportRole role)
{
    const std::lock_guard<std::mutex> guard(mutex_);
    blob_.reset(new (std::nothrow) transport::SessionBlob());
    if (!blob_) {
        return fail(Status::OutOfMemory, "SignalingCollector::reserve");
    }
    blob_->clear();
    blob_->set_role(role);
    candidate_count_ = 0;
    candidates_dropped_ = 0;
    has_description_ = false;
    gathering_complete_ = false;
    return ok();
}

void SignalingCollector::on_description(Span<const char> text) noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    if (!blob_) {
        return;
    }

    const std::size_t length = trimmed_length(text);
    if (length == 0) {
        return;
    }

    const Outcome stored = blob_->set_description(Span<const char>(text.data(), length));
    if (stored.ok()) {
        has_description_ = true;
    }
}

void SignalingCollector::on_candidate(Span<const char> text) noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    if (!blob_) {
        return;
    }

    const std::size_t length = trimmed_length(text);
    if (length == 0) {
        gathering_complete_ = true;
        return;
    }

    const Outcome stored = blob_->add_candidate(Span<const char>(text.data(), length));
    if (stored.ok()) {
        ++candidate_count_;
    } else {
        ++candidates_dropped_;
    }
}

bool SignalingCollector::has_description() const noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    return has_description_;
}

bool SignalingCollector::gathering_complete() const noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    return gathering_complete_ && has_description_;
}

std::uint32_t SignalingCollector::candidate_count() const noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    return candidate_count_;
}

std::uint32_t SignalingCollector::candidates_dropped() const noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    return candidates_dropped_;
}

Outcome SignalingCollector::encode(char* out, std::size_t capacity,
                                   std::size_t& length) const noexcept
{
    length = 0;

    const std::lock_guard<std::mutex> guard(mutex_);
    if (!blob_ || !has_description_) {
        return fail(Status::Unavailable, "SignalingCollector::encode: sem descricao");
    }

    Result<std::size_t> encoded = transport::encode_session_blob(*blob_, Span<char>(out, capacity));
    if (!encoded.ok()) {
        return Outcome{encoded.error()};
    }

    length = encoded.value();
    return ok();
}

Outcome apply_remote_blob(transport::MediaTransport& media,
                          const transport::SessionBlob& blob) noexcept
{
    TL_TRY(media.set_remote_description(blob.description()));

    for (std::size_t index = 0; index < blob.candidate_count(); ++index) {
        const Outcome added = media.add_remote_candidate(blob.candidate(index));
        if (!added.ok()) {
            TL_LOG_WARN("sinalizacao: candidato remoto recusado (%s)", to_string(added.status()));
        }
    }
    return ok();
}

Outcome publish_token(const SignalingOptions& options, const char* label, const char* token,
                      std::size_t length) noexcept
{
    if (options.out_path[0] != '\0') {
        std::FILE* file = std::fopen(options.out_path, "wb");
        if (file == nullptr) {
            return fail(Status::PermissionDenied, "publish_token: fopen");
        }
        const std::size_t written = std::fwrite(token, 1, length, file);
        std::fputc('\n', file);
        const int closed = std::fclose(file);
        if (written != length || closed != 0) {
            return fail(Status::PlatformError, "publish_token: fwrite");
        }
        std::printf("%s gravado em %s\n", label, options.out_path);
        std::fflush(stdout);
        return ok();
    }

    if (options.use_clipboard && clipboard_available()) {
        const Outcome copied = clipboard_write(Span<const char>(token, length));
        if (copied.ok()) {
            std::printf(
                "\n%s copiado para a area de transferencia.\n"
                "Cole agora na conversa com a outra pessoa e tecle enter aqui.\n",
                label);
            std::fflush(stdout);
            return ok();
        }
        TL_LOG_WARN("sinalizacao: nao consegui usar a area de transferencia (%s)",
                    to_string(copied.status()));
    }

    std::printf("\n===== %s =====\n%.*s\n===== fim =====\n\n", label, static_cast<int>(length),
                token);
    std::fflush(stdout);
    return ok();
}

Outcome consume_token(const SignalingOptions& options, const char* label, char* out,
                      std::size_t capacity, std::size_t& length) noexcept
{
    length = 0;

    if (options.in_path[0] != '\0') {
        std::FILE* file = std::fopen(options.in_path, "rb");
        if (file == nullptr) {
            return fail(Status::NotFound, "consume_token: fopen");
        }
        const std::size_t read = std::fread(out, 1, capacity - 1, file);
        std::fclose(file);
        out[read] = '\0';
        length = read;
        while (length > 0 && (out[length - 1] == '\n' || out[length - 1] == '\r' ||
                              out[length - 1] == ' ' || out[length - 1] == '\t')) {
            out[--length] = '\0';
        }
        return length == 0 ? fail(Status::Empty, "consume_token: vazio") : ok();
    }

    if (options.use_clipboard && clipboard_available()) {
        for (int attempt = 0; attempt < kClipboardAttempts; ++attempt) {
            std::printf(
                "\nCopie o %s que a outra pessoa te mandou, com ctrl c, e tecle enter "
                "aqui.\n",
                label);
            std::fflush(stdout);
            wait_for_enter();

            const Outcome pasted = clipboard_read(out, capacity, length);
            if (pasted.ok()) {
                length = strip_edges(out, length);
                if (looks_like_token(out, length)) {
                    std::printf("Recebido.\n");
                    std::fflush(stdout);
                    return ok();
                }
                std::printf(
                    "O que esta copiado nao parece um codigo do telinha. Copie o texto "
                    "inteiro que a outra pessoa mandou.\n");
            } else {
                std::printf("Nao achei texto na area de transferencia.\n");
            }
            std::fflush(stdout);
        }
        std::printf("Vamos pelo terminal entao.\n");
        std::fflush(stdout);
        length = 0;
    }

    std::printf("Cole aqui o %s e tecle enter duas vezes:\n", label);
    std::fflush(stdout);

    char line[1024];
    while (std::fgets(line, static_cast<int>(sizeof(line)), stdin) != nullptr) {
        std::size_t useful = std::strlen(line);
        while (useful > 0 && (line[useful - 1] == '\n' || line[useful - 1] == '\r' ||
                              line[useful - 1] == ' ' || line[useful - 1] == '\t')) {
            --useful;
        }
        if (useful == 0) {
            break;
        }
        if (length + useful + 1 > capacity) {
            return fail(Status::OutOfRange, "consume_token: token longo demais");
        }
        std::memcpy(out + length, line, useful);
        length += useful;
    }

    out[length] = '\0';
    return length == 0 ? fail(Status::Empty, "consume_token: vazio") : ok();
}

}  // namespace tl::app
