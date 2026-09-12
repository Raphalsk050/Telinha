#pragma once

#include <cstdint>

#include "telinha/core/config.hpp"

namespace tl {
inline constexpr std::uint32_t kInvalidIndex = 0xFFFFFFFFu;

template<typename Tag>
struct Handle {
    std::uint32_t index = kInvalidIndex;
    std::uint32_t generation = 0;

    [[nodiscard]] constexpr bool valid() const noexcept { return index != kInvalidIndex; }
    constexpr explicit operator bool() const noexcept { return valid(); }

    friend constexpr bool operator==(const Handle& a, const Handle& b) noexcept
    {
        return a.index == b.index && a.generation == b.generation;
    }
    friend constexpr bool operator!=(const Handle& a, const Handle& b) noexcept
    {
        return !(a == b);
    }
};
}  // namespace tl
