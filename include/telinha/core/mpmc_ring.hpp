#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <type_traits>
#include <utility>

#include "telinha/core/config.hpp"

namespace tl {
template<typename T, std::size_t Capacity>
class MpmcRing {
    static_assert(Capacity >= 2, "a ring of one has no steady state");
    static_assert((Capacity & (Capacity - 1)) == 0, "Capacity must be a power of two");
    static_assert(std::is_nothrow_move_assignable_v<T>,
                  "the queue must not be able to fail halfway through a publish");
    static_assert(std::is_nothrow_default_constructible_v<T>,
                  "cells are default constructed once, at ring construction");

public:
    static constexpr std::size_t capacity = Capacity;

    MpmcRing() noexcept
    {
        for (std::size_t i = 0; i < Capacity; ++i) {
            cells_[i].sequence.store(i, std::memory_order_relaxed);
        }
        enqueue_pos_.value.store(0, std::memory_order_relaxed);
        dequeue_pos_.value.store(0, std::memory_order_relaxed);
    }

    MpmcRing(const MpmcRing&) = delete;
    MpmcRing& operator=(const MpmcRing&) = delete;

    [[nodiscard]] bool push(T value) noexcept
    {
        Cell* cell = nullptr;
        std::size_t pos = enqueue_pos_.value.load(std::memory_order_relaxed);

        for (;;) {
            cell = &cells_[pos & kMask];
            const std::size_t sequence = cell->sequence.load(std::memory_order_acquire);
            const auto diff =
                static_cast<std::intptr_t>(sequence) - static_cast<std::intptr_t>(pos);

            if (diff == 0) {
                if (enqueue_pos_.value.compare_exchange_weak(pos, pos + 1,
                                                             std::memory_order_relaxed)) {
                    break;
                }
            } else if (diff < 0) {
                return false;
            } else {
                pos = enqueue_pos_.value.load(std::memory_order_relaxed);
            }
        }

        cell->data = std::move(value);
        cell->sequence.store(pos + 1, std::memory_order_release);
        return true;
    }

    [[nodiscard]] bool pop(T& out) noexcept
    {
        Cell* cell = nullptr;
        std::size_t pos = dequeue_pos_.value.load(std::memory_order_relaxed);

        for (;;) {
            cell = &cells_[pos & kMask];
            const std::size_t sequence = cell->sequence.load(std::memory_order_acquire);
            const auto diff =
                static_cast<std::intptr_t>(sequence) - static_cast<std::intptr_t>(pos + 1);

            if (diff == 0) {
                if (dequeue_pos_.value.compare_exchange_weak(pos, pos + 1,
                                                             std::memory_order_relaxed)) {
                    break;
                }
            } else if (diff < 0) {
                return false;
            } else {
                pos = dequeue_pos_.value.load(std::memory_order_relaxed);
            }
        }

        out = std::move(cell->data);
        cell->sequence.store(pos + kMask + 1, std::memory_order_release);
        return true;
    }

    [[nodiscard]] std::size_t size_approx() const noexcept
    {
        const std::size_t tail = enqueue_pos_.value.load(std::memory_order_acquire);
        const std::size_t head = dequeue_pos_.value.load(std::memory_order_acquire);
        return tail - head;
    }

private:
    static constexpr std::size_t kMask = Capacity - 1;

    struct Cell {
        std::atomic<std::size_t> sequence{0};
        T data{};
    };

    struct alignas(kCacheLineSize) PaddedIndex {
        std::atomic<std::size_t> value{0};
    };

    alignas(kCacheLineSize) Cell cells_[Capacity]{};
    PaddedIndex enqueue_pos_;
    PaddedIndex dequeue_pos_;
};
}  // namespace tl
