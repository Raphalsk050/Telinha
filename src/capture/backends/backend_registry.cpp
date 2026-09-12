#include "telinha/capture/capture_source.hpp"

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

#endif

}  // namespace tl::capture
