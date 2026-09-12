#pragma once

#include "report_builder.hpp"

namespace tl::diag {
void probe_system(ReportBuilder& builder) noexcept;
void probe_graphics(ReportBuilder& builder) noexcept;
void probe_audio(ReportBuilder& builder) noexcept;
}  // namespace tl::diag
