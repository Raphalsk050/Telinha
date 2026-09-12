#pragma once

#include <bit>
#include <cstddef>
#include <cstdint>

#include "telinha/core/arena.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/rect.hpp"
#include "telinha/core/span.hpp"

namespace tl {
class TileDirtyMap {
public:
    TileDirtyMap() noexcept = default;

    [[nodiscard]] bool initialize(LinearArena& arena, std::uint32_t surface_width,
                                  std::uint32_t surface_height, std::uint32_t tile_size) noexcept
    {
        if (surface_width == 0 || surface_height == 0 || tile_size == 0) {
            return false;
        }
        tile_size_ = tile_size;
        tiles_x_ = (surface_width + tile_size - 1) / tile_size;
        tiles_y_ = (surface_height + tile_size - 1) / tile_size;

        const std::uint64_t total = static_cast<std::uint64_t>(tiles_x_) * tiles_y_;
        word_count_ = static_cast<std::uint32_t>((total + 63) / 64);
        words_ = arena.allocate_array_aligned<std::uint64_t>(word_count_, kCacheLineSize);
        if (words_ == nullptr) {
            *this = TileDirtyMap{};
            return false;
        }
        clear();
        return true;
    }

    void clear() noexcept
    {
        for (std::uint32_t i = 0; i < word_count_; ++i) {
            words_[i] = 0;
        }
    }

    void mark_all() noexcept
    {
        const std::uint64_t total = static_cast<std::uint64_t>(tiles_x_) * tiles_y_;
        set_range(0, total);
    }

    void mark(const Rect& rect) noexcept
    {
        if (rect.empty() || tiles_x_ == 0 || tiles_y_ == 0) {
            return;
        }

        const std::int64_t left = rect.left < 0 ? 0 : rect.left;
        const std::int64_t top = rect.top < 0 ? 0 : rect.top;
        if (rect.right <= left || rect.bottom <= top) {
            return;
        }

        std::uint32_t first_x = static_cast<std::uint32_t>(left) / tile_size_;
        std::uint32_t first_y = static_cast<std::uint32_t>(top) / tile_size_;
        std::uint32_t last_x = static_cast<std::uint32_t>(rect.right - 1) / tile_size_;
        std::uint32_t last_y = static_cast<std::uint32_t>(rect.bottom - 1) / tile_size_;

        if (first_x >= tiles_x_ || first_y >= tiles_y_) {
            return;
        }
        if (last_x >= tiles_x_) {
            last_x = tiles_x_ - 1;
        }
        if (last_y >= tiles_y_) {
            last_y = tiles_y_ - 1;
        }

        for (std::uint32_t y = first_y; y <= last_y; ++y) {
            const std::uint64_t row_base = static_cast<std::uint64_t>(y) * tiles_x_;
            set_range(row_base + first_x, row_base + last_x + 1);
        }
    }

    void mark(const RectSoA& rects) noexcept
    {
        const std::uint32_t count = rects.count();
        for (std::uint32_t i = 0; i < count; ++i) {
            mark(rects.at(i));
        }
    }

    [[nodiscard]] bool is_dirty(std::uint32_t tile_x, std::uint32_t tile_y) const noexcept
    {
        if (tile_x >= tiles_x_ || tile_y >= tiles_y_) {
            return false;
        }
        const std::uint64_t bit = static_cast<std::uint64_t>(tile_y) * tiles_x_ + tile_x;
        return (words_[bit >> 6] & (std::uint64_t{1} << (bit & 63))) != 0;
    }

    [[nodiscard]] std::uint32_t dirty_tile_count() const noexcept
    {
        std::uint32_t total = 0;
        for (std::uint32_t i = 0; i < word_count_; ++i) {
            total += static_cast<std::uint32_t>(std::popcount(words_[i]));
        }
        return total;
    }

    [[nodiscard]] std::uint32_t tile_count() const noexcept { return tiles_x_ * tiles_y_; }
    [[nodiscard]] std::uint32_t tiles_x() const noexcept { return tiles_x_; }
    [[nodiscard]] std::uint32_t tiles_y() const noexcept { return tiles_y_; }
    [[nodiscard]] std::uint32_t tile_size() const noexcept { return tile_size_; }
    [[nodiscard]] bool valid() const noexcept { return words_ != nullptr; }

    [[nodiscard]] Span<const std::uint64_t> words() const noexcept
    {
        return Span<const std::uint64_t>(words_, word_count_);
    }

private:
    void set_range(std::uint64_t first, std::uint64_t last) noexcept
    {
        if (first >= last) {
            return;
        }
        const std::uint64_t first_word = first >> 6;
        const std::uint64_t last_word = (last - 1) >> 6;

        if (first_word == last_word) {
            const std::uint64_t mask = range_mask(first & 63, ((last - 1) & 63) + 1);
            words_[first_word] |= mask;
            return;
        }

        words_[first_word] |= range_mask(first & 63, 64);
        for (std::uint64_t w = first_word + 1; w < last_word; ++w) {
            words_[w] = ~std::uint64_t{0};
        }
        words_[last_word] |= range_mask(0, ((last - 1) & 63) + 1);
    }

    [[nodiscard]] static constexpr std::uint64_t range_mask(std::uint64_t low,
                                                            std::uint64_t high) noexcept
    {
        const std::uint64_t all = ~std::uint64_t{0};
        const std::uint64_t high_mask = high >= 64 ? all : ((std::uint64_t{1} << high) - 1);
        return high_mask & (all << low);
    }

    std::uint64_t* words_ = nullptr;
    std::uint32_t word_count_ = 0;
    std::uint32_t tiles_x_ = 0;
    std::uint32_t tiles_y_ = 0;
    std::uint32_t tile_size_ = 0;
};
}  // namespace tl
