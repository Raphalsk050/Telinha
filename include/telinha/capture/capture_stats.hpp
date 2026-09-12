#pragma once

#include <cstdint>

#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/latency_histogram.hpp"

namespace tl::capture {
struct CaptureStats {
    LatencyHistogram acquire_ns;
    LatencyHistogram present_interval_ns;
    LatencyHistogram classify_ns;

    std::uint64_t frames_acquired = 0;
    std::uint64_t frames_timed_out = 0;
    std::uint64_t frames_dropped = 0;
    std::uint64_t frames_cursor_only = 0;
    std::uint64_t frames_full_dirty = 0;
    std::uint64_t frames_without_dirty_metadata = 0;
    std::uint64_t frames_coalesced = 0;
    std::uint64_t device_lost_recoveries = 0;

    std::uint64_t dirty_rects_total = 0;
    std::uint64_t dirty_tiles_total = 0;
    std::uint64_t tiles_total = 0;

    void reset() noexcept
    {
        acquire_ns.reset();
        present_interval_ns.reset();
        classify_ns.reset();

        frames_acquired = 0;
        frames_timed_out = 0;
        frames_dropped = 0;
        frames_cursor_only = 0;
        frames_full_dirty = 0;
        frames_without_dirty_metadata = 0;
        frames_coalesced = 0;
        device_lost_recoveries = 0;
        dirty_rects_total = 0;
        dirty_tiles_total = 0;
        tiles_total = 0;
    }

    [[nodiscard]] double mean_dirty_tile_ratio() const noexcept
    {
        return tiles_total == 0
                   ? 0.0
                   : static_cast<double>(dirty_tiles_total) / static_cast<double>(tiles_total);
    }

    [[nodiscard]] double dirty_metadata_coverage() const noexcept
    {
        return frames_acquired == 0 ? 0.0
                                    : 1.0 - static_cast<double>(frames_without_dirty_metadata) /
                                                static_cast<double>(frames_acquired);
    }

    [[nodiscard]] double coalesce_ratio() const noexcept
    {
        return frames_acquired == 0
                   ? 0.0
                   : static_cast<double>(frames_coalesced) / static_cast<double>(frames_acquired);
    }
};

[[nodiscard]] int format_capture_report(const CaptureStats& stats, Nanoseconds budget_ns,
                                        char* buffer, int capacity) noexcept;
}  // namespace tl::capture
