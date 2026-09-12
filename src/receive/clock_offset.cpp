#include "telinha/receive/clock_offset.hpp"

#include <limits>
#include <new>

namespace tl::receive {

ClockOffsetEstimator::~ClockOffsetEstimator() = default;

Outcome ClockOffsetEstimator::reserve(const ClockOffsetConfig& config)
{
    if (config.capacity == 0) {
        return fail(Status::InvalidArgument, "ClockOffsetEstimator::reserve");
    }

    samples_.reset(new (std::nothrow) Sample[config.capacity]);
    if (!samples_) {
        return fail(Status::OutOfMemory, "ClockOffsetEstimator::reserve");
    }

    capacity_ = config.capacity;
    window_ns_ = config.window_ns;
    reset();
    return ok();
}

void ClockOffsetEstimator::reset() noexcept
{
    head_ = 0;
    count_ = 0;
    offset_ns_ = 0;
    spread_ns_ = 0;
}

void ClockOffsetEstimator::observe(Nanoseconds remote_ns, Nanoseconds local_ns) noexcept
{
    if (capacity_ == 0 || remote_ns == 0) {
        return;
    }

    const std::int64_t delta =
        static_cast<std::int64_t>(local_ns) - static_cast<std::int64_t>(remote_ns);

    samples_[head_] = Sample{local_ns, delta};
    head_ = (head_ + 1) % capacity_;
    if (count_ < capacity_) {
        ++count_;
    }

    recompute();
}

void ClockOffsetEstimator::recompute() noexcept
{
    std::int64_t lowest = std::numeric_limits<std::int64_t>::max();
    std::int64_t highest = std::numeric_limits<std::int64_t>::min();

    const std::uint32_t newest = (head_ + capacity_ - 1) % capacity_;
    const Nanoseconds newest_local = samples_[newest].local_ns;

    for (std::uint32_t step = 0; step < count_; ++step) {
        const std::uint32_t index = (head_ + capacity_ - 1 - step) % capacity_;
        const Sample& sample = samples_[index];
        if (window_ns_ != 0 && newest_local - sample.local_ns > window_ns_) {
            break;
        }
        if (sample.delta_ns < lowest) {
            lowest = sample.delta_ns;
        }
        if (sample.delta_ns > highest) {
            highest = sample.delta_ns;
        }
    }

    if (lowest > highest) {
        return;
    }

    offset_ns_ = lowest;
    spread_ns_ = static_cast<Nanoseconds>(highest - lowest);
}

Nanoseconds ClockOffsetEstimator::to_local(Nanoseconds remote_ns) const noexcept
{
    const std::int64_t local = static_cast<std::int64_t>(remote_ns) + offset_ns_;
    return local <= 0 ? 0 : static_cast<Nanoseconds>(local);
}

}  // namespace tl::receive
