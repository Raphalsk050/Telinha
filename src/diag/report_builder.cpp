#include "report_builder.hpp"

#include <cstring>

namespace tl::diag {
namespace {
constexpr char kHexDigits[] = "0123456789abcdef";
}

ReportBuilder::ReportBuilder(DiagnosticReport& report) noexcept : report_(report) {}

bool ReportBuilder::full() const noexcept
{
    return report_.item_count >= kMaxDiagnosticItems;
}

DiagItem* ReportBuilder::add(DiagCategory category, const char* name) noexcept
{
    if (full()) {
        return nullptr;
    }

    DiagItem& item = report_.items[report_.item_count];
    ++report_.item_count;

    item = DiagItem{};
    item.category = category;
    write_text(item.name, kDiagNameCapacity, name);
    return &item;
}

void ReportBuilder::add_ok(DiagCategory category, const char* name, const char* detail) noexcept
{
    DiagItem* item = add(category, name);
    if (item == nullptr) {
        return;
    }

    item->available = true;
    item->status = Status::Ok;
    write_text(item->detail, kDiagDetailCapacity, detail);
}

void ReportBuilder::add_failure(DiagCategory category, const char* name, const Error& error,
                                const char* detail) noexcept
{
    DiagItem* item = add(category, name);
    if (item == nullptr) {
        return;
    }

    item->available = false;
    item->status = error.status;
    item->platform_code = error.platform_code;

    const char* text = detail != nullptr && detail[0] != '\0' ? detail : error.context;
    write_text(item->detail, kDiagDetailCapacity, text);

    if (error.platform_code != 0) {
        append_text(item->detail, kDiagDetailCapacity, " (0x");
        append_hex32(item->detail, kDiagDetailCapacity,
                     static_cast<std::uint32_t>(error.platform_code));
        append_text(item->detail, kDiagDetailCapacity, ")");
    }
}

void write_text(char* destination, std::size_t capacity, const char* source) noexcept
{
    if (destination == nullptr || capacity == 0) {
        return;
    }

    destination[0] = '\0';
    if (source == nullptr) {
        return;
    }

    const std::size_t length = std::strlen(source);
    const std::size_t copied = length < capacity - 1 ? length : capacity - 1;
    std::memcpy(destination, source, copied);
    destination[copied] = '\0';
}

void append_text(char* destination, std::size_t capacity, const char* source) noexcept
{
    if (destination == nullptr || capacity == 0 || source == nullptr) {
        return;
    }

    const std::size_t used = std::strlen(destination);
    if (used + 1 >= capacity) {
        return;
    }

    const std::size_t room = capacity - used - 1;
    const std::size_t length = std::strlen(source);
    const std::size_t copied = length < room ? length : room;
    std::memcpy(destination + used, source, copied);
    destination[used + copied] = '\0';
}

void append_unsigned(char* destination, std::size_t capacity, std::uint64_t value) noexcept
{
    char digits[24] = {};
    std::size_t index = sizeof(digits);

    do {
        --index;
        digits[index] = static_cast<char>('0' + static_cast<char>(value % 10u));
        value /= 10u;
    } while (value != 0 && index > 0);

    char text[25] = {};
    const std::size_t length = sizeof(digits) - index;
    std::memcpy(text, digits + index, length);
    text[length] = '\0';
    append_text(destination, capacity, text);
}

void append_hex32(char* destination, std::size_t capacity, std::uint32_t value) noexcept
{
    char text[9] = {};
    for (std::size_t i = 0; i < 8; ++i) {
        const std::uint32_t shift = static_cast<std::uint32_t>((7 - i) * 4);
        text[i] = kHexDigits[(value >> shift) & 0xFu];
    }
    text[8] = '\0';
    append_text(destination, capacity, text);
}

void format_detail(char* destination, std::size_t capacity, const char* prefix,
                   std::uint64_t value) noexcept
{
    write_text(destination, capacity, prefix);
    append_unsigned(destination, capacity, value);
}
}  // namespace tl::diag
