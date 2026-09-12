#pragma once

#include "report_builder.hpp"

namespace tl::diag {
void probe_capture(ReportBuilder& builder) noexcept;
void probe_encode(ReportBuilder& builder) noexcept;
}  // namespace tl::diag
