#include "window_capture_source.hpp"

#include <new>

#include "desktop_duplication_source.hpp"
#include "graphics_capture_source.hpp"
#include "telinha/core/clock.hpp"
#include "telinha/core/log.hpp"
#include "win_capture_common.hpp"

#include <shellapi.h>

namespace tl::capture::win {
namespace {

inline constexpr Nanoseconds kShellStateLifetimeNs = 250ull * kNanosecondsPerMillisecond;
inline constexpr Nanoseconds kScreenRetryNs = 1000ull * kNanosecondsPerMillisecond;

// An exclusive fullscreen window bypasses the compositor, so window capture stops receiving
// frames. While such a window is in front, its picture comes from the monitor it owns.
class WindowCaptureSource final : public CaptureSource {
public:
    WindowCaptureSource(const CaptureTarget& target, const CaptureOptions& options,
                        std::unique_ptr<CaptureSource> windowed) noexcept
        : target_(target),
          options_(options),
          window_(reinterpret_cast<HWND>(static_cast<std::uintptr_t>(target.handle))),
          windowed_(std::move(windowed))
    {}

    ~WindowCaptureSource() override { stop(); }

    Outcome start() noexcept override
    {
        if (started_) {
            return ok();
        }
        TL_TRY(windowed_->start());
        started_ = true;
        return ok();
    }

    void stop() noexcept override
    {
        leave_screen();
        windowed_->stop();
        started_ = false;
    }

    [[nodiscard]] Outcome acquire(CapturedFrame& out, std::uint32_t timeout_ms) noexcept override
    {
        if (!started_) {
            return fail(Status::Unavailable, "window capture not started");
        }

        HMONITOR monitor = nullptr;
        const bool exclusive = exclusive_fullscreen(monitor);
        if (screen_ != nullptr && (!exclusive || monitor != screen_monitor_)) {
            leave_screen();
            return fail(Status::ConfigurationChanged, "window left exclusive fullscreen");
        }
        if (screen_ == nullptr && exclusive && enter_screen(monitor)) {
            return fail(Status::ConfigurationChanged, "window entered exclusive fullscreen");
        }
        if (screen_ == nullptr) {
            return windowed_->acquire(out, timeout_ms);
        }

        const Outcome acquired = screen_->acquire(out, timeout_ms);
        if (acquired.ok() && !exclusive_fullscreen(monitor)) {
            // The window lost the monitor while waiting, so this frame may show the desktop.
            screen_->release();
            return fail(Status::Timeout, "window left exclusive fullscreen");
        }
        return acquired;
    }

    void release() noexcept override { active().release(); }

    [[nodiscard]] Outcome map_for_readback(FrameSurface& out) noexcept override
    {
        return active().map_for_readback(out);
    }

    void unmap_readback() noexcept override { active().unmap_readback(); }

    [[nodiscard]] CaptureSourceInfo info() const noexcept override
    {
        CaptureSourceInfo out = active().info();
        out.target = target_;
        out.process_id = windowed_->info().process_id;
        return out;
    }

private:
    [[nodiscard]] CaptureSource& active() const noexcept
    {
        return screen_ != nullptr ? *screen_ : *windowed_;
    }

    [[nodiscard]] bool shell_reports_exclusive() noexcept
    {
        const Nanoseconds now = now_ns();
        if (shell_checked_ns_ == 0 || now - shell_checked_ns_ >= kShellStateLifetimeNs) {
            QUERY_USER_NOTIFICATION_STATE state = QUNS_ACCEPTS_NOTIFICATIONS;
            shell_exclusive_ = SUCCEEDED(SHQueryUserNotificationState(&state)) &&
                               state == QUNS_RUNNING_D3D_FULL_SCREEN;
            shell_checked_ns_ = now;
        }
        return shell_exclusive_;
    }

    [[nodiscard]] bool exclusive_fullscreen(HMONITOR& monitor) noexcept
    {
        monitor = nullptr;
        if (IsIconic(window_) != 0) {
            return false;
        }
        const HWND foreground = GetForegroundWindow();
        if (foreground == nullptr || GetAncestor(foreground, GA_ROOTOWNER) != window_) {
            return false;
        }

        const HMONITOR owner = MonitorFromWindow(window_, MONITOR_DEFAULTTONULL);
        RECT bounds = {};
        MONITORINFO monitor_info = {};
        monitor_info.cbSize = sizeof(monitor_info);
        if (owner == nullptr || GetWindowRect(window_, &bounds) == 0 ||
            GetMonitorInfoW(owner, &monitor_info) == 0) {
            return false;
        }
        const RECT& screen = monitor_info.rcMonitor;
        if (bounds.left > screen.left || bounds.top > screen.top || bounds.right < screen.right ||
            bounds.bottom < screen.bottom) {
            return false;
        }
        if (!shell_reports_exclusive()) {
            return false;
        }
        monitor = owner;
        return true;
    }

    [[nodiscard]] bool enter_screen(HMONITOR monitor) noexcept
    {
        const Nanoseconds now = now_ns();
        if (now < screen_retry_ns_) {
            return false;
        }

        std::unique_ptr<CaptureSource> source(new (std::nothrow) DesktopDuplicationSource(
            CaptureTarget::monitor(reinterpret_cast<std::uint64_t>(monitor)), options_));
        Outcome started = fail(Status::OutOfMemory, "DesktopDuplicationSource");
        if (source) {
            started = source->start();
        }
        if (!started.ok()) {
            // The mode switch into fullscreen refuses duplication for a moment.
            TL_LOG_WARN("captura: a tela cheia exclusiva ainda nao deixou capturar o monitor (%s)",
                        to_string(started.status()));
            screen_retry_ns_ = now + kScreenRetryNs;
            return false;
        }

        screen_ = std::move(source);
        screen_monitor_ = monitor;
        TL_LOG_INFO("captura: janela em tela cheia exclusiva, a imagem agora vem do monitor");
        return true;
    }

    void leave_screen() noexcept
    {
        if (screen_ == nullptr) {
            return;
        }
        screen_->stop();
        screen_.reset();
        screen_monitor_ = nullptr;
        TL_LOG_INFO("captura: a janela saiu da tela cheia exclusiva, voltando a captura da janela");
    }

    CaptureTarget target_;
    CaptureOptions options_;
    HWND window_ = nullptr;

    std::unique_ptr<CaptureSource> windowed_;
    std::unique_ptr<CaptureSource> screen_;
    HMONITOR screen_monitor_ = nullptr;

    Nanoseconds shell_checked_ns_ = 0;
    Nanoseconds screen_retry_ns_ = 0;
    bool shell_exclusive_ = false;
    bool started_ = false;
};

}  // namespace

Result<std::unique_ptr<CaptureSource>> create_window_capture_source(
    const CaptureTarget& target, const CaptureOptions& options) noexcept
{
    Result<std::unique_ptr<CaptureSource>> windowed =
        create_graphics_capture_source(target, options);
    if (!windowed.ok()) {
        return windowed;
    }

    auto* source =
        new (std::nothrow) WindowCaptureSource(target, options, std::move(windowed).value());
    if (source == nullptr) {
        return Error{Status::OutOfMemory, "WindowCaptureSource"};
    }
    return std::unique_ptr<CaptureSource>(source);
}

}  // namespace tl::capture::win
