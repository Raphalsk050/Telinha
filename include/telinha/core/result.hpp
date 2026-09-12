#pragma once

#include <cstdint>
#include <new>
#include <type_traits>
#include <utility>

#include "telinha/core/config.hpp"

namespace tl {
enum class Status : std::uint32_t {
    Ok = 0,

    Unknown,
    InvalidArgument,
    OutOfMemory,
    OutOfRange,
    NotImplemented,
    NotSupported,
    Unavailable,
    AlreadyExists,
    NotFound,
    PermissionDenied,

    Timeout,
    WouldBlock,
    Empty,
    Full,

    DeviceLost,
    TargetGone,
    ConfigurationChanged,

    PlatformError,
};

const char* to_string(Status status) noexcept;

struct Error {
    Status status = Status::Unknown;
    std::int32_t platform_code = 0;
    const char* context = "";

    constexpr Error() noexcept = default;

    constexpr explicit Error(Status s, const char* ctx = "", std::int32_t code = 0) noexcept
        : status(s), platform_code(code), context(ctx)
    {}
};

static_assert(std::is_trivially_copyable_v<Error>);

class [[nodiscard]] Outcome {
public:
    constexpr Outcome() noexcept = default;
    constexpr Outcome(Error error) noexcept : error_(error) {}

    [[nodiscard]] constexpr bool ok() const noexcept { return error_.status == Status::Ok; }
    constexpr explicit operator bool() const noexcept { return ok(); }

    [[nodiscard]] constexpr Status status() const noexcept { return error_.status; }
    [[nodiscard]] constexpr const Error& error() const noexcept { return error_; }

private:
    Error error_{Status::Ok, "", 0};
};

constexpr Outcome ok() noexcept
{
    return Outcome{};
}

constexpr Outcome fail(Status status, const char* context = "", std::int32_t code = 0) noexcept
{
    return Outcome{Error{status, context, code}};
}

template<typename T>
class [[nodiscard]] Result {
public:
    using value_type = T;

    constexpr Result(T value) noexcept(std::is_nothrow_move_constructible_v<T>) : has_value_(true)
    {
        ::new (static_cast<void*>(&storage_)) T(std::move(value));
    }

    constexpr Result(Error error) noexcept : error_(error), has_value_(false) {}

    Result(const Result& other) noexcept(std::is_nothrow_copy_constructible_v<T>)
        : error_(other.error_), has_value_(other.has_value_)
    {
        if (has_value_) {
            ::new (static_cast<void*>(&storage_)) T(other.value_ref());
        }
    }

    Result(Result&& other) noexcept(std::is_nothrow_move_constructible_v<T>)
        : error_(other.error_), has_value_(other.has_value_)
    {
        if (has_value_) {
            ::new (static_cast<void*>(&storage_)) T(std::move(other.value_ref()));
        }
    }

    Result& operator=(const Result& other) noexcept(std::is_nothrow_copy_constructible_v<T>)
    {
        if (this != &other) {
            destroy();
            error_ = other.error_;
            has_value_ = other.has_value_;
            if (has_value_) {
                ::new (static_cast<void*>(&storage_)) T(other.value_ref());
            }
        }
        return *this;
    }

    Result& operator=(Result&& other) noexcept(std::is_nothrow_move_constructible_v<T>)
    {
        if (this != &other) {
            destroy();
            error_ = other.error_;
            has_value_ = other.has_value_;
            if (has_value_) {
                ::new (static_cast<void*>(&storage_)) T(std::move(other.value_ref()));
            }
        }
        return *this;
    }

    ~Result() { destroy(); }

    [[nodiscard]] bool ok() const noexcept { return has_value_; }
    explicit operator bool() const noexcept { return has_value_; }

    [[nodiscard]] Status status() const noexcept { return has_value_ ? Status::Ok : error_.status; }
    [[nodiscard]] const Error& error() const noexcept { return error_; }

    [[nodiscard]] T& value() & noexcept
    {
        TL_ASSERT(has_value_);
        return value_ref();
    }
    [[nodiscard]] const T& value() const& noexcept
    {
        TL_ASSERT(has_value_);
        return value_ref();
    }
    [[nodiscard]] T&& value() && noexcept
    {
        TL_ASSERT(has_value_);
        return std::move(value_ref());
    }

    [[nodiscard]] T value_or(T fallback) const&
    {
        return has_value_ ? value_ref() : std::move(fallback);
    }

private:
    void destroy() noexcept
    {
        if (has_value_) {
            value_ref().~T();
            has_value_ = false;
        }
    }

    [[nodiscard]] T& value_ref() noexcept { return *std::launder(reinterpret_cast<T*>(&storage_)); }
    [[nodiscard]] const T& value_ref() const noexcept
    {
        return *std::launder(reinterpret_cast<const T*>(&storage_));
    }

    alignas(T) std::byte storage_[sizeof(T)]{};
    Error error_{Status::Ok, "", 0};
    bool has_value_ = false;
};
}  // namespace tl

#define TL_TRY(expr)                            \
    do {                                        \
        ::tl::Outcome tl_try_outcome_ = (expr); \
        if (!tl_try_outcome_.ok()) {            \
            return tl_try_outcome_;             \
        }                                       \
    } while (false)
