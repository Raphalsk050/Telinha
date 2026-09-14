#include "target_enumeration.hpp"

#include <dwmapi.h>

#include "d3d11_capture_device.hpp"
#include "media_foundation_source.hpp"

namespace tl::capture::win {
namespace {

inline constexpr std::uint32_t kMaxDisplayPaths = 64;
inline constexpr std::uint32_t kMaxDisplayModes = 128;

struct EnumerationSink {
    Span<CaptureTargetInfo> out;
    std::uint32_t written = 0;
    std::uint32_t available = 0;
};

void emit(EnumerationSink& sink, const CaptureTargetInfo& info) noexcept
{
    ++sink.available;
    if (sink.written < sink.out.size()) {
        sink.out[sink.written] = info;
        ++sink.written;
    }
}

[[nodiscard]] std::uint32_t refresh_from_display_config(const wchar_t* gdi_device_name,
                                                        wchar_t* friendly_name,
                                                        std::uint32_t friendly_capacity) noexcept
{
    UINT32 path_count = 0;
    UINT32 mode_count = 0;
    if (GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &path_count, &mode_count) !=
        ERROR_SUCCESS) {
        return 0;
    }
    if (path_count == 0 || path_count > kMaxDisplayPaths || mode_count > kMaxDisplayModes) {
        return 0;
    }

    DISPLAYCONFIG_PATH_INFO paths[kMaxDisplayPaths] = {};
    DISPLAYCONFIG_MODE_INFO modes[kMaxDisplayModes] = {};
    if (QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &path_count, paths, &mode_count, modes,
                           nullptr) != ERROR_SUCCESS) {
        return 0;
    }

    for (UINT32 i = 0; i < path_count; ++i) {
        DISPLAYCONFIG_SOURCE_DEVICE_NAME source = {};
        source.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source.header.size = sizeof(source);
        source.header.adapterId = paths[i].sourceInfo.adapterId;
        source.header.id = paths[i].sourceInfo.id;
        if (DisplayConfigGetDeviceInfo(&source.header) != ERROR_SUCCESS) {
            continue;
        }
        if (lstrcmpiW(source.viewGdiDeviceName, gdi_device_name) != 0) {
            continue;
        }

        if (friendly_name != nullptr && friendly_capacity != 0) {
            DISPLAYCONFIG_TARGET_DEVICE_NAME target = {};
            target.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = sizeof(target);
            target.header.adapterId = paths[i].targetInfo.adapterId;
            target.header.id = paths[i].targetInfo.id;
            if (DisplayConfigGetDeviceInfo(&target.header) == ERROR_SUCCESS &&
                target.monitorFriendlyDeviceName[0] != L'\0') {
                lstrcpynW(friendly_name, target.monitorFriendlyDeviceName,
                          static_cast<int>(friendly_capacity));
            }
        }

        const DISPLAYCONFIG_RATIONAL rate = paths[i].targetInfo.refreshRate;
        if (rate.Denominator != 0) {
            const std::uint64_t millihertz =
                (static_cast<std::uint64_t>(rate.Numerator) * 1000ull) / rate.Denominator;
            return static_cast<std::uint32_t>(millihertz);
        }
        return 0;
    }
    return 0;
}

[[nodiscard]] std::uint32_t refresh_from_display_settings(const wchar_t* gdi_device_name) noexcept
{
    DEVMODEW mode = {};
    mode.dmSize = sizeof(mode);
    if (EnumDisplaySettingsW(gdi_device_name, ENUM_CURRENT_SETTINGS, &mode) == 0) {
        return 0;
    }
    return mode.dmDisplayFrequency * 1000u;
}

[[nodiscard]] SurfaceRotation rotation_for_monitor(HMONITOR monitor) noexcept
{
    ComPtr<IDXGIAdapter1> adapter;
    ComPtr<IDXGIOutput1> output;
    if (!find_output_for_monitor(monitor, adapter, output).ok()) {
        return SurfaceRotation::None;
    }

    DXGI_OUTPUT_DESC description = {};
    if (FAILED(output->GetDesc(&description))) {
        return SurfaceRotation::None;
    }
    return rotation_from_dxgi(description.Rotation);
}

