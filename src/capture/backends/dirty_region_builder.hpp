#pragma once

#include <cstdint>

#include "telinha/capture/captured_frame.hpp"
#include "telinha/core/arena.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/rect.hpp"
#include "telinha/core/span.hpp"

namespace tl::capture {

class DirtyRegionBuilder {
public:
    static constexpr std::uint32_t kMergeWindow = 8;

    struct Limits {
        std::uint32_t max_dirty_rects = 512;
        std::uint32_t max_move_rects = 128;
        std::uint32_t full_surface_area_percent = 60;
    };

    struct Statistics {
        std::uint64_t frames = 0;
        std::uint64_t submitted_rects = 0;
        std::uint64_t emitted_rects = 0;
        std::uint64_t clipped_rects = 0;
        std::uint64_t rejected_rects = 0;
        std::uint64_t merged_rects = 0;
        std::uint64_t emitted_area = 0;
        std::uint64_t full_surface_frames = 0;
        std::uint64_t overflow_frames = 0;
        std::uint64_t area_collapsed_frames = 0;
        std::uint64_t dropped_move_rects = 0;
    };

    DirtyRegionBuilder() noexcept = default;

    DirtyRegionBuilder(const DirtyRegionBuilder&) = delete;
    DirtyRegionBuilder& operator=(const DirtyRegionBuilder&) = delete;

    [[nodiscard]] bool initialize(LinearArena& arena, std::uint32_t surface_width,
                                  std::uint32_t surface_height, const Limits& limits) noexcept;

    void resize(std::uint32_t surface_width, std::uint32_t surface_height) noexcept;

    void begin_frame() noexcept;
    void add_dirty(const Rect& rect) noexcept;
    void add_move(const MoveRect& move) noexcept;
    void force_full_surface() noexcept;
    void finish() noexcept;

    [[nodiscard]] const RectSoA& dirty_rects() const noexcept { return emitted_; }
    [[nodiscard]] Span<const MoveRect> move_rects() const noexcept
    {
        return Span<const MoveRect>(moves_, move_count_);
    }
    [[nodiscard]] bool full_surface() const noexcept { return full_surface_; }
    [[nodiscard]] std::uint32_t staged_count() const noexcept { return staged_count_; }
    [[nodiscard]] std::int64_t staged_area() const noexcept { return staged_area_; }

    [[nodiscard]] const Statistics& statistics() const noexcept { return statistics_; }
    void reset_statistics() noexcept { statistics_ = Statistics{}; }
    [[nodiscard]] bool initialized() const noexcept { return staged_ != nullptr; }
    [[nodiscard]] Rect surface_rect() const noexcept
    {
        return Rect{0, 0, surface_width_, surface_height_};
    }

private:
    void stage(Rect candidate) noexcept;
    [[nodiscard]] std::int64_t surface_area() const noexcept;

    Rect* staged_ = nullptr;
    MoveRect* moves_ = nullptr;
    RectSoA emitted_;
    Limits limits_;
    std::uint32_t staged_capacity_ = 0;
    std::uint32_t staged_count_ = 0;
    std::uint32_t move_capacity_ = 0;
    std::uint32_t move_count_ = 0;
    std::int64_t staged_area_ = 0;
    std::int32_t surface_width_ = 0;
    std::int32_t surface_height_ = 0;
    bool full_surface_ = true;
    bool frame_open_ = false;
    Statistics statistics_;
};

}  // namespace tl::capture
