#include "telinha/app/signaling.hpp"

#include <cstdio>
#include <cstring>

namespace tl::app {
namespace {

constexpr char kPrefix[] = "TELINHA1.";
constexpr std::size_t kPrefixLength = sizeof(kPrefix) - 1;
constexpr char kAlphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
constexpr char kDescriptionMarker[] = "\x01";
constexpr char kCandidateMarker[] = "\x02";

std::uint32_t clamp_length(Span<const char> text, std::uint32_t capacity) noexcept
{
    std::uint32_t length = static_cast<std::uint32_t>(text.size());
    while (length > 0 && text[length - 1] == '\0') {
        --length;
    }
    return length > capacity ? capacity : length;
}

int decode_symbol(char symbol) noexcept
{
    if (symbol >= 'A' && symbol <= 'Z') {
        return symbol - 'A';
    }
    if (symbol >= 'a' && symbol <= 'z') {
        return symbol - 'a' + 26;
    }
    if (symbol >= '0' && symbol <= '9') {
        return symbol - '0' + 52;
    }
    if (symbol == '+') {
        return 62;
    }
    if (symbol == '/') {
        return 63;
    }
    return -1;
}

std::size_t base64_encode(const char* data, std::size_t length, char* out,
                          std::size_t capacity) noexcept
{
    const std::size_t required = ((length + 2) / 3) * 4;
    if (required + 1 > capacity) {
        return 0;
    }

    std::size_t written = 0;
    std::size_t position = 0;
    while (position + 2 < length) {
        const std::uint32_t triple = (static_cast<std::uint8_t>(data[position]) << 16) |
                                     (static_cast<std::uint8_t>(data[position + 1]) << 8) |
                                     static_cast<std::uint8_t>(data[position + 2]);
        out[written++] = kAlphabet[(triple >> 18) & 0x3F];
        out[written++] = kAlphabet[(triple >> 12) & 0x3F];
        out[written++] = kAlphabet[(triple >> 6) & 0x3F];
        out[written++] = kAlphabet[triple & 0x3F];
        position += 3;
    }

    const std::size_t remaining = length - position;
    if (remaining == 1) {
        const std::uint32_t triple =
            static_cast<std::uint32_t>(static_cast<std::uint8_t>(data[position]) << 16);
        out[written++] = kAlphabet[(triple >> 18) & 0x3F];
        out[written++] = kAlphabet[(triple >> 12) & 0x3F];
        out[written++] = '=';
        out[written++] = '=';
    } else if (remaining == 2) {
        const std::uint32_t triple =
            static_cast<std::uint32_t>(static_cast<std::uint8_t>(data[position]) << 16) |
            static_cast<std::uint32_t>(static_cast<std::uint8_t>(data[position + 1]) << 8);
        out[written++] = kAlphabet[(triple >> 18) & 0x3F];
        out[written++] = kAlphabet[(triple >> 12) & 0x3F];
        out[written++] = kAlphabet[(triple >> 6) & 0x3F];
        out[written++] = '=';
    }

    out[written] = '\0';
    return written;
}

std::size_t base64_decode(const char* data, std::size_t length, char* out,
                          std::size_t capacity) noexcept
{
    std::uint32_t accumulator = 0;
    std::uint32_t bits = 0;
    std::size_t written = 0;

    for (std::size_t index = 0; index < length; ++index) {
        const char symbol = data[index];
        if (symbol == '=' || symbol == '\n' || symbol == '\r' || symbol == ' ' || symbol == '\t') {
            continue;
        }
        const int value = decode_symbol(symbol);
        if (value < 0) {
            return 0;
        }
        accumulator = (accumulator << 6) | static_cast<std::uint32_t>(value);
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            if (written + 1 >= capacity) {
                return 0;
            }
            out[written++] = static_cast<char>((accumulator >> bits) & 0xFFu);
        }
    }

    out[written] = '\0';
    return written;
}

}  // namespace

void SignalingCollector::on_description(Span<const char> text) noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    const std::uint32_t length = clamp_length(text, kDescriptionCapacity - 1);
    if (length == 0) {
        return;
    }
    std::memcpy(payload_.description, text.data(), length);
    payload_.description[length] = '\0';
    payload_.description_length = length;
}

void SignalingCollector::on_candidate(Span<const char> text) noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    const std::uint32_t length = clamp_length(text, kCandidateTextCapacity - 1);
    if (length == 0) {
        payload_.gathering_complete = true;
        return;
    }
    if (payload_.candidate_count >= kMaxCandidates) {
        ++payload_.candidates_dropped;
        return;
    }
    const std::uint32_t slot = payload_.candidate_count;
    std::memcpy(payload_.candidates[slot], text.data(), length);
    payload_.candidates[slot][length] = '\0';
    payload_.candidate_length[slot] = length;
    ++payload_.candidate_count;
}

bool SignalingCollector::has_description() const noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    return payload_.description_length != 0;
}

bool SignalingCollector::gathering_complete() const noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    return payload_.gathering_complete && payload_.description_length != 0;
}

void SignalingCollector::snapshot(SignalingPayload& out) const noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    out = payload_;
}