inline constexpr std::uint32_t kClassNameCapacity = 64;

BOOL CALLBACK core_window_visitor(HWND child, LPARAM context) noexcept
{
    wchar_t class_name[kClassNameCapacity] = {};
    if (GetClassNameW(child, class_name, static_cast<int>(kClassNameCapacity)) == 0) {
        return TRUE;
    }
    if (lstrcmpiW(class_name, L"Windows.UI.Core.CoreWindow") != 0) {
        return TRUE;
    }
    *reinterpret_cast<HWND*>(context) = child;
    return FALSE;
}

[[nodiscard]] HWND audio_owner_window(HWND window) noexcept
{
    wchar_t class_name[kClassNameCapacity] = {};
    if (GetClassNameW(window, class_name, static_cast<int>(kClassNameCapacity)) == 0) {
        return window;
    }
    if (lstrcmpiW(class_name, L"ApplicationFrameWindow") != 0) {
        return window;
    }

    HWND core_window = nullptr;
    EnumChildWindows(window, &core_window_visitor, reinterpret_cast<LPARAM>(&core_window));
    return core_window != nullptr ? core_window : window;
}

BOOL CALLBACK monitor_visitor(HMONITOR monitor, HDC, LPRECT, LPARAM context) noexcept
{
    EnumerationSink& sink = *reinterpret_cast<EnumerationSink*>(context);
    CaptureTargetInfo info;
    if (describe_monitor(monitor, info).ok()) {
        emit(sink, info);
    }
    return TRUE;
}

BOOL CALLBACK window_visitor(HWND window, LPARAM context) noexcept
{
    EnumerationSink& sink = *reinterpret_cast<EnumerationSink*>(context);
    if (!window_is_offerable(window)) {
        return TRUE;
    }
    CaptureTargetInfo info;
    if (describe_window(window, info).ok()) {
        emit(sink, info);
    }
    return TRUE;
}

}  // namespace

SurfaceRotation rotation_from_dxgi(DXGI_MODE_ROTATION rotation) noexcept
{
    switch (rotation) {
        case DXGI_MODE_ROTATION_ROTATE90: return SurfaceRotation::Clockwise90;
        case DXGI_MODE_ROTATION_ROTATE180: return SurfaceRotation::Clockwise180;
        case DXGI_MODE_ROTATION_ROTATE270: return SurfaceRotation::Clockwise270;
        case DXGI_MODE_ROTATION_IDENTITY:
        case DXGI_MODE_ROTATION_UNSPECIFIED:
        default: break;
    }
    return SurfaceRotation::None;
}

Outcome describe_monitor(HMONITOR monitor, CaptureTargetInfo& out) noexcept
{
    MONITORINFOEXW monitor_info = {};
    monitor_info.cbSize = sizeof(monitor_info);
    if (GetMonitorInfoW(monitor, &monitor_info) == 0) {
        return fail(Status::NotFound, "GetMonitorInfoW");
    }

    out = CaptureTargetInfo{};
    out.target = CaptureTarget::monitor(reinterpret_cast<std::uint64_t>(monitor));
    out.x = monitor_info.rcMonitor.left;
    out.y = monitor_info.rcMonitor.top;
    out.width =
        static_cast<std::uint32_t>(monitor_info.rcMonitor.right - monitor_info.rcMonitor.left);
    out.height =
        static_cast<std::uint32_t>(monitor_info.rcMonitor.bottom - monitor_info.rcMonitor.top);
    out.primary = (monitor_info.dwFlags & MONITORINFOF_PRIMARY) != 0;
    out.rotation = rotation_for_monitor(monitor);

    wchar_t friendly[kTargetNameCapacity] = {};
    out.refresh_millihertz =
        refresh_from_display_config(monitor_info.szDevice, friendly, kTargetNameCapacity);
    if (out.refresh_millihertz == 0) {
        out.refresh_millihertz = refresh_from_display_settings(monitor_info.szDevice);
    }

    const wchar_t* name = friendly[0] != L'\0' ? friendly : monitor_info.szDevice;
    (void)utf16_to_utf8(name, out.name, kTargetNameCapacity);
    return ok();
}

