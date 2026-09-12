#include "platform_probes.hpp"

namespace tl::diag {
namespace {
constexpr Error unsupported_host() noexcept
{
    return Error{Status::NotSupported, "telinha diagnostics only inspect this on windows"};
}
}  // namespace

void probe_system(ReportBuilder& builder) noexcept
{
    builder.add_failure(DiagCategory::System, "windows version", unsupported_host(), nullptr);
    builder.add_failure(DiagCategory::System, "per process loopback", unsupported_host(), nullptr);
}

void probe_graphics(ReportBuilder& builder) noexcept
{
    builder.add_failure(DiagCategory::Graphics, "adapters", unsupported_host(), nullptr);
}

void probe_audio(ReportBuilder& builder) noexcept
{
    builder.add_failure(DiagCategory::Audio, "default render device", unsupported_host(), nullptr);
}
}  // namespace tl::diag
