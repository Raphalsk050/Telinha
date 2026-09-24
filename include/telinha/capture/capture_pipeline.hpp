#pragma once

#include <cstdint>
#include <memory>

#include "telinha/capture/capture_source.hpp"
#include "telinha/capture/capture_stats.hpp"
#include "telinha/core/arena.hpp"
#include "telinha/core/result.hpp"
#include "telinha/core/tile_dirty_map.hpp"

namespace tl::capture {
struct ClassifiedFrame {
    const CapturedFrame* frame = nullptr;
    const TileDirtyMap* tiles = nullptr;
    std::uint32_t dirty_tile_count = 0;
};

class CapturePipeline {
public:
    CapturePipeline() noexcept = default;
    ~CapturePipeline();

    CapturePipeline(const CapturePipeline&) = delete;
    CapturePipeline& operator=(const CapturePipeline&) = delete;

    [[nodiscard]] Outcome initialize(std::unique_ptr<CaptureSource> source,
                                     const CaptureOptions& options);

    [[nodiscard]] Outcome start();
    void stop() noexcept;

    [[nodiscard]] Outcome capture_next(ClassifiedFrame& out, std::uint32_t timeout_ms);

    [[nodiscard]] Outcome reconfigure();

    void release() noexcept;

    void keep_dirty() noexcept;

    [[nodiscard]] const CaptureStats& stats() const noexcept { return stats_; }
    [[nodiscard]] CaptureStats& stats() noexcept { return stats_; }
    [[nodiscard]] CaptureSourceInfo info() const noexcept;
    [[nodiscard]] std::size_t arena_high_water() const noexcept
    {
        return storage_.arena().high_water_mark();
    }

private:
    [[nodiscard]] Outcome prepare_tiles(const CaptureSourceInfo& source_info);
    [[nodiscard]] Outcome rebuild_tiles(std::uint32_t width, std::uint32_t height);
    [[nodiscard]] Outcome apply_surface_extent(std::uint32_t width, std::uint32_t height);
    [[nodiscard]] std::uint32_t tile_span(std::uint32_t extent) const noexcept;

    std::unique_ptr<CaptureSource> source_;
    CaptureOptions options_;
    ArenaStorage storage_;
    TileDirtyMap tiles_;
    LinearArena::Marker tiles_marker_{};
    CapturedFrame current_frame_;
    CaptureStats stats_;
    Nanoseconds last_present_ns_ = 0;
    bool frame_held_ = false;
    bool started_ = false;
    bool carry_dirty_ = false;
};
}  // namespace tl::capture
