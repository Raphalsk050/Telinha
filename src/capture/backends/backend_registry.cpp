#include "telinha/capture/capture_source.hpp"

#if defined(TELINHA_HAS_PLATFORM_BACKENDS)

#include <cstring>
#include <new>

#include "synthetic_source.hpp"

#if TL_PLATFORM_WINDOWS
#include "win/desktop_duplication_source.hpp"
#include "win/graphics_capture_source.hpp"
#include "win/media_foundation_source.hpp"
#include "win/target_enumeration.hpp"
#endif

#endif

namespace tl::capture {

#if !defined(TELINHA_HAS_PLATFORM_BACKENDS)

bool backend_available(CaptureBackend) noexcept
{
    return false;
}

Outcome enumerate_targets(CaptureTargetKind, Span<CaptureTargetInfo>, std::uint32_t& written,
                          std::uint32_t& available) noexcept
{
    written = 0;
    available = 0;
    return fail(Status::NotImplemented, "enumerate_targets: no backend linked");
}

Result<std::unique_ptr<CaptureSource>> create_capture_source(const CaptureTarget&,
                                                             const CaptureOptions&)
{
    return Error{Status::NotImplemented, "create_capture_source: no backend linked"};
}

#else

namespace {

[[nodiscard]] CaptureBackend resolve_backend(CaptureBackend requested,
                                             CaptureTargetKind kind) noexcept
{
    if (requested != CaptureBackend::Automatic) {
        return requested;
    }
#if TL_PLATFORM_WINDOWS
    if (kind == CaptureTargetKind::Device) {
        return CaptureBackend::MediaFoundation;
    }
    return kind == CaptureTargetKind::Window ? CaptureBackend::GraphicsCapture
                                             : CaptureBackend::DesktopDuplication;
#else
    (void)kind;
    return CaptureBackend::Synthetic;
#endif
}

[[nodiscard]] Result<std::unique_ptr<CaptureSource>> make_synthetic(
    const CaptureTarget& target, const CaptureOptions& options) noexcept
{
    auto* source = new (std::nothrow) SyntheticSource(target, options);
    if (source == nullptr) {
        return Error{Status::OutOfMemory, "SyntheticSource"};
    }
    return std::unique_ptr<CaptureSource>(source);
}

#if !TL_PLATFORM_WINDOWS

[[nodiscard]] Outcome enumerate_synthetic_targets(CaptureTargetKind kind,
                                                  Span<CaptureTargetInfo> out,
                                                  std::uint32_t& written,
                                                  std::uint32_t& available) noexcept
{
    written = 0;
    available = 0;
    if (kind != CaptureTargetKind::Monitor) {
        return ok();
    }

    available = 1;
    if (out.empty()) {
        return fail(Status::OutOfRange, "enumerate_targets: buffer too small");
    }

    CaptureTargetInfo info;
    info.target = CaptureTarget::monitor(1);
    info.width = CaptureOptions{}.synthetic_width;
    info.height = CaptureOptions{}.synthetic_height;
    info.refresh_millihertz = CaptureOptions{}.synthetic_refresh_millihertz;
    info.primary = true;
    static constexpr char kSyntheticName[] = "synthetic";
    static_assert(sizeof(kSyntheticName) <= kTargetNameCapacity);
    std::memcpy(info.name, kSyntheticName, sizeof(kSyntheticName));

    out[0] = info;
    written = 1;
    return ok();
}

#endif

}  // namespace

bool backend_available(CaptureBackend backend) noexcept
{
    switch (backend) {
        case CaptureBackend::Synthetic: return true;
        case CaptureBackend::Automatic: return true;
#if TL_PLATFORM_WINDOWS
        case CaptureBackend::DesktopDuplication: return true;
        case CaptureBackend::GraphicsCapture: return win::graphics_capture_supported();
        case CaptureBackend::MediaFoundation: return true;
#else
        case CaptureBackend::DesktopDuplication:
        case CaptureBackend::GraphicsCapture:
        case CaptureBackend::MediaFoundation: return false;
#endif
    }
    return false;
}

Outcome enumerate_targets(CaptureTargetKind kind, Span<CaptureTargetInfo> out,
                          std::uint32_t& written, std::uint32_t& available) noexcept
{
#if TL_PLATFORM_WINDOWS
    return win::enumerate_targets(kind, out, written, available);
#else
    return enumerate_synthetic_targets(kind, out, written, available);
#endif
}

Result<std::unique_ptr<CaptureSource>> create_capture_source(const CaptureTarget& target,
                                                             const CaptureOptions& options)
{
    if (!target.valid()) {
        return Error{Status::InvalidArgument, "create_capture_source: invalid target"};
    }

    const CaptureBackend backend = resolve_backend(options.backend, target.kind);
    switch (backend) {
        case CaptureBackend::Synthetic: return make_synthetic(target, options);

#if TL_PLATFORM_WINDOWS
        case CaptureBackend::DesktopDuplication: {
            if (target.kind != CaptureTargetKind::Monitor) {
                return Error{Status::InvalidArgument, "desktop duplication only captures monitors"};
            }
            auto* source = new (std::nothrow) win::DesktopDuplicationSource(target, options);
            if (source == nullptr) {
                return Error{Status::OutOfMemory, "DesktopDuplicationSource"};
            }
            return std::unique_ptr<CaptureSource>(source);
        }

        case CaptureBackend::GraphicsCapture:
            if (target.kind != CaptureTargetKind::Window) {
                return Error{Status::InvalidArgument, "graphics capture only captures windows"};
            }
            if (!win::graphics_capture_supported()) {
                return Error{Status::NotSupported, "Windows Graphics Capture is unavailable"};
            }
            return win::create_graphics_capture_source(target, options);

        case CaptureBackend::MediaFoundation:
            if (target.kind != CaptureTargetKind::Device) {
                return Error{Status::InvalidArgument, "media foundation only captures devices"};
            }
            return win::create_media_foundation_source(target, options);
#else
        case CaptureBackend::DesktopDuplication:
        case CaptureBackend::GraphicsCapture:
        case CaptureBackend::MediaFoundation:
            return Error{Status::NotSupported, "backend requires Windows"};
#endif

        case CaptureBackend::Automatic: break;
    }

    return Error{Status::NotSupported, "create_capture_source: unknown backend"};
}

#endif

}  // namespace tl::capture
