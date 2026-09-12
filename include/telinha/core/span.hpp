#pragma once

#include <cstddef>
#include <type_traits>

#include "telinha/core/config.hpp"

namespace tl {
template<typename T>
class Span {
public:
    constexpr Span() noexcept = default;
    constexpr Span(T* data, std::size_t size) noexcept : data_(data), size_(size) {}

    template<std::size_t N>
    constexpr Span(T (&array)[N]) noexcept : data_(array), size_(N)
    {}

    template<typename U, typename = std::enable_if_t<std::is_convertible_v<U (*)[], T (*)[]>>>
    constexpr Span(const Span<U>& other) noexcept : data_(other.data()), size_(other.size())
    {}

    [[nodiscard]] constexpr T* data() const noexcept { return data_; }
    [[nodiscard]] constexpr std::size_t size() const noexcept { return size_; }
    [[nodiscard]] constexpr std::size_t size_bytes() const noexcept { return size_ * sizeof(T); }
    [[nodiscard]] constexpr bool empty() const noexcept { return size_ == 0; }

    [[nodiscard]] constexpr T& operator[](std::size_t index) const noexcept
    {
        TL_ASSERT(index < size_);
        return data_[index];
    }

    [[nodiscard]] constexpr T* begin() const noexcept { return data_; }
    [[nodiscard]] constexpr T* end() const noexcept { return data_ + size_; }

    [[nodiscard]] constexpr Span subspan(std::size_t offset, std::size_t count) const noexcept
    {
        TL_ASSERT(offset + count <= size_);
        return Span(data_ + offset, count);
    }

    [[nodiscard]] constexpr Span first(std::size_t count) const noexcept
    {
        TL_ASSERT(count <= size_);
        return Span(data_, count);
    }

private:
    T* data_ = nullptr;
    std::size_t size_ = 0;
};
}  // namespace tl
