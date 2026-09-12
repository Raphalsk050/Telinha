#include "telinha/app/clipboard.hpp"

#include <cstring>

#if TL_PLATFORM_WINDOWS
#include <windows.h>
#endif

namespace tl::app {
namespace {

#if TL_PLATFORM_WINDOWS

constexpr int kOpenAttempts = 8;
constexpr DWORD kOpenBackoffMs = 20;

class ClipboardLock {
public:
    ClipboardLock() noexcept
    {
        for (int attempt = 0; attempt < kOpenAttempts; ++attempt) {
            if (OpenClipboard(nullptr) != 0) {
                held_ = true;
                return;
            }
            Sleep(kOpenBackoffMs);
        }
    }

    ClipboardLock(const ClipboardLock&) = delete;
    ClipboardLock& operator=(const ClipboardLock&) = delete;

    ~ClipboardLock()
    {
        if (held_) {
            CloseClipboard();
        }
    }

    [[nodiscard]] bool held() const noexcept { return held_; }

private:
    bool held_ = false;
};

#endif

}  // namespace

bool clipboard_available() noexcept
{
    return TL_PLATFORM_WINDOWS != 0;
}

#if TL_PLATFORM_WINDOWS

Outcome clipboard_write(Span<const char> text) noexcept
{
    if (text.empty()) {
        return fail(Status::InvalidArgument, "clipboard_write");
    }

    const int wide_length =
        MultiByteToWideChar(CP_UTF8, 0, text.data(), static_cast<int>(text.size()), nullptr, 0);
    if (wide_length <= 0) {
        return fail(Status::InvalidArgument, "clipboard_write: MultiByteToWideChar");
    }

    const ClipboardLock lock;
    if (!lock.held()) {
        return fail(Status::Unavailable, "clipboard_write: OpenClipboard");
    }

    if (EmptyClipboard() == 0) {
        return fail(Status::PlatformError, "clipboard_write: EmptyClipboard",
                    static_cast<std::int32_t>(GetLastError()));
    }

    const SIZE_T bytes = (static_cast<SIZE_T>(wide_length) + 1) * sizeof(wchar_t);
    const HGLOBAL block = GlobalAlloc(GMEM_MOVEABLE, bytes);
    if (block == nullptr) {
        return fail(Status::OutOfMemory, "clipboard_write: GlobalAlloc");
    }

    auto* destination = static_cast<wchar_t*>(GlobalLock(block));
    if (destination == nullptr) {
        GlobalFree(block);
        return fail(Status::PlatformError, "clipboard_write: GlobalLock");
    }

    MultiByteToWideChar(CP_UTF8, 0, text.data(), static_cast<int>(text.size()), destination,
                        wide_length);
    destination[wide_length] = L'\0';
    GlobalUnlock(block);

    if (SetClipboardData(CF_UNICODETEXT, block) == nullptr) {
        GlobalFree(block);
        return fail(Status::PlatformError, "clipboard_write: SetClipboardData",
                    static_cast<std::int32_t>(GetLastError()));
    }

    return ok();
}

Outcome clipboard_read(char* out, std::size_t capacity, std::size_t& length) noexcept
{
    length = 0;
    if (out == nullptr || capacity == 0) {
        return fail(Status::InvalidArgument, "clipboard_read");
    }
    out[0] = '\0';

    const ClipboardLock lock;
    if (!lock.held()) {
        return fail(Status::Unavailable, "clipboard_read: OpenClipboard");
    }

    const HANDLE block = GetClipboardData(CF_UNICODETEXT);
    if (block == nullptr) {
        return fail(Status::Empty, "clipboard_read: sem texto");
    }

    const auto* source = static_cast<const wchar_t*>(GlobalLock(block));
    if (source == nullptr) {
        return fail(Status::PlatformError, "clipboard_read: GlobalLock");
    }

    const int written = WideCharToMultiByte(CP_UTF8, 0, source, -1, out, static_cast<int>(capacity),
                                            nullptr, nullptr);
    GlobalUnlock(block);

    if (written <= 0) {
        return fail(Status::OutOfRange, "clipboard_read: texto longo demais");
    }

    length = static_cast<std::size_t>(written) - 1;
    return ok();
}

#else

Outcome clipboard_write(Span<const char> text) noexcept
{
    (void)text;
    return fail(Status::NotSupported, "clipboard_write");
}

Outcome clipboard_read(char* out, std::size_t capacity, std::size_t& length) noexcept
{
    (void)capacity;
    length = 0;
    if (out != nullptr && capacity != 0) {
        out[0] = '\0';
    }
    return fail(Status::NotSupported, "clipboard_read");
}

#endif

}  // namespace tl::app
