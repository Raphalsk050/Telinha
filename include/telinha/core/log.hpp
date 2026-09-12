#pragma once

#include <cstdarg>
#include <cstddef>
#include <cstdint>

#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"

namespace tl {
enum class LogLevel : std::uint8_t {
    Trace = 0,
    Debug,
    Info,
    Warn,
    Error,
    Off,
};

const char* to_string(LogLevel level) noexcept;

inline constexpr std::size_t kLogMessageCapacity = 184;
inline constexpr std::size_t kLogRingCapacity = 1024;

struct LogRecord {
    Nanoseconds timestamp_ns = 0;
    std::uint32_t thread_id = 0;
    LogLevel level = LogLevel::Info;
    char message[kLogMessageCapacity] = {};
};

class Logger {
public:
    static Logger& instance() noexcept;

    void set_min_level(LogLevel level) noexcept;
    [[nodiscard]] LogLevel min_level() const noexcept;

    [[nodiscard]] bool enabled(LogLevel level) const noexcept;

    void write(LogLevel level, const char* format, ...) noexcept;
    void write_v(LogLevel level, const char* format, std::va_list args) noexcept;

    template<typename Sink>
    std::size_t drain(Sink&& sink) noexcept
    {
        LogRecord record;
        std::size_t delivered = 0;
        while (pop(record)) {
            sink(record);
            ++delivered;
        }
        return delivered;
    }

    std::size_t drain_to_stderr() noexcept;

    [[nodiscard]] std::uint64_t dropped_records() const noexcept;

    void reset_dropped_records() noexcept;

private:
    Logger();
    ~Logger();

    Logger(const Logger&) = delete;
    Logger& operator=(const Logger&) = delete;

    [[nodiscard]] bool pop(LogRecord& out) noexcept;

    struct Impl;
    Impl* impl_;
};
}  // namespace tl

#define TL_LOG(level, ...)                                   \
    do {                                                     \
        ::tl::Logger& tl_logger_ = ::tl::Logger::instance(); \
        if (tl_logger_.enabled(level)) {                     \
            tl_logger_.write(level, __VA_ARGS__);            \
        }                                                    \
    } while (false)

#define TL_LOG_TRACE(...) TL_LOG(::tl::LogLevel::Trace, __VA_ARGS__)
#define TL_LOG_DEBUG(...) TL_LOG(::tl::LogLevel::Debug, __VA_ARGS__)
#define TL_LOG_INFO(...) TL_LOG(::tl::LogLevel::Info, __VA_ARGS__)
#define TL_LOG_WARN(...) TL_LOG(::tl::LogLevel::Warn, __VA_ARGS__)
#define TL_LOG_ERROR(...) TL_LOG(::tl::LogLevel::Error, __VA_ARGS__)
