#include "telinha/core/latency_histogram.hpp"

#include <algorithm>
#include <bit>
#include <cmath>
#include <cstring>
#include <limits>

namespace tl {
void LatencyHistogram::reset() noexcept
{
    std::memset(counters_, 0, sizeof(counters_));
    count_ = 0;
    sum_ns_ = 0;
    min_ns_ = std::numeric_limits<std::uint64_t>::max();
    max_ns_ = 0;
}

std::uint32_t LatencyHistogram::counter_index(std::uint64_t value_ns) noexcept
{
    if (value_ns < kSubBucketCount) {
        return static_cast<std::uint32_t>(value_ns);
    }

    const auto exponent = static_cast<std::uint32_t>(63 - std::countl_zero(value_ns));

    std::uint32_t bucket = exponent - (kSignificantBits - 1);
    if (bucket > kBucketCount) {
        bucket = kBucketCount;
    }

    const std::uint64_t scaled = value_ns >> bucket;
    std::uint32_t sub_index = static_cast<std::uint32_t>(scaled - kSubBucketHalfCount);
    if (sub_index >= kSubBucketHalfCount) {
        sub_index = kSubBucketHalfCount - 1;
    }

    return kSubBucketCount + (bucket - 1) * kSubBucketHalfCount + sub_index;
}

std::uint64_t LatencyHistogram::lowest_equivalent(std::uint32_t index) noexcept
{
    if (index < kSubBucketCount) {
        return index;
    }
    const std::uint32_t offset = index - kSubBucketCount;
    const std::uint32_t bucket = offset / kSubBucketHalfCount + 1;
    const std::uint64_t sub_index = offset % kSubBucketHalfCount + kSubBucketHalfCount;
    return sub_index << bucket;
}

std::uint64_t LatencyHistogram::highest_equivalent(std::uint32_t index) noexcept
{
    if (index < kSubBucketCount) {
        return index;
    }
    const std::uint32_t offset = index - kSubBucketCount;
    const std::uint32_t bucket = offset / kSubBucketHalfCount + 1;
    return lowest_equivalent(index) + (std::uint64_t{1} << bucket) - 1;
}

std::uint64_t LatencyHistogram::percentile_ns(double percentile) const noexcept
{
    if (count_ == 0) {
        return 0;
    }

    const double clamped = std::clamp(percentile, 0.0, 100.0);
    const double exact = clamped / 100.0 * static_cast<double>(count_);
    std::uint64_t rank = static_cast<std::uint64_t>(std::ceil(exact));
    if (rank == 0) {
        rank = 1;
    }
    if (rank > count_) {
        rank = count_;
    }

    std::uint64_t seen = 0;
    for (std::uint32_t index = 0; index < kCounterCount; ++index) {
        seen += counters_[index];
        if (seen >= rank) {
            return highest_equivalent(index);
        }
    }
    return max_ns_;
}

double LatencyHistogram::worst_fraction_mean_ns(double fraction) const noexcept
{
    if (count_ == 0) {
        return 0.0;
    }

    const double clamped = std::clamp(fraction, 0.0, 1.0);
    std::uint64_t wanted =
        static_cast<std::uint64_t>(std::ceil(clamped * static_cast<double>(count_)));
    if (wanted == 0) {
        wanted = 1;
    }
    if (wanted > count_) {
        wanted = count_;
    }

    std::uint64_t taken = 0;
    double accumulated = 0.0;
    for (std::uint32_t offset = 0; offset < kCounterCount; ++offset) {
        const std::uint32_t index = kCounterCount - 1 - offset;
        std::uint64_t available = counters_[index];
        if (available == 0) {
            continue;
        }
        const std::uint64_t use = std::min(available, wanted - taken);
        accumulated += static_cast<double>(highest_equivalent(index)) * static_cast<double>(use);
        taken += use;
        if (taken == wanted) {
            break;
        }
    }

    return taken == 0 ? 0.0 : accumulated / static_cast<double>(taken);
}

LatencyHistogram::Report LatencyHistogram::report() const noexcept
{
    Report out;
    out.count = count_;
    if (count_ == 0) {
        return out;
    }
    out.min_ns = min_ns_;
    out.max_ns = max_ns_;
    out.mean_ns = mean_ns();
    out.p50_ns = percentile_ns(50.0);
    out.p95_ns = percentile_ns(95.0);
    out.p99_ns = percentile_ns(99.0);
    out.p999_ns = percentile_ns(99.9);
    out.low_one_percent_ns = worst_fraction_mean_ns(0.01);
    return out;
}
}  // namespace tl
