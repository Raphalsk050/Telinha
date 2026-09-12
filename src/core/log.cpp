#include "telinha/core/log.hpp"

#include <atomic>
#include <cstdio>
#include <cstring>

#include "telinha/core/mpmc_ring.hpp"

namespace tl {
namespace {
std::uint32_t current_thread_id() noexcept
{
    static std::atomic<std::uint32_t> next_id{1};
    static thread_local const std::uint32_t id = next_id.fetch_add(1, std::memory_order_relaxed);
    return id;
}
}  // namespace

const char* to_string(LogLevel level) noexcept
{
    switch (level) {
        case LogLevel::Trace: return "trace";
        case LogLevel::Debug: return "debug";
        case LogLevel::Info: return "info";
        case LogLevel::Warn: return "warn";
        case LogLevel::Error: return "error";
        case LogLevel::Off: return "off";
    }
    return "?";
}

struct Logger::Impl {
    MpmcRing<LogRecord, kLogRingCapacity> ring;
    std::atomic<LogLevel> min_level{LogLevel::Info};
    std::atomic<std::uint64_t> dropped{0};
};

Logger::Logger() : impl_(new Impl()) {}

Logger::~Logger()
{
    delete impl_;
}

Logger& Logger::instance() noexcept
{
    static Logger logger;
    return logger;
}

void Logger::set_min_level(LogLevel level) noexcept
{
    impl_->min_level.store(level, std::memory_order_relaxed);
}

LogLevel Logger::min_level() const noexcept
{
    return impl_->min_level.load(std::memory_order_relaxed);
}

bool Logger::enabled(LogLevel level) const noexcept
{
    const LogLevel minimum = impl_->min_level.load(std::memory_order_relaxed);
    return minimum != LogLevel::Off && level >= minimum;
}

void Logger::write(LogLevel level, const char* format, ...) noexcept
{
    std::va_list args;
    va_start(args, format);
    write_v(level, format, args);
    va_end(args);
}

void Logger::write_v(LogLevel level, const char* format, std::va_list args) noexcept
{
    if (!enabled(level)) {
        return;
    }

    LogRecord record;
    record.timestamp_ns = now_ns();
    record.thread_id = current_thread_id();
    record.level = level;

    const int written = std::vsnprintf(record.message, kLogMessageCapacity, format, args);
    if (written < 0) {
        static constexpr char kFormatError[] = "<format error>";
        static_assert(sizeof(kFormatError) <= kLogMessageCapacity);
        std::memcpy(record.message, kFormatError, sizeof(kFormatError));
    }
    record.message[kLogMessageCapacity - 1] = '\0';

    if (!impl_->ring.push(std::move(record))) {
        impl_->dropped.fetch_add(1, std::memory_order_relaxed);
    }
}

bool Logger::pop(LogRecord& out) noexcept
{
    return impl_->ring.pop(out);
}

std::size_t Logger::drain_to_stderr() noexcept
{
    return drain([](const LogRecord& record) {
        const double seconds = static_cast<double>(record.timestamp_ns) / 1e9;
        std::fprintf(stderr, "[%12.6f] [%-5s] [t%02u] %s\n", seconds, to_string(record.level),
                     record.thread_id, record.message);
    });
}

std::uint64_t Logger::dropped_records() const noexcept
{
    return impl_->dropped.load(std::memory_order_relaxed);
}

void Logger::reset_dropped_records() noexcept
{
    impl_->dropped.store(0, std::memory_order_relaxed);
}
}  // namespace tl
