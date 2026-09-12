#include "region_of_interest.hpp"

namespace tl::encode {

std::int8_t RegionOfInterestMap::clamp_delta(std::int32_t delta) noexcept
{
    if (delta < -128) {
        return -128;
    }
    if (delta > 127) {
        return 127;
    }
    return static_cast<std::int8_t>(delta);
}

bool RegionOfInterestMap::initialize(LinearArena& arena, std::uint32_t surface_width,
                                     std::uint32_t surface_height,
                                     std::uint32_t coding_unit_size) noexcept
{
    if (surface_width == 0 || surface_height == 0 || coding_unit_size == 0) {
        return false;
    }

    units_x_ = (surface_width + coding_unit_size - 1) / coding_unit_size;
    units_y_ = (surface_height + coding_unit_size - 1) / coding_unit_size;
    unit_count_ = units_x_ * units_y_;
    coding_unit_size_ = coding_unit_size;

    deltas_ = arena.allocate_array_aligned<std::int8_t>(unit_count_, kCacheLineSize);
    if (deltas_ == nullptr) {
        *this = RegionOfInterestMap{};
        return false;
    }

    fill(0);
    return true;
}

void RegionOfInterestMap::fill(std::int32_t delta) noexcept
{
    const std::int8_t value = clamp_delta(delta);
    for (std::uint32_t i = 0; i < unit_count_; ++i) {
        deltas_[i] = value;
    }
    dirty_unit_count_ = unit_count_;
}

void RegionOfInterestMap::build(const TileDirtyMap& tiles, std::int32_t dirty_delta,
                                std::int32_t static_delta) noexcept
{
    if (deltas_ == nullptr) {
        return;
    }
    if (!tiles.valid()) {
        fill(dirty_delta);
        return;
    }

    const std::int8_t dirty_value = clamp_delta(dirty_delta);
    const std::int8_t static_value = clamp_delta(static_delta);
    const std::uint32_t tile_size = tiles.tile_size();
    dirty_unit_count_ = 0;

    if (tile_size == coding_unit_size_ && tiles.tiles_x() == units_x_ &&
        tiles.tiles_y() == units_y_) {
        const Span<const std::uint64_t> words = tiles.words();
        for (std::uint32_t unit = 0; unit < unit_count_; ++unit) {
            const bool dirty = (words[unit >> 6] & (std::uint64_t{1} << (unit & 63))) != 0;
            deltas_[unit] = dirty ? dirty_value : static_value;
            dirty_unit_count_ += dirty ? 1u : 0u;
        }
        return;
    }

    for (std::uint32_t unit_y = 0; unit_y < units_y_; ++unit_y) {
        const std::uint32_t first_tile_y = (unit_y * coding_unit_size_) / tile_size;
        const std::uint32_t last_tile_y = ((unit_y + 1) * coding_unit_size_ - 1) / tile_size;

        for (std::uint32_t unit_x = 0; unit_x < units_x_; ++unit_x) {
            const std::uint32_t first_tile_x = (unit_x * coding_unit_size_) / tile_size;
            const std::uint32_t last_tile_x = ((unit_x + 1) * coding_unit_size_ - 1) / tile_size;

            bool dirty = false;
            for (std::uint32_t tile_y = first_tile_y; tile_y <= last_tile_y && !dirty; ++tile_y) {
                for (std::uint32_t tile_x = first_tile_x; tile_x <= last_tile_x; ++tile_x) {
                    if (tiles.is_dirty(tile_x, tile_y)) {
                        dirty = true;
                        break;
                    }
                }
            }

            deltas_[unit_y * units_x_ + unit_x] = dirty ? dirty_value : static_value;
            dirty_unit_count_ += dirty ? 1u : 0u;
        }
    }
}

std::uint32_t RegionOfInterestMap::extract_dirty_rects(RectSoA& out) const noexcept
{
    out.clear();
    if (deltas_ == nullptr || unit_count_ == 0) {
        return 0;
    }

    const std::int8_t static_value = deltas_[0];
    std::uint32_t emitted = 0;

    for (std::uint32_t unit_y = 0; unit_y < units_y_; ++unit_y) {
        std::uint32_t run_start = units_x_;

        for (std::uint32_t unit_x = 0; unit_x <= units_x_; ++unit_x) {
            const bool dirty =
                unit_x < units_x_ && deltas_[unit_y * units_x_ + unit_x] != static_value;

            if (dirty && run_start == units_x_) {
                run_start = unit_x;
                continue;
            }
            if (!dirty && run_start != units_x_) {
                const Rect rect{static_cast<std::int32_t>(run_start * coding_unit_size_),
                                static_cast<std::int32_t>(unit_y * coding_unit_size_),
                                static_cast<std::int32_t>(unit_x * coding_unit_size_),
                                static_cast<std::int32_t>((unit_y + 1) * coding_unit_size_)};
                if (out.push(rect)) {
                    ++emitted;
                }
                run_start = units_x_;
            }
        }
    }

    return emitted;
}

}  // namespace tl::encode
