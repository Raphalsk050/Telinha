#include "dirty_region_builder.hpp"

#include <algorithm>

namespace tl::capture {
namespace {

[[nodiscard]] Rect union_of(const Rect& a, const Rect& b) noexcept
{
    return Rect{std::min(a.left, b.left), std::min(a.top, b.top), std::max(a.right, b.right),
                std::max(a.bottom, b.bottom)};
}

[[nodiscard]] bool contains(const Rect& outer, const Rect& inner) noexcept
{
    return outer.left <= inner.left && outer.top <= inner.top && outer.right >= inner.right &&
           outer.bottom >= inner.bottom;
}

}  // namespace

bool DirtyRegionBuilder::initialize(LinearArena& arena, std::uint32_t surface_width,
                                    std::uint32_t surface_height, const Limits& limits) noexcept
{
    if (surface_width == 0 || surface_height == 0 || limits.max_dirty_rects == 0) {
        return false;
    }

    staged_ = arena.allocate_array_aligned<Rect>(limits.max_dirty_rects, kCacheLineSize);
    if (staged_ == nullptr) {
        return false;
    }
    if (limits.max_move_rects != 0) {
        moves_ = arena.allocate_array_aligned<MoveRect>(limits.max_move_rects, kCacheLineSize);
        if (moves_ == nullptr) {
            staged_ = nullptr;
            return false;
        }
    }
    if (!emitted_.initialize(arena, limits.max_dirty_rects)) {
        staged_ = nullptr;
        moves_ = nullptr;
        return false;
    }

    limits_ = limits;
    staged_capacity_ = limits.max_dirty_rects;
    move_capacity_ = limits.max_move_rects;
    resize(surface_width, surface_height);
    return true;
}

void DirtyRegionBuilder::resize(std::uint32_t surface_width, std::uint32_t surface_height) noexcept
{
    surface_width_ = static_cast<std::int32_t>(surface_width);
    surface_height_ = static_cast<std::int32_t>(surface_height);
    staged_count_ = 0;
    staged_area_ = 0;
    move_count_ = 0;
    full_surface_ = true;
    metadata_absent_ = false;
    frame_open_ = false;
    emitted_.clear();
}

std::int64_t DirtyRegionBuilder::surface_area() const noexcept
{
    return static_cast<std::int64_t>(surface_width_) * surface_height_;
}

void DirtyRegionBuilder::begin_frame() noexcept
{
    staged_count_ = 0;
    staged_area_ = 0;
    move_count_ = 0;
    full_surface_ = false;
    metadata_absent_ = false;
    frame_open_ = true;
    emitted_.clear();
}

void DirtyRegionBuilder::force_full_surface() noexcept
{
    full_surface_ = true;
    staged_count_ = 0;
    staged_area_ = 0;
}

void DirtyRegionBuilder::force_full_surface_without_metadata() noexcept
{
    force_full_surface();
    metadata_absent_ = true;
    ++statistics_.frames_without_metadata;
}

void DirtyRegionBuilder::stage(Rect candidate) noexcept
{
    for (std::uint32_t pass = 0; pass < kMergeWindow; ++pass) {
        const std::uint32_t window_begin =
            staged_count_ > kMergeWindow ? staged_count_ - kMergeWindow : 0;
        bool merged = false;

        for (std::uint32_t i = staged_count_; i > window_begin;) {
            --i;
            const Rect existing = staged_[i];
            if (contains(existing, candidate)) {
                ++statistics_.merged_rects;
                return;
            }

            const Rect combined = union_of(existing, candidate);
            if (combined.area() <= existing.area() + candidate.area()) {
                staged_area_ -= existing.area();
                staged_[i] = staged_[staged_count_ - 1];
                --staged_count_;
                candidate = combined;
                ++statistics_.merged_rects;
                merged = true;
                break;
            }
        }

        if (!merged) {
            break;
        }
    }

    if (staged_count_ >= staged_capacity_) {
        ++statistics_.overflow_frames;
        force_full_surface();
        return;
    }

    staged_[staged_count_] = candidate;
    ++staged_count_;
    staged_area_ += candidate.area();
}

void DirtyRegionBuilder::add_dirty(const Rect& rect) noexcept
{
    TL_ASSERT(frame_open_);
    ++statistics_.submitted_rects;

    if (full_surface_) {
        return;
    }

    const Rect surface = surface_rect();
    const Rect clamped{std::max(rect.left, surface.left), std::max(rect.top, surface.top),
                       std::min(rect.right, surface.right), std::min(rect.bottom, surface.bottom)};
    if (clamped != rect) {
        ++statistics_.clipped_rects;
    }
    if (clamped.empty()) {
        ++statistics_.rejected_rects;
        return;
    }

    stage(clamped);
}

void DirtyRegionBuilder::add_move(const MoveRect& move) noexcept
{
    TL_ASSERT(frame_open_);

    const Rect source{move.source_x, move.source_y, move.source_x + move.destination.width(),
                      move.source_y + move.destination.height()};
    add_dirty(source);
    add_dirty(move.destination);

    if (move_count_ >= move_capacity_) {
        ++statistics_.dropped_move_rects;
        return;
    }
    moves_[move_count_] = move;
    ++move_count_;
}

void DirtyRegionBuilder::finish() noexcept
{
    ++statistics_.frames;
    frame_open_ = false;
    emitted_.clear();

    const std::int64_t area = surface_area();
    if (!full_surface_ && staged_area_ > 0 && area > 0 &&
        staged_area_ * 100 >= area * static_cast<std::int64_t>(limits_.full_surface_area_percent)) {
        ++statistics_.area_collapsed_frames;
        full_surface_ = true;
        staged_count_ = 0;
        staged_area_ = 0;
    }

    if (full_surface_) {
        ++statistics_.full_surface_frames;
        move_count_ = 0;
        if (!metadata_absent_) {
            (void)emitted_.push(surface_rect());
        }
        statistics_.emitted_rects += emitted_.count();
        statistics_.emitted_area += static_cast<std::uint64_t>(area);
        return;
    }

    for (std::uint32_t i = 0; i < staged_count_; ++i) {
        (void)emitted_.push(staged_[i]);
    }
    statistics_.emitted_rects += emitted_.count();
    statistics_.emitted_area += static_cast<std::uint64_t>(staged_area_);
}

}  // namespace tl::capture
