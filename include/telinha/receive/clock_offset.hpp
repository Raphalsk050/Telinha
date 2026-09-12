#pragma once

#include <cstdint>
#include <memory>

#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/result.hpp"

namespace tl::receive {

struct ClockOffsetConfig {
    Nanoseconds window_ns = 4ull * kNanosecondsPerSecond;
    std::uint32_t capacity = 512;
};

class ClockOffsetEstimator {
public:
    ClockOffsetEstimator() noexcept = default;
    ~ClockOffsetEstimator();

    ClockOffsetEstimator(const ClockOffsetEstimator&) = delete;
    ClockOffsetEstimator& operator=(const ClockOffsetEstimator&) = delete;

    [[nodiscard]] Outcome reserve(const ClockOffsetConfig& config);

    void observe(Nanoseconds remote_ns, Nanoseconds local_ns) noexcept;

    [[nodiscard]] bool ready() const noexcept { return count_ != 0; }

    [[nodiscard]] std::int64_t offset_ns() const noexcept { return offset_ns_; }

    [[nodiscard]] Nanoseconds spread_ns() const noexcept { return spread_ns_; }

    [[nodiscard]] Nanoseconds to_local(Nanoseconds remote_ns) const noexcept;

    void reset() noexcept;

private:
    struct Sample {
        Nanoseconds local_ns = 0;
        std::int64_t delta_ns = 0;
    };

    void recompute() noexcept;

    std::unique_ptr<Sample[]> samples_;
    std::uint32_t capacity_ = 0;
    std::uint32_t head_ = 0;
    std::uint32_t count_ = 0;
    Nanoseconds window_ns_ = 0;
    std::int64_t offset_ns_ = 0;
    Nanoseconds spread_ns_ = 0;
};

}  // namespace tl::receive
