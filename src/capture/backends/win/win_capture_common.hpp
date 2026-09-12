#pragma once

#include <windows.h>

#include <d3d11.h>
#include <dxgi1_2.h>
#include <wrl/client.h>

#include <cstdint>

#include "telinha/capture/pixel_format.hpp"
#include "telinha/core/result.hpp"

namespace tl::capture::win {

template<typename T>
using ComPtr = Microsoft::WRL::ComPtr<T>;

[[nodiscard]] Status status_from_hresult(HRESULT hr) noexcept;

[[nodiscard]] Outcome outcome_from_hresult(HRESULT hr, const char* context) noexcept;

[[nodiscard]] PixelFormat pixel_format_from_dxgi(DXGI_FORMAT format) noexcept;

[[nodiscard]] DXGI_FORMAT dxgi_format_from_pixel_format(PixelFormat format) noexcept;

[[nodiscard]] bool utf16_to_utf8(const wchar_t* source, char* destination,
                                 std::uint32_t capacity) noexcept;

}  // namespace tl::capture::win
