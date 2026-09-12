#include "telinha/core/clock.hpp"

#if TL_PLATFORM_WINDOWS
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
#include <ctime>
#endif

namespace tl {
namespace {
#if TL_PLATFORM_WINDOWS

struct ClockCalibration {
    std::uint64_t frequency = 1;

    ClockCalibration() noexcept
    {
        LARGE_INTEGER value{};
        if (QueryPerformanceFrequency(&value) != 0 && value.QuadPart > 0) {
            frequency = static_cast<std::uint64_t>(value.QuadPart);
        }
    }
};

const ClockCalibration& calibration() noexcept
{
    static const ClockCalibration instance;
    return instance;
}

#endif
}  // namespace

std::uint64_t now_ticks() noexcept
{
#if TL_PLATFORM_WINDOWS
    LARGE_INTEGER value{};
    QueryPerformanceCounter(&value);
    return static_cast<std::uint64_t>(value.QuadPart);
#else
    timespec ts{};
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return static_cast<std::uint64_t>(ts.tv_sec) * kNanosecondsPerSecond +
           static_cast<std::uint64_t>(ts.tv_nsec);
#endif
}

std::uint64_t ticks_per_second() noexcept
{
#if TL_PLATFORM_WINDOWS
    return calibration().frequency;
#else
    return kNanosecondsPerSecond;
#endif
}

Nanoseconds ticks_to_ns(std::uint64_t ticks) noexcept
{
#if TL_PLATFORM_WINDOWS
    const std::uint64_t frequency = calibration().frequency;

    const std::uint64_t seconds = ticks / frequency;
    const std::uint64_t remainder = ticks - seconds * frequency;
    return seconds * kNanosecondsPerSecond + (remainder * kNanosecondsPerSecond) / frequency;
#else
    return ticks;
#endif
}

Nanoseconds now_ns() noexcept
{
    return ticks_to_ns(now_ticks());
}
}  // namespace tl