void SignalingCollector::reset() noexcept
{
    const std::lock_guard<std::mutex> guard(mutex_);
    payload_ = SignalingPayload{};
}

Outcome encode_signaling_token(const SignalingPayload& payload, char* out, std::size_t capacity,
                               std::size_t& length) noexcept
{
    length = 0;
    if (payload.description_length == 0) {
        return fail(Status::InvalidArgument, "encode_signaling_token: sem descricao");
    }

    static thread_local char
        plain[kDescriptionCapacity + kMaxCandidates * kCandidateTextCapacity + kMaxCandidates + 8];
    std::size_t written = 0;

    plain[written++] = kDescriptionMarker[0];
    std::memcpy(plain + written, payload.description, payload.description_length);
    written += payload.description_length;

    for (std::uint32_t index = 0; index < payload.candidate_count; ++index) {
        plain[written++] = kCandidateMarker[0];
        std::memcpy(plain + written, payload.candidates[index], payload.candidate_length[index]);
        written += payload.candidate_length[index];
    }

    if (capacity <= kPrefixLength) {
        return fail(Status::OutOfRange, "encode_signaling_token");
    }
    std::memcpy(out, kPrefix, kPrefixLength);

    const std::size_t encoded =
        base64_encode(plain, written, out + kPrefixLength, capacity - kPrefixLength);
    if (encoded == 0) {
        return fail(Status::OutOfRange, "encode_signaling_token");
    }

    length = kPrefixLength + encoded;
    return ok();
}

Outcome decode_signaling_token(Span<const char> token, SignalingPayload& out) noexcept
{
    out = SignalingPayload{};

    const char* data = token.data();
    std::size_t length = token.size();
    while (length > 0 &&
           (data[0] == ' ' || data[0] == '\n' || data[0] == '\r' || data[0] == '\t')) {
        ++data;
        --length;
    }
    while (length > 0 && (data[length - 1] == ' ' || data[length - 1] == '\n' ||
                          data[length - 1] == '\r' || data[length - 1] == '\t')) {
        --length;
    }

    if (length <= kPrefixLength || std::memcmp(data, kPrefix, kPrefixLength) != 0) {
        return fail(Status::InvalidArgument, "decode_signaling_token: prefixo");
    }

    static thread_local char
        plain[kDescriptionCapacity + kMaxCandidates * kCandidateTextCapacity + kMaxCandidates + 8];
    const std::size_t decoded =
        base64_decode(data + kPrefixLength, length - kPrefixLength, plain, sizeof(plain));
    if (decoded == 0) {
        return fail(Status::InvalidArgument, "decode_signaling_token: base64");
    }

    std::size_t cursor = 0;
    while (cursor < decoded) {
        const char marker = plain[cursor];
        ++cursor;
        std::size_t end = cursor;
        while (end < decoded && plain[end] != kDescriptionMarker[0] &&
               plain[end] != kCandidateMarker[0]) {
            ++end;
        }
        const std::size_t field = end - cursor;

        if (marker == kDescriptionMarker[0]) {
            if (field >= kDescriptionCapacity) {
                return fail(Status::OutOfRange, "decode_signaling_token: descricao");
            }
            std::memcpy(out.description, plain + cursor, field);
            out.description[field] = '\0';
            out.description_length = static_cast<std::uint32_t>(field);
        } else if (marker == kCandidateMarker[0]) {
            if (field >= kCandidateTextCapacity) {
                return fail(Status::OutOfRange, "decode_signaling_token: candidato");
            }
            if (out.candidate_count >= kMaxCandidates) {
                ++out.candidates_dropped;
            } else {
                const std::uint32_t slot = out.candidate_count;
                std::memcpy(out.candidates[slot], plain + cursor, field);
                out.candidates[slot][field] = '\0';
                out.candidate_length[slot] = static_cast<std::uint32_t>(field);
                ++out.candidate_count;
            }
        } else {
            return fail(Status::InvalidArgument, "decode_signaling_token: marcador");
        }

        cursor = end;
    }

    if (out.description_length == 0) {
        return fail(Status::InvalidArgument, "decode_signaling_token: sem descricao");
    }

    out.gathering_complete = true;
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
        return read == 0 ? fail(Status::Empty, "consume_token: vazio") : ok();
    }

    std::printf("Cole aqui o %s e tecle enter duas vezes:\n", label);
    std::fflush(stdout);

    char line[1024];
    while (std::fgets(line, static_cast<int>(sizeof(line)), stdin) != nullptr) {
        std::size_t trimmed = std::strlen(line);
        while (trimmed > 0 && (line[trimmed - 1] == '\n' || line[trimmed - 1] == '\r' ||
                               line[trimmed - 1] == ' ' || line[trimmed - 1] == '\t')) {
            --trimmed;
        }
        if (trimmed == 0) {
            break;
        }
        if (length + trimmed + 1 > capacity) {
            return fail(Status::OutOfRange, "consume_token: token longo demais");
        }
        std::memcpy(out + length, line, trimmed);
        length += trimmed;
    }

    out[length] = '\0';
    return length == 0 ? fail(Status::Empty, "consume_token: vazio") : ok();
}

}  // namespace tl::app
