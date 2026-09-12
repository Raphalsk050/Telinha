#pragma once

#include <cstdint>
#include <cstdio>
#include <utility>

#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/latency_histogram.hpp"

#if TL_COMPILER_MSVC
#include <intrin.h>
#endif

namespace tl::bench {

#if TL_COMPILER_MSVC

template<typename T>
TL_FORCEINLINE void do_not_optimize(const T& value) noexcept
{
    const volatile char* sink = reinterpret_cast<const volatile char*>(&value);
    (void)*sink;
    _ReadWriteBarrier();
}

TL_FORCEINLINE void clobber_memory() noexcept
{
    _ReadWriteBarrier();
}

#else

template<typename T>
TL_FORCEINLINE void do_not_optimize(const T& value) noexcept
{
    __asm__ __volatile__("" : : "r,m"(value) : "memory");
}

TL_FORCEINLINE void clobber_memory() noexcept
{
    __asm__ __volatile__("" : : : "memory");
}

#endif

struct BenchmarkResult {
    const char* name = "";
    std::uint64_t iterations = 0;
    std::uint64_t items_per_iteration = 1;
    LatencyHistogram::Report report;
};

template<typename Fn>
[[nodiscard]] BenchmarkResult run(const char* name, std::uint64_t iterations, Fn&& body,
                                  std::uint64_t items_per_iteration = 1)
{
    const std::uint64_t warmup = iterations / 8 + 1;
    for (std::uint64_t i = 0; i < warmup; ++i) {
        body();
    }

    LatencyHistogram histogram;
    for (std::uint64_t i = 0; i < iterations; ++i) {
        const Nanoseconds start = now_ns();
        body();
        const Nanoseconds finish = now_ns();
        histogram.record(finish - start);
    }

    BenchmarkResult result;
    result.name = name;
    result.iterations = iterations;
    result.items_per_iteration = items_per_iteration;
    result.report = histogram.report();
    return result;
}

inline void print(const BenchmarkResult& result)
{
    const double items =
        result.items_per_iteration == 0 ? 1.0 : static_cast<double>(result.items_per_iteration);

    std::printf(
        "%-40s n=%-8llu  p50 %8.2f  p99 %8.2f  p99.9 %8.2f  1%%low %8.2f  max %8.2f  "
        "(ns per item)\n",
        result.name, static_cast<unsigned long long>(result.iterations),
        static_cast<double>(result.report.p50_ns) / items,
        static_cast<double>(result.report.p99_ns) / items,
        static_cast<double>(result.report.p999_ns) / items,
        result.report.low_one_percent_ns / items,
        static_cast<double>(result.report.max_ns) / items);
}

}  // namespace tl::bench
