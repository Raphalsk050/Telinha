#pragma once

#include <cstddef>

#include "telinha/core/result.hpp"
#include "telinha/core/span.hpp"

namespace tl::app {

[[nodiscard]] bool clipboard_available() noexcept;

[[nodiscard]] Outcome clipboard_write(Span<const char> text) noexcept;

[[nodiscard]] Outcome clipboard_read(char* out, std::size_t capacity, std::size_t& length) noexcept;

}  // namespace tl::app
