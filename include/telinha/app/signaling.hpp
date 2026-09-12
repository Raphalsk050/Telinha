#pragma once

#include <cstddef>
#include <cstdint>
#include <mutex>

#include "telinha/app/app_options.hpp"
#include "telinha/core/result.hpp"
#include "telinha/core/span.hpp"

namespace tl::app {

inline constexpr std::uint32_t kDescriptionCapacity = 16u * 1024u;
inline constexpr std::uint32_t kCandidateTextCapacity = 320;
inline constexpr std::uint32_t kMaxCandidates = 48;
inline constexpr std::uint32_t kTokenCapacity = 48u * 1024u;

struct SignalingPayload {
    char description[kDescriptionCapacity] = {};
    std::uint32_t description_length = 0;
    char candidates[kMaxCandidates][kCandidateTextCapacity] = {};
    std::uint32_t candidate_length[kMaxCandidates] = {};
    std::uint32_t candidate_count = 0;
    std::uint32_t candidates_dropped = 0;
    bool gathering_complete = false;
};

class SignalingCollector {
public:
    SignalingCollector() noexcept = default;

    SignalingCollector(const SignalingCollector&) = delete;
    SignalingCollector& operator=(const SignalingCollector&) = delete;

    void on_description(Span<const char> text) noexcept;
    void on_candidate(Span<const char> text) noexcept;

    [[nodiscard]] bool has_description() const noexcept;
    [[nodiscard]] bool gathering_complete() const noexcept;

    void snapshot(SignalingPayload& out) const noexcept;

    void reset() noexcept;

private:
    mutable std::mutex mutex_;
    SignalingPayload payload_;
};

[[nodiscard]] Outcome encode_signaling_token(const SignalingPayload& payload, char* out,
                                             std::size_t capacity, std::size_t& length) noexcept;

[[nodiscard]] Outcome decode_signaling_token(Span<const char> token,
                                             SignalingPayload& out) noexcept;

[[nodiscard]] Outcome publish_token(const SignalingOptions& options, const char* label,
                                    const char* token, std::size_t length) noexcept;

[[nodiscard]] Outcome consume_token(const SignalingOptions& options, const char* label, char* out,
                                    std::size_t capacity, std::size_t& length) noexcept;

}  // namespace tl::app
