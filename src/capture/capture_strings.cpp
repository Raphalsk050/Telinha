#include "telinha/capture/capture_source.hpp"
#include "telinha/capture/capture_target.hpp"
#include "telinha/capture/pixel_format.hpp"

namespace tl::capture {

const char* to_string(PixelFormat format) noexcept
{
    switch (format) {
        case PixelFormat::Unknown: return "Unknown";
        case PixelFormat::B8G8R8A8Unorm: return "B8G8R8A8Unorm";
        case PixelFormat::R8G8B8A8Unorm: return "R8G8B8A8Unorm";
        case PixelFormat::R10G10B10A2Unorm: return "R10G10B10A2Unorm";
        case PixelFormat::R16G16B16A16Float: return "R16G16B16A16Float";
        case PixelFormat::NV12: return "NV12";
    }
    return "Unrecognized";
}

const char* to_string(SurfaceRotation rotation) noexcept
{
    switch (rotation) {
        case SurfaceRotation::None: return "None";
        case SurfaceRotation::Clockwise90: return "Clockwise90";
        case SurfaceRotation::Clockwise180: return "Clockwise180";
        case SurfaceRotation::Clockwise270: return "Clockwise270";
    }
    return "Unrecognized";
}

const char* to_string(CaptureTargetKind kind) noexcept
{
    switch (kind) {
        case CaptureTargetKind::None: return "None";
        case CaptureTargetKind::Monitor: return "Monitor";
        case CaptureTargetKind::Window: return "Window";
    }
    return "Unrecognized";
}

const char* to_string(CaptureBackend backend) noexcept
{
    switch (backend) {
        case CaptureBackend::Automatic: return "Automatic";
        case CaptureBackend::DesktopDuplication: return "DesktopDuplication";
        case CaptureBackend::GraphicsCapture: return "GraphicsCapture";
        case CaptureBackend::Synthetic: return "Synthetic";
    }
    return "Unrecognized";
}

}  // namespace tl::capture
