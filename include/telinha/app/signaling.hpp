#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>
#include <mutex>

#include "telinha/app/app_options.hpp"
#include "telinha/core/result.hpp"
#include "telinha/core/span.hpp"
#include "telinha/transport/session_blob.hpp"

namespace tl::app {

inline constexpr std::size_t kTokenCapacity = transport::kSessionBlobScratchBytes * 2;

class SignalingCollector {
public:
    SignalingCollector() noexcept = default;

    SignalingCollector(const SignalingCollector&) = delete;
    SignalingCollector& operator=(const SignalingCollector&) = delete;

    [[nodiscard]] Outcome reserve(transport::TransportRole role);

    void on_description(Span<const char> text) noexcept;
    void on_candidate(Span<const char> text) noexcept;
    void on_gathering_complete() noexcept;

    [[nodiscard]] bool has_description() const noexcept;
    [[nodiscard]] bool gathering_complete() const noexcept;
    [[nodiscard]] std::uint32_t candidate_count() const noexcept;
    [[nodiscard]] std::uint32_t candidates_dropped() const noexcept;

    [[nodiscard]] Outcome encode(char* out, std::size_t capacity,
                                 std::size_t& length) const noexcept;

private:
    mutable std::mutex mutex_;
    std::unique_ptr<transport::SessionBlob> blob_;
    std::uint32_t candidate_count_ = 0;
    std::uint32_t candidates_dropped_ = 0;
    bool has_description_ = false;
    bool gathering_complete_ = false;
};

[[nodiscard]] Outcome publish_token(const SignalingOptions& options, const char* kind,
                                    const char* label, const char* token,
                                    std::size_t length) noexcept;

[[nodiscard]] Outcome consume_token(const SignalingOptions& options, const char* kind,
                                    const char* label, char* out, std::size_t capacity,
                                    std::size_t& length) noexcept;

[[nodiscard]] Outcome receive_remote_blob(const SignalingOptions& options, const char* kind,
                                          const char* label, transport::TransportRole expected,
                                          char* scratch, std::size_t capacity,
                                          transport::SessionBlob& blob) noexcept;

[[nodiscard]] Outcome apply_remote_blob(transport::MediaTransport& media,
                                        const transport::SessionBlob& blob) noexcept;

}  // namespace tl::app
