#pragma once

#include <cstddef>
#include <cstdint>

#include "telinha/core/result.hpp"
#include "telinha/core/span.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::transport {

inline constexpr std::size_t kMaxSessionDescriptionBytes = 16u * 1024u;
inline constexpr std::size_t kMaxSessionCandidates = 64;
inline constexpr std::size_t kMaxSessionCandidateBytes = 448;
inline constexpr std::size_t kSessionBlobHeaderBytes = 12;
inline constexpr std::size_t kSessionBlobScratchBytes =
    kSessionBlobHeaderBytes + kMaxSessionDescriptionBytes +
    kMaxSessionCandidates * (kMaxSessionCandidateBytes + 2) + 4;

class SessionBlob {
public:
    [[nodiscard]] Outcome set_description(Span<const char> description) noexcept;
    [[nodiscard]] Outcome add_candidate(Span<const char> candidate) noexcept;
    void clear() noexcept;

    void set_role(TransportRole role) noexcept { role_ = role; }
    [[nodiscard]] TransportRole role() const noexcept { return role_; }

    [[nodiscard]] Span<const char> description() const noexcept
    {
        return Span<const char>(description_, description_length_);
    }

    [[nodiscard]] std::size_t candidate_count() const noexcept { return candidate_count_; }
    [[nodiscard]] Span<const char> candidate(std::size_t index) const noexcept;

    [[nodiscard]] std::size_t encoded_bounds() const noexcept;

private:
    friend Result<std::size_t> encode_session_blob(const SessionBlob&, Span<char>) noexcept;
    friend Outcome decode_session_blob(Span<const char>, SessionBlob&) noexcept;

    mutable std::uint8_t scratch_[kSessionBlobScratchBytes] = {};
    char description_[kMaxSessionDescriptionBytes] = {};
    char candidates_[kMaxSessionCandidates][kMaxSessionCandidateBytes] = {};
    std::uint16_t candidate_lengths_[kMaxSessionCandidates] = {};
    std::uint32_t description_length_ = 0;
    std::uint16_t candidate_count_ = 0;
    TransportRole role_ = TransportRole::Sender;
};

[[nodiscard]] Result<std::size_t> encode_session_blob(const SessionBlob& blob,
                                                      Span<char> out) noexcept;

[[nodiscard]] Outcome decode_session_blob(Span<const char> text, SessionBlob& out) noexcept;

}  // namespace tl::transport
