#include "telinha/transport/session_blob.hpp"

#include <cstring>

namespace tl::transport {
namespace {

constexpr char kMagic[4] = {'T', 'L', 'S', '1'};
constexpr std::uint8_t kVersion = 1;
constexpr std::size_t kHeaderBytes = kSessionBlobHeaderBytes;
static_assert(kHeaderBytes == sizeof(kMagic) + 1 + 1 + 2 + 4);
constexpr std::size_t kChecksumBytes = 4;

constexpr char kAlphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

[[nodiscard]] std::uint8_t decode_symbol(char symbol) noexcept
{
    if (symbol >= 'A' && symbol <= 'Z') {
        return static_cast<std::uint8_t>(symbol - 'A');
    }
    if (symbol >= 'a' && symbol <= 'z') {
        return static_cast<std::uint8_t>(symbol - 'a' + 26);
    }
    if (symbol >= '0' && symbol <= '9') {
        return static_cast<std::uint8_t>(symbol - '0' + 52);
    }
    if (symbol == '-') {
        return 62;
    }
    if (symbol == '_') {
        return 63;
    }
    return 0xFF;
}

[[nodiscard]] bool is_skippable(char symbol) noexcept
{
    return symbol == ' ' || symbol == '\t' || symbol == '\r' || symbol == '\n' || symbol == '=';
}

[[nodiscard]] std::uint32_t crc32(const std::uint8_t* data, std::size_t size) noexcept
{
    std::uint32_t remainder = 0xFFFFFFFFu;
    for (std::size_t index = 0; index < size; ++index) {
        remainder ^= data[index];
        for (int bit = 0; bit < 8; ++bit) {
            const std::uint32_t mask =
                static_cast<std::uint32_t>(-static_cast<std::int32_t>(remainder & 1u));
            remainder = (remainder >> 1) ^ (0xEDB88320u & mask);
        }
    }
    return ~remainder;
}

void write_u16(std::uint8_t* out, std::uint16_t value) noexcept
{
    out[0] = static_cast<std::uint8_t>(value & 0xFFu);
    out[1] = static_cast<std::uint8_t>((value >> 8) & 0xFFu);
}

void write_u32(std::uint8_t* out, std::uint32_t value) noexcept
{
    out[0] = static_cast<std::uint8_t>(value & 0xFFu);
    out[1] = static_cast<std::uint8_t>((value >> 8) & 0xFFu);
    out[2] = static_cast<std::uint8_t>((value >> 16) & 0xFFu);
    out[3] = static_cast<std::uint8_t>((value >> 24) & 0xFFu);
}

[[nodiscard]] std::uint16_t read_u16(const std::uint8_t* in) noexcept
{
    return static_cast<std::uint16_t>(in[0] | (static_cast<std::uint16_t>(in[1]) << 8));
}

[[nodiscard]] std::uint32_t read_u32(const std::uint8_t* in) noexcept
{
    return static_cast<std::uint32_t>(in[0]) | (static_cast<std::uint32_t>(in[1]) << 8) |
           (static_cast<std::uint32_t>(in[2]) << 16) | (static_cast<std::uint32_t>(in[3]) << 24);
}

[[nodiscard]] std::size_t binary_size(const SessionBlob& blob) noexcept
{
    std::size_t size = kHeaderBytes + blob.description().size();
    for (std::size_t index = 0; index < blob.candidate_count(); ++index) {
        size += 2 + blob.candidate(index).size();
    }
    return size + kChecksumBytes;
}

}  // namespace

void SessionBlob::clear() noexcept
{
    description_length_ = 0;
    candidate_count_ = 0;
}

Outcome SessionBlob::set_description(Span<const char> description) noexcept
{
    if (description.empty()) {
        return fail(Status::InvalidArgument, "set_description: empty description");
    }
    if (description.size() > kMaxSessionDescriptionBytes) {
        return fail(Status::OutOfRange, "set_description: the description does not fit");
    }
    std::memcpy(description_, description.data(), description.size());
    description_length_ = static_cast<std::uint32_t>(description.size());
    return ok();
}

Outcome SessionBlob::add_candidate(Span<const char> candidate) noexcept
{
    if (candidate.empty()) {
        return fail(Status::InvalidArgument, "add_candidate: empty candidate");
    }
    if (candidate.size() > kMaxSessionCandidateBytes) {
        return fail(Status::OutOfRange, "add_candidate: the candidate does not fit");
    }
    if (candidate_count_ >= kMaxSessionCandidates) {
        return fail(Status::Full, "add_candidate: the blob already holds every candidate slot");
    }
    std::memcpy(candidates_[candidate_count_], candidate.data(), candidate.size());
    candidate_lengths_[candidate_count_] = static_cast<std::uint16_t>(candidate.size());
    ++candidate_count_;
    return ok();
}

Span<const char> SessionBlob::candidate(std::size_t index) const noexcept
{
    if (index >= candidate_count_) {
        return {};
    }
    return Span<const char>(candidates_[index], candidate_lengths_[index]);
}

std::size_t SessionBlob::encoded_bounds() const noexcept
{
    const std::size_t binary = binary_size(*this);
    return (binary + 2) / 3 * 4;
}

Result<std::size_t> encode_session_blob(const SessionBlob& blob, Span<char> out) noexcept
{
    if (blob.description_length_ == 0) {
        return Error{Status::InvalidArgument, "encode_session_blob: the blob has no description"};
    }

    if (binary_size(blob) > kSessionBlobScratchBytes) {
        return Error{Status::OutOfRange, "encode_session_blob: the blob is too large"};
    }

    std::uint8_t* const scratch = blob.scratch_;

    std::size_t cursor = 0;
    std::memcpy(scratch + cursor, kMagic, sizeof(kMagic));
    cursor += sizeof(kMagic);
    scratch[cursor++] = kVersion;
    scratch[cursor++] = static_cast<std::uint8_t>(blob.role_);
    write_u16(scratch + cursor, blob.candidate_count_);
    cursor += 2;
    write_u32(scratch + cursor, blob.description_length_);
    cursor += 4;
    std::memcpy(scratch + cursor, blob.description_, blob.description_length_);
    cursor += blob.description_length_;

    for (std::size_t index = 0; index < blob.candidate_count_; ++index) {
        const std::uint16_t length = blob.candidate_lengths_[index];
        write_u16(scratch + cursor, length);
        cursor += 2;
        std::memcpy(scratch + cursor, blob.candidates_[index], length);
        cursor += length;
    }

    write_u32(scratch + cursor, crc32(scratch, cursor));
    cursor += kChecksumBytes;

    const std::size_t needed = (cursor + 2) / 3 * 4;
    if (out.size() < needed) {
        return Error{Status::OutOfRange, "encode_session_blob: the output buffer is too small"};
    }

    std::size_t written = 0;
    for (std::size_t index = 0; index < cursor; index += 3) {
        const std::size_t remaining = cursor - index;
        const std::uint32_t triple =
            (static_cast<std::uint32_t>(scratch[index]) << 16) |
            (remaining > 1 ? static_cast<std::uint32_t>(scratch[index + 1]) << 8 : 0u) |
            (remaining > 2 ? static_cast<std::uint32_t>(scratch[index + 2]) : 0u);

        out[written++] = kAlphabet[(triple >> 18) & 0x3Fu];
        out[written++] = kAlphabet[(triple >> 12) & 0x3Fu];
        if (remaining > 1) {
            out[written++] = kAlphabet[(triple >> 6) & 0x3Fu];
        }
        if (remaining > 2) {
            out[written++] = kAlphabet[triple & 0x3Fu];
        }
    }

    return written;
}

Outcome decode_session_blob(Span<const char> text, SessionBlob& out) noexcept
{
    std::uint8_t* const scratch = out.scratch_;

    std::size_t produced = 0;
    std::uint32_t accumulator = 0;
    int accumulated_bits = 0;

    for (std::size_t index = 0; index < text.size(); ++index) {
        const char symbol = text[index];
        if (is_skippable(symbol)) {
            continue;
        }
        const std::uint8_t value = decode_symbol(symbol);
        if (value == 0xFF) {
            return fail(Status::InvalidArgument, "decode_session_blob: unexpected character");
        }
        accumulator = (accumulator << 6) | value;
        accumulated_bits += 6;
        if (accumulated_bits >= 8) {
            accumulated_bits -= 8;
            if (produced >= kSessionBlobScratchBytes) {
                return fail(Status::OutOfRange, "decode_session_blob: the text is too long");
            }
            scratch[produced++] =
                static_cast<std::uint8_t>((accumulator >> accumulated_bits) & 0xFFu);
        }
    }

    if (produced < kHeaderBytes + kChecksumBytes) {
        return fail(Status::InvalidArgument, "decode_session_blob: the text was truncated");
    }
    if (std::memcmp(scratch, kMagic, sizeof(kMagic)) != 0) {
        return fail(Status::InvalidArgument, "decode_session_blob: this is not a session code");
    }
    if (scratch[sizeof(kMagic)] != kVersion) {
        return fail(Status::NotSupported,
                    "decode_session_blob: the session code is another version");
    }

    const std::size_t payload = produced - kChecksumBytes;
    if (crc32(scratch, payload) != read_u32(scratch + payload)) {
        return fail(Status::InvalidArgument,
                    "decode_session_blob: the session code was corrupted in transit");
    }

    std::size_t cursor = sizeof(kMagic) + 1;
    const std::uint8_t role = scratch[cursor++];
    if (role > static_cast<std::uint8_t>(TransportRole::Receiver)) {
        return fail(Status::InvalidArgument, "decode_session_blob: unknown role");
    }
    const std::uint16_t candidate_count = read_u16(scratch + cursor);
    cursor += 2;
    const std::uint32_t description_length = read_u32(scratch + cursor);
    cursor += 4;

    if (candidate_count > kMaxSessionCandidates ||
        description_length > kMaxSessionDescriptionBytes || cursor + description_length > payload) {
        return fail(Status::InvalidArgument, "decode_session_blob: the session code is malformed");
    }

    out.clear();
    out.set_role(static_cast<TransportRole>(role));
    TL_TRY(out.set_description(
        Span<const char>(reinterpret_cast<const char*>(scratch + cursor), description_length)));
    cursor += description_length;

    for (std::uint16_t index = 0; index < candidate_count; ++index) {
        if (cursor + 2 > payload) {
            return fail(Status::InvalidArgument,
                        "decode_session_blob: a candidate length is missing");
        }
        const std::uint16_t length = read_u16(scratch + cursor);
        cursor += 2;
        if (length > kMaxSessionCandidateBytes || cursor + length > payload) {
            return fail(Status::InvalidArgument, "decode_session_blob: a candidate is truncated");
        }
        TL_TRY(out.add_candidate(
            Span<const char>(reinterpret_cast<const char*>(scratch + cursor), length)));
        cursor += length;
    }

    return ok();
}

}  // namespace tl::transport
