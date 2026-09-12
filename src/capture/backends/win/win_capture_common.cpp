#include "win_capture_common.hpp"

namespace tl::capture::win {

Status status_from_hresult(HRESULT hr) noexcept
{
    if (SUCCEEDED(hr)) {
        return Status::Ok;
    }

    switch (hr) {
        case DXGI_ERROR_WAIT_TIMEOUT: return Status::Timeout;
        case DXGI_ERROR_ACCESS_LOST:
        case DXGI_ERROR_DEVICE_REMOVED:
        case DXGI_ERROR_DEVICE_RESET:
        case DXGI_ERROR_DEVICE_HUNG:
        case DXGI_ERROR_DRIVER_INTERNAL_ERROR: return Status::DeviceLost;
        case DXGI_ERROR_ACCESS_DENIED:
        case E_ACCESSDENIED: return Status::PermissionDenied;
        case DXGI_ERROR_SESSION_DISCONNECTED:
        case DXGI_ERROR_NOT_CURRENTLY_AVAILABLE: return Status::Unavailable;
        case DXGI_ERROR_NOT_FOUND: return Status::NotFound;
        case DXGI_ERROR_UNSUPPORTED:
        case E_NOINTERFACE: return Status::NotSupported;
        case DXGI_ERROR_MORE_DATA: return Status::OutOfRange;
        case E_OUTOFMEMORY: return Status::OutOfMemory;
        case E_INVALIDARG: return Status::InvalidArgument;
        case E_NOTIMPL: return Status::NotImplemented;
        default: break;
    }
    return Status::PlatformError;
}

Outcome outcome_from_hresult(HRESULT hr, const char* context) noexcept
{
    if (SUCCEEDED(hr)) {
        return ok();
    }
    return fail(status_from_hresult(hr), context, static_cast<std::int32_t>(hr));
}

PixelFormat pixel_format_from_dxgi(DXGI_FORMAT format) noexcept
{
    switch (format) {
        case DXGI_FORMAT_B8G8R8A8_UNORM:
        case DXGI_FORMAT_B8G8R8A8_UNORM_SRGB: return PixelFormat::B8G8R8A8Unorm;
        case DXGI_FORMAT_R8G8B8A8_UNORM:
        case DXGI_FORMAT_R8G8B8A8_UNORM_SRGB: return PixelFormat::R8G8B8A8Unorm;
        case DXGI_FORMAT_R10G10B10A2_UNORM: return PixelFormat::R10G10B10A2Unorm;
        case DXGI_FORMAT_R16G16B16A16_FLOAT: return PixelFormat::R16G16B16A16Float;
        case DXGI_FORMAT_NV12: return PixelFormat::NV12;
        default: break;
    }
    return PixelFormat::Unknown;
}

DXGI_FORMAT dxgi_format_from_pixel_format(PixelFormat format) noexcept
{
    switch (format) {
        case PixelFormat::B8G8R8A8Unorm: return DXGI_FORMAT_B8G8R8A8_UNORM;
        case PixelFormat::R8G8B8A8Unorm: return DXGI_FORMAT_R8G8B8A8_UNORM;
        case PixelFormat::R10G10B10A2Unorm: return DXGI_FORMAT_R10G10B10A2_UNORM;
        case PixelFormat::R16G16B16A16Float: return DXGI_FORMAT_R16G16B16A16_FLOAT;
        case PixelFormat::NV12: return DXGI_FORMAT_NV12;
        case PixelFormat::Unknown: break;
    }
    return DXGI_FORMAT_UNKNOWN;
}

bool utf16_to_utf8(const wchar_t* source, char* destination, std::uint32_t capacity) noexcept
{
    if (destination == nullptr || capacity == 0) {
        return false;
    }
    destination[0] = '\0';
    if (source == nullptr || source[0] == L'\0') {
        return true;
    }

    const int needed = WideCharToMultiByte(CP_UTF8, 0, source, -1, nullptr, 0, nullptr, nullptr);
    if (needed <= 0) {
        return false;
    }
    if (static_cast<std::uint32_t>(needed) <= capacity) {
        return WideCharToMultiByte(CP_UTF8, 0, source, -1, destination, static_cast<int>(capacity),
                                   nullptr, nullptr) > 0;
    }

    const int budget = static_cast<int>(capacity) - 1;
    int low = 0;
    int high = lstrlenW(source);
    while (low < high) {
        const int middle = low + (high - low + 1) / 2;
        const int length =
            WideCharToMultiByte(CP_UTF8, 0, source, middle, nullptr, 0, nullptr, nullptr);
        if (length > 0 && length <= budget) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    if (low == 0) {
        return true;
    }

    const int written =
        WideCharToMultiByte(CP_UTF8, 0, source, low, destination, budget, nullptr, nullptr);
    if (written <= 0) {
        destination[0] = '\0';
        return false;
    }
    destination[written] = '\0';
    return true;
}

}  // namespace tl::capture::win
