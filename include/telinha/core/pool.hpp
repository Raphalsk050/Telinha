#pragma once

#include <cstddef>
#include <cstdint>
#include <type_traits>
#include <utility>

#include "telinha/core/arena.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/handle.hpp"
#include "telinha/core/span.hpp"

namespace tl {
template<typename T, typename Tag = T>
class Pool {
public:
    using handle_type = Handle<Tag>;

    Pool() noexcept = default;

    Pool(const Pool&) = delete;
    Pool& operator=(const Pool&) = delete;

    [[nodiscard]] bool initialize(LinearArena& arena, std::uint32_t capacity) noexcept
    {
        static_assert(std::is_trivially_destructible_v<T>,
                      "pooled objects are released without running a destructor");
        if (capacity == 0 || capacity >= kInvalidIndex) {
            return false;
        }

        slots_ = arena.allocate_array_aligned<T>(capacity, kCacheLineSize);
        generations_ = arena.allocate_array<std::uint32_t>(capacity);
        next_free_ = arena.allocate_array<std::uint32_t>(capacity);
        if (slots_ == nullptr || generations_ == nullptr || next_free_ == nullptr) {
            slots_ = nullptr;
            generations_ = nullptr;
            next_free_ = nullptr;
            return false;
        }

        capacity_ = capacity;
        for (std::uint32_t i = 0; i < capacity; ++i) {
            generations_[i] = 0;
            next_free_[i] = i + 1;
        }
        next_free_[capacity - 1] = kInvalidIndex;
        free_head_ = 0;
        live_count_ = 0;
        return true;
    }

    template<typename... Args>
    [[nodiscard]] handle_type acquire(Args&&... args) noexcept
    {
        if (TL_UNLIKELY(free_head_ == kInvalidIndex)) {
            return handle_type{};
        }
        const std::uint32_t index = free_head_;
        free_head_ = next_free_[index];
        ++generations_[index];
        ::new (static_cast<void*>(&slots_[index])) T(std::forward<Args>(args)...);
        ++live_count_;
        return handle_type{index, generations_[index]};
    }

    bool release(handle_type handle) noexcept
    {
        if (!alive(handle)) {
            return false;
        }
        const std::uint32_t index = handle.index;
        ++generations_[index];
        next_free_[index] = free_head_;
        free_head_ = index;
        --live_count_;
        return true;
    }

    [[nodiscard]] bool alive(handle_type handle) const noexcept
    {
        return handle.index < capacity_ && (handle.generation & 1u) != 0 &&
               generations_[handle.index] == handle.generation;
    }

    [[nodiscard]] T* get(handle_type handle) noexcept
    {
        return alive(handle) ? &slots_[handle.index] : nullptr;
    }
    [[nodiscard]] const T* get(handle_type handle) const noexcept
    {
        return alive(handle) ? &slots_[handle.index] : nullptr;
    }

    [[nodiscard]] T& at(std::uint32_t index) noexcept
    {
        TL_ASSERT(index < capacity_);
        return slots_[index];
    }
    [[nodiscard]] const T& at(std::uint32_t index) const noexcept
    {
        TL_ASSERT(index < capacity_);
        return slots_[index];
    }

    [[nodiscard]] std::uint32_t capacity() const noexcept { return capacity_; }
    [[nodiscard]] std::uint32_t live_count() const noexcept { return live_count_; }
    [[nodiscard]] bool empty() const noexcept { return live_count_ == 0; }
    [[nodiscard]] bool full() const noexcept { return free_head_ == kInvalidIndex; }
    [[nodiscard]] bool initialized() const noexcept { return slots_ != nullptr; }

    [[nodiscard]] Span<T> storage() noexcept { return Span<T>(slots_, capacity_); }

private:
    T* slots_ = nullptr;
    std::uint32_t* generations_ = nullptr;
    std::uint32_t* next_free_ = nullptr;
    std::uint32_t capacity_ = 0;
    std::uint32_t free_head_ = kInvalidIndex;
    std::uint32_t live_count_ = 0;
};
}  // namespace tl
