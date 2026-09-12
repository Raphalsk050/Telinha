#include "telinha/capture/capture_stats.hpp"

#include <cstdio>

namespace tl::capture {

int format_capture_report(const CaptureStats& stats, Nanoseconds budget_ns, char* buffer,
                          int capacity) noexcept
{
    if (buffer == nullptr || capacity <= 0) {
        return 0;
    }

    const LatencyHistogram::Report acquire = stats.acquire_ns.report();
    const LatencyHistogram::Report classify = stats.classify_ns.report();
    const LatencyHistogram::Report interval = stats.present_interval_ns.report();

    return std::snprintf(
        buffer, static_cast<std::size_t>(capacity),
        "frames        acquired %llu  timed_out %llu  dropped %llu  cursor_only %llu\n"
        "              full_dirty %llu  coalesced %llu (%.2f per frame)  recoveries %llu\n"
        "acquire  ms   p50 %.3f  p95 %.3f  p99 %.3f  p99.9 %.3f  1%% low %.3f  max %.3f\n"
        "classify ms   p50 %.3f  p99 %.3f  max %.3f\n"
        "interval ms   p50 %.3f  p99 %.3f  1%% low %.3f\n"
        "tiles         dirty ratio %.4f  rects/frame %.2f\n"
        "budget   ms   %.3f  acquire+classify p99 %.3f  %s\n",
        static_cast<unsigned long long>(stats.frames_acquired),
        static_cast<unsigned long long>(stats.frames_timed_out),
        static_cast<unsigned long long>(stats.frames_dropped),
        static_cast<unsigned long long>(stats.frames_cursor_only),
        static_cast<unsigned long long>(stats.frames_full_dirty),
        static_cast<unsigned long long>(stats.frames_coalesced), stats.coalesce_ratio(),
        static_cast<unsigned long long>(stats.device_lost_recoveries), ns_to_ms(acquire.p50_ns),
        ns_to_ms(acquire.p95_ns), ns_to_ms(acquire.p99_ns), ns_to_ms(acquire.p999_ns),
        acquire.low_one_percent_ns / 1.0e6, ns_to_ms(acquire.max_ns), ns_to_ms(classify.p50_ns),
        ns_to_ms(classify.p99_ns), ns_to_ms(classify.max_ns), ns_to_ms(interval.p50_ns),
        ns_to_ms(interval.p99_ns), interval.low_one_percent_ns / 1.0e6,
        stats.mean_dirty_tile_ratio(),
        stats.frames_acquired == 0 ? 0.0
                                   : static_cast<double>(stats.dirty_rects_total) /
                                         static_cast<double>(stats.frames_acquired),
        ns_to_ms(budget_ns), ns_to_ms(acquire.p99_ns + classify.p99_ns),
        (acquire.p99_ns + classify.p99_ns) <= budget_ns ? "within budget" : "OVER BUDGET");
}

}  // namespace tl::capture
