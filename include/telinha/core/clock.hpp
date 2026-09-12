#pragma once

#include <cstdint>

#include "telinha/core/config.hpp"

namespace tl {
using Nanoseconds = std::uint64_t;

inline constexpr Nanoseconds kNanosecondsPerMicrosecond = 1000ull;
inline constexpr Nanoseconds kNanosecondsPerMillisecond = 1000ull * 1000ull;
inline constexpr Nanoseconds kNanosecondsPerSecond = 1000ull * 1000ull * 1000ull;
inline constexpr Nanoseconds kNanosecondsPerHundredNanoseconds = 100ull;

[[nodiscard]] Nanoseconds now_ns() noexcept;

[[nodiscard]] std::uint64_t now_ticks() noexcept;
[[nodiscard]] std::uint64_t ticks_per_second() noexcept;
[[nodiscard]] Nanoseconds ticks_to_ns(std::uint64_t ticks) noexcept;

[[nodiscard]] constexpr Nanoseconds hundred_ns_to_ns(std::uint64_t hundred_ns) noexcept
{
    return hundred_ns * kNanosecondsPerHundredNanoseconds;
}

[[nodiscard]] constexpr double ns_to_ms(Nanoseconds ns) noexcept
{
    return static_cast<double>(ns) / 1'000'000.0;
}

class ScopedTimer {
public:
    explicit ScopedTimer(Nanoseconds& destination) noexcept
        : destination_(&destination), start_(now_ns())
    {}

    ScopedTimer(const ScopedTimer&) = delete;
    ScopedTimer& operator=(const ScopedTimer&) = delete;

    ~ScopedTimer() { *destination_ = now_ns() - start_; }

private:
    Nanoseconds* destination_;
    Nanoseconds start_;
};
}  // namespace tl
