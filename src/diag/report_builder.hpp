#pragma once

#include "telinha/diag/diagnostics.hpp"

namespace tl::diag {
class ReportBuilder {
public:
    explicit ReportBuilder(DiagnosticReport& report) noexcept;

    DiagItem* add(DiagCategory category, const char* name) noexcept;

    void add_ok(DiagCategory category, const char* name, const char* detail) noexcept;
    void add_failure(DiagCategory category, const char* name, const Error& error,
                     const char* detail) noexcept;

    [[nodiscard]] bool full() const noexcept;

private:
    DiagnosticReport& report_;
};

void write_text(char* destination, std::size_t capacity, const char* source) noexcept;

void format_detail(char* destination, std::size_t capacity, const char* prefix,
                   std::uint64_t value) noexcept;

void append_text(char* destination, std::size_t capacity, const char* source) noexcept;

void append_unsigned(char* destination, std::size_t capacity, std::uint64_t value) noexcept;

void append_hex32(char* destination, std::size_t capacity, std::uint32_t value) noexcept;
}  // namespace tl::diag