bool window_is_offerable(HWND window) noexcept
{
    if (window == nullptr || IsWindow(window) == 0) {
        return false;
    }
    if (IsWindowVisible(window) == 0 || IsIconic(window) != 0) {
        return false;
    }
    if (GetAncestor(window, GA_ROOT) != window) {
        return false;
    }
    if (GetWindow(window, GW_OWNER) != nullptr) {
        return false;
    }

    const LONG_PTR extended_style = GetWindowLongPtrW(window, GWL_EXSTYLE);
    if ((extended_style & WS_EX_TOOLWINDOW) != 0) {
        return false;
    }
    if (GetWindowTextLengthW(window) == 0) {
        return false;
    }

    BOOL cloaked = FALSE;
    if (SUCCEEDED(DwmGetWindowAttribute(window, DWMWA_CLOAKED, &cloaked, sizeof(cloaked))) &&
        cloaked != FALSE) {
        return false;
    }

    RECT bounds = {};
    if (GetWindowRect(window, &bounds) == 0) {
        return false;
    }
    return bounds.right > bounds.left && bounds.bottom > bounds.top;
}

Outcome describe_window(HWND window, CaptureTargetInfo& out) noexcept
{
    if (window == nullptr || IsWindow(window) == 0) {
        return fail(Status::TargetGone, "describe_window");
    }

    RECT bounds = {};
    if (FAILED(
            DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, &bounds, sizeof(bounds))) ||
        bounds.right <= bounds.left || bounds.bottom <= bounds.top) {
        if (GetWindowRect(window, &bounds) == 0) {
            return fail(Status::NotFound, "GetWindowRect");
        }
    }

    out = CaptureTargetInfo{};
    out.target = CaptureTarget::window(reinterpret_cast<std::uint64_t>(window));
    out.x = bounds.left;
    out.y = bounds.top;
    out.width = static_cast<std::uint32_t>(bounds.right - bounds.left);
    out.height = static_cast<std::uint32_t>(bounds.bottom - bounds.top);

    DWORD process_id = 0;
    GetWindowThreadProcessId(audio_owner_window(window), &process_id);
    out.process_id = process_id;

    wchar_t title[kTargetNameCapacity] = {};
    GetWindowTextW(window, title, static_cast<int>(kTargetNameCapacity));
    (void)utf16_to_utf8(title, out.name, kTargetNameCapacity);
    return ok();
}

Outcome enumerate_targets(CaptureTargetKind kind, Span<CaptureTargetInfo> out,
                          std::uint32_t& written, std::uint32_t& available) noexcept
{
    written = 0;
    available = 0;

    EnumerationSink sink;
    sink.out = out;

    switch (kind) {
        case CaptureTargetKind::Monitor:
            if (EnumDisplayMonitors(nullptr, nullptr, &monitor_visitor,
                                    reinterpret_cast<LPARAM>(&sink)) == 0) {
                return fail(Status::Unavailable, "EnumDisplayMonitors");
            }
            break;
        case CaptureTargetKind::Window:
            if (EnumWindows(&window_visitor, reinterpret_cast<LPARAM>(&sink)) == 0 &&
                GetLastError() != ERROR_SUCCESS) {
                return fail(Status::Unavailable, "EnumWindows");
            }
            break;
        case CaptureTargetKind::Device: return enumerate_capture_devices(out, written, available);
        case CaptureTargetKind::None: return fail(Status::InvalidArgument, "target kind");
    }

    written = sink.written;
    available = sink.available;
    if (sink.available > sink.written) {
        return fail(Status::OutOfRange, "enumerate_targets: buffer too small");
    }
    return ok();
}

}  // namespace tl::capture::win
