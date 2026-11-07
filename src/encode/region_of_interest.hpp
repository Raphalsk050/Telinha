#pragma once

#include <cstdint>

#include "telinha/core/arena.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/rect.hpp"
#include "telinha/core/span.hpp"
#include "telinha/core/tile_dirty_map.hpp"

namespace tl::encode {

class RegionOfInterestMap {
public:
    RegionOfInterestMap() noexcept = default;

    [[nodiscard]] bool initialize(LinearArena& arena, std::uint32_t surface_width,
                                  std::uint32_t surface_height,
                                  std::uint32_t coding_unit_size) noexcept;

    void fill(std::int32_t delta) noexcept;

    // A unit stays at dirty_delta for kHoldFrames after it last changed, and a frame where at
    // least kFlatDirtyPercent of the units are dirty gets dirty_delta everywhere. The dirty rects
    // Windows reports for a game window can miss parts of what changed; a unit wrongly taken as
    // static gets a high QP, the encoder skips it, and the old picture stays there.
    void build(const TileDirtyMap& tiles, std::int32_t dirty_delta,
               std::int32_t static_delta) noexcept;

    static constexpr std::uint8_t kHoldFrames = 60;
    static constexpr std::uint32_t kFlatDirtyPercent = 25;

    [[nodiscard]] std::uint32_t extract_dirty_rects(RectSoA& out) const noexcept;

    [[nodiscard]] Span<const std::int8_t> deltas() const noexcept
    {
        return Span<const std::int8_t>(deltas_, unit_count_);
    }

    [[nodiscard]] std::uint32_t units_x() const noexcept { return units_x_; }
    [[nodiscard]] std::uint32_t units_y() const noexcept { return units_y_; }
    [[nodiscard]] std::uint32_t unit_count() const noexcept { return unit_count_; }
    [[nodiscard]] std::uint32_t coding_unit_size() const noexcept { return coding_unit_size_; }
    [[nodiscard]] std::uint32_t dirty_unit_count() const noexcept { return dirty_unit_count_; }
    [[nodiscard]] bool valid() const noexcept { return deltas_ != nullptr; }

    [[nodiscard]] double dirty_ratio() const noexcept
    {
        return unit_count_ == 0
                   ? 0.0
                   : static_cast<double>(dirty_unit_count_) / static_cast<double>(unit_count_);
    }

private:
    [[nodiscard]] static std::int8_t clamp_delta(std::int32_t delta) noexcept;
    void set_unit(std::uint32_t unit, bool dirty, std::int8_t dirty_value,
                  std::int8_t static_value) noexcept;
    void flatten_if_mostly_dirty(std::int8_t dirty_value) noexcept;

    std::int8_t* deltas_ = nullptr;
    std::uint8_t* hold_ = nullptr;
    std::uint32_t units_x_ = 0;
    std::uint32_t units_y_ = 0;
    std::uint32_t unit_count_ = 0;
    std::uint32_t coding_unit_size_ = 0;
    std::uint32_t dirty_unit_count_ = 0;
};

}  // namespace tl::encode
