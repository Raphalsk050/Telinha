#pragma once

#include <cstddef>
#include <cstdint>

#include "telinha/core/config.hpp"

namespace tl {
class LatencyHistogram {
public:
    static constexpr std::uint32_t kSignificantBits = 8;
    static constexpr std::uint32_t kSubBucketCount = 1u << kSignificantBits;
    static constexpr std::uint32_t kSubBucketHalfCount = kSubBucketCount >> 1;
    static constexpr std::uint32_t kMaxExponent = 40;
    static constexpr std::uint32_t kBucketCount = kMaxExponent - kSignificantBits + 1;
    static constexpr std::uint32_t kCounterCount =
        kSubBucketCount + kBucketCount * kSubBucketHalfCount;

    struct Report {
        std::uint64_t count = 0;
        std::uint64_t min_ns = 0;
        std::uint64_t max_ns = 0;
        double mean_ns = 0.0;
        std::uint64_t p50_ns = 0;
        std::uint64_t p95_ns = 0;
        std::uint64_t p99_ns = 0;
        std::uint64_t p999_ns = 0;
        double low_one_percent_ns = 0.0;
    };

    LatencyHistogram() noexcept { reset(); }

    void reset() noexcept;

    void record(std::uint64_t value_ns) noexcept
    {
        const std::uint32_t index = counter_index(value_ns);
        ++counters_[index];
        ++count_;
        sum_ns_ += value_ns;
        if (value_ns < min_ns_) {
            min_ns_ = value_ns;
        }
        if (value_ns > max_ns_) {
            max_ns_ = value_ns;
        }
    }

    [[nodiscard]] std::uint64_t count() const noexcept { return count_; }
    [[nodiscard]] std::uint64_t min_ns() const noexcept { return count_ == 0 ? 0 : min_ns_; }
    [[nodiscard]] std::uint64_t max_ns() const noexcept { return max_ns_; }
    [[nodiscard]] double mean_ns() const noexcept
    {
        return count_ == 0 ? 0.0 : static_cast<double>(sum_ns_) / static_cast<double>(count_);
    }

    [[nodiscard]] std::uint64_t percentile_ns(double percentile) const noexcept;

    [[nodiscard]] double worst_fraction_mean_ns(double fraction) const noexcept;

    [[nodiscard]] Report report() const noexcept;

    [[nodiscard]] static std::uint32_t counter_index(std::uint64_t value_ns) noexcept;
    [[nodiscard]] static std::uint64_t lowest_equivalent(std::uint32_t index) noexcept;
    [[nodiscard]] static std::uint64_t highest_equivalent(std::uint32_t index) noexcept;

private:
    std::uint64_t counters_[kCounterCount]{};
    std::uint64_t count_ = 0;
    std::uint64_t sum_ns_ = 0;
    std::uint64_t min_ns_ = 0;
    std::uint64_t max_ns_ = 0;
};
}  // namespace tl
