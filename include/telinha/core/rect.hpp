#pragma once

#include <algorithm>
#include <cstddef>
#include <cstdint>

#include "telinha/core/arena.hpp"
#include "telinha/core/config.hpp"

namespace tl {
struct Rect {
    std::int32_t left = 0;
    std::int32_t top = 0;
    std::int32_t right = 0;
    std::int32_t bottom = 0;

    [[nodiscard]] constexpr std::int32_t width() const noexcept { return right - left; }
    [[nodiscard]] constexpr std::int32_t height() const noexcept { return bottom - top; }
    [[nodiscard]] constexpr bool empty() const noexcept { return right <= left || bottom <= top; }
    [[nodiscard]] constexpr std::int64_t area() const noexcept
    {
        return empty() ? 0 : static_cast<std::int64_t>(width()) * height();
    }

    friend constexpr bool operator==(const Rect& a, const Rect& b) noexcept
    {
        return a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom;
    }
    friend constexpr bool operator!=(const Rect& a, const Rect& b) noexcept { return !(a == b); }
};

class RectSoA {
public:
    RectSoA() noexcept = default;

    [[nodiscard]] bool initialize(LinearArena& arena, std::uint32_t capacity) noexcept
    {
        left_ = arena.allocate_array_aligned<std::int32_t>(capacity, kCacheLineSize);
        top_ = arena.allocate_array_aligned<std::int32_t>(capacity, kCacheLineSize);
        right_ = arena.allocate_array_aligned<std::int32_t>(capacity, kCacheLineSize);
        bottom_ = arena.allocate_array_aligned<std::int32_t>(capacity, kCacheLineSize);
        if (left_ == nullptr || top_ == nullptr || right_ == nullptr || bottom_ == nullptr) {
            *this = RectSoA{};
            return false;
        }
        capacity_ = capacity;
        count_ = 0;
        return true;
    }

    [[nodiscard]] bool push(const Rect& rect) noexcept
    {
        if (count_ >= capacity_) {
            return false;
        }
        left_[count_] = rect.left;
        top_[count_] = rect.top;
        right_[count_] = rect.right;
        bottom_[count_] = rect.bottom;
        ++count_;
        return true;
    }

    void clear() noexcept { count_ = 0; }

    [[nodiscard]] Rect at(std::uint32_t index) const noexcept
    {
        TL_ASSERT(index < count_);
        return Rect{left_[index], top_[index], right_[index], bottom_[index]};
    }

    [[nodiscard]] std::uint32_t count() const noexcept { return count_; }
    [[nodiscard]] std::uint32_t capacity() const noexcept { return capacity_; }
    [[nodiscard]] bool empty() const noexcept { return count_ == 0; }

    [[nodiscard]] const std::int32_t* left() const noexcept { return left_; }
    [[nodiscard]] const std::int32_t* top() const noexcept { return top_; }
    [[nodiscard]] const std::int32_t* right() const noexcept { return right_; }
    [[nodiscard]] const std::int32_t* bottom() const noexcept { return bottom_; }

    [[nodiscard]] Rect bounds() const noexcept
    {
        if (count_ == 0) {
            return Rect{};
        }
        std::int32_t l = left_[0];
        std::int32_t t = top_[0];
        std::int32_t r = right_[0];
        std::int32_t b = bottom_[0];
        for (std::uint32_t i = 1; i < count_; ++i) {
            l = std::min(l, left_[i]);
            t = std::min(t, top_[i]);
            r = std::max(r, right_[i]);
            b = std::max(b, bottom_[i]);
        }
        return Rect{l, t, r, b};
    }

    [[nodiscard]] std::int64_t total_area() const noexcept
    {
        std::int64_t sum = 0;
        for (std::uint32_t i = 0; i < count_; ++i) {
            const std::int32_t w = right_[i] - left_[i];
            const std::int32_t h = bottom_[i] - top_[i];
            if (w > 0 && h > 0) {
                sum += static_cast<std::int64_t>(w) * h;
            }
        }
        return sum;
    }

private:
    std::int32_t* left_ = nullptr;
    std::int32_t* top_ = nullptr;
    std::int32_t* right_ = nullptr;
    std::int32_t* bottom_ = nullptr;
    std::uint32_t capacity_ = 0;
    std::uint32_t count_ = 0;
};
}  // namespace tl
