#pragma once

#include <cstdint>

#include "cursor_shape.hpp"
#include "dirty_region_builder.hpp"
#include "telinha/capture/capture_source.hpp"
#include "telinha/core/arena.hpp"

namespace tl::capture {

class SyntheticSource final : public CaptureSource {
public:
    static constexpr std::uint32_t kFullSurfacePeriod = 60;
    static constexpr std::uint32_t kBlockExtent = 128;
    static constexpr std::uint32_t kBlockStep = 16;
    static constexpr std::uint32_t kBlinkExtent = 32;
    static constexpr std::uint32_t kCursorExtent = 32;
    static constexpr std::uint32_t kCursorShapePeriod = 120;

    explicit SyntheticSource(const CaptureTarget& target, const CaptureOptions& options) noexcept;

    Outcome start() noexcept override;
    void stop() noexcept override;
    [[nodiscard]] Outcome acquire(CapturedFrame& out, std::uint32_t timeout_ms) noexcept override;
    void release() noexcept override;
    [[nodiscard]] Outcome map_for_readback(FrameSurface& out) noexcept override;
    void unmap_readback() noexcept override;
    [[nodiscard]] CaptureSourceInfo info() const noexcept override;

    [[nodiscard]] const DirtyRegionBuilder& dirty_region_builder() const noexcept { return dirty_; }
    [[nodiscard]] std::size_t arena_high_water() const noexcept
    {
        return storage_.arena().high_water_mark();
    }

private:
    [[nodiscard]] Outcome wait_for_next_frame(std::uint32_t timeout_ms) noexcept;
    void paint_background() noexcept;
    void fill_rect(const Rect& rect, std::uint8_t blue, std::uint8_t green,
                   std::uint8_t red) noexcept;
    [[nodiscard]] Rect block_bounds(std::uint64_t frame_index) const noexcept;
    [[nodiscard]] Rect blink_bounds() const noexcept;
    [[nodiscard]] Outcome refresh_cursor_shape() noexcept;
    void advance_cursor(std::uint64_t frame_index) noexcept;

    CaptureTarget target_;
    CaptureOptions options_;
    ArenaStorage storage_;
    DirtyRegionBuilder dirty_;
    CursorTracker cursor_;
    std::byte* pixels_ = nullptr;
    std::uint32_t width_ = 0;
    std::uint32_t height_ = 0;
    std::uint32_t row_pitch_ = 0;
    Nanoseconds frame_interval_ns_ = 0;
    Nanoseconds next_present_ns_ = 0;
    std::uint64_t frame_index_ = 0;
    bool started_ = false;
    bool leased_ = false;
};

}  // namespace tl::capture
