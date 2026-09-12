#pragma once

#include <atomic>
#include <cstddef>
#include <new>
#include <type_traits>
#include <utility>

#include "telinha/core/config.hpp"

namespace tl {
template<typename T, std::size_t Capacity>
class SpscRing {
    static_assert(Capacity >= 2, "a ring of one has no steady state");
    static_assert((Capacity & (Capacity - 1)) == 0, "Capacity must be a power of two");
    static_assert(std::is_nothrow_move_assignable_v<T>,
                  "the queue must not be able to fail halfway through a publish");
    static_assert(std::is_nothrow_default_constructible_v<T>,
                  "slots are default constructed once, at ring construction");

public:
    static constexpr std::size_t capacity = Capacity;

    SpscRing() noexcept = default;
    SpscRing(const SpscRing&) = delete;
    SpscRing& operator=(const SpscRing&) = delete;

    [[nodiscard]] bool push(T value) noexcept
    {
        const std::size_t tail = tail_.value.load(std::memory_order_relaxed);
        const std::size_t next = tail + 1;

        if (next - cached_head_ > Capacity) {
            cached_head_ = head_.value.load(std::memory_order_acquire);
            if (next - cached_head_ > Capacity) {
                return false;
            }
        }

        slots_[tail & kMask] = std::move(value);
        tail_.value.store(next, std::memory_order_release);
        return true;
    }

    [[nodiscard]] bool pop(T& out) noexcept
    {
        const std::size_t head = head_.value.load(std::memory_order_relaxed);

        if (head == cached_tail_) {
            cached_tail_ = tail_.value.load(std::memory_order_acquire);
            if (head == cached_tail_) {
                return false;
            }
        }

        out = std::move(slots_[head & kMask]);
        head_.value.store(head + 1, std::memory_order_release);
        return true;
    }

    [[nodiscard]] std::size_t size_approx() const noexcept
    {
        const std::size_t tail = tail_.value.load(std::memory_order_acquire);
        const std::size_t head = head_.value.load(std::memory_order_acquire);
        return tail - head;
    }

    [[nodiscard]] bool empty_approx() const noexcept { return size_approx() == 0; }

private:
    static constexpr std::size_t kMask = Capacity - 1;

    struct alignas(kCacheLineSize) PaddedIndex {
        std::atomic<std::size_t> value{0};
    };

    PaddedIndex tail_;
    alignas(kCacheLineSize) std::size_t cached_head_{0};
    PaddedIndex head_;
    alignas(kCacheLineSize) std::size_t cached_tail_{0};
    alignas(kCacheLineSize) T slots_[Capacity]{};
};
}  // namespace tl
