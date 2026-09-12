#pragma once

#include <cstdint>
#include <memory>

#include "telinha/capture/capture_target.hpp"
#include "telinha/capture/captured_frame.hpp"
#include "telinha/core/result.hpp"

namespace tl::capture {
enum class CaptureBackend : std::uint8_t {
    Automatic = 0,
    DesktopDuplication,
    GraphicsCapture,
    Synthetic,
};

const char* to_string(CaptureBackend backend) noexcept;

struct CaptureOptions {
    CaptureBackend backend = CaptureBackend::Automatic;
    bool include_cursor = true;
    bool draw_border = false;
    std::uint32_t tile_size = 16;
    std::uint32_t max_dirty_rects = 512;
    std::uint32_t max_move_rects = 128;
    std::uint32_t synthetic_width = 1920;
    std::uint32_t synthetic_height = 1080;
    std::uint32_t synthetic_refresh_millihertz = 60000;
};

struct CaptureSourceInfo {
    CaptureTarget target;
    CaptureBackend backend = CaptureBackend::Automatic;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    PixelFormat format = PixelFormat::Unknown;
    std::uint32_t refresh_millihertz = 0;
};

class CaptureSource {
public:
    virtual ~CaptureSource() = default;

    CaptureSource(const CaptureSource&) = delete;
    CaptureSource& operator=(const CaptureSource&) = delete;

    virtual Outcome start() = 0;
    virtual void stop() noexcept = 0;

    [[nodiscard]] virtual Outcome acquire(CapturedFrame& out, std::uint32_t timeout_ms) = 0;

    virtual void release() noexcept = 0;

    [[nodiscard]] virtual CaptureSourceInfo info() const noexcept = 0;

protected:
    CaptureSource() = default;
};

class FrameLease {
public:
    explicit FrameLease(CaptureSource& source) noexcept : source_(&source) {}

    FrameLease(const FrameLease&) = delete;
    FrameLease& operator=(const FrameLease&) = delete;

    ~FrameLease()
    {
        if (held_) {
            source_->release();
        }
    }

    [[nodiscard]] Outcome acquire(CapturedFrame& out, std::uint32_t timeout_ms)
    {
        if (held_) {
            source_->release();
            held_ = false;
        }
        Outcome result = source_->acquire(out, timeout_ms);
        held_ = result.ok();
        return result;
    }

    [[nodiscard]] bool held() const noexcept { return held_; }

private:
    CaptureSource* source_;
    bool held_ = false;
};

[[nodiscard]] Outcome enumerate_targets(CaptureTargetKind kind, Span<CaptureTargetInfo> out,
                                        std::uint32_t& written, std::uint32_t& available) noexcept;

[[nodiscard]] Result<std::unique_ptr<CaptureSource>> create_capture_source(
    const CaptureTarget& target, const CaptureOptions& options);

[[nodiscard]] bool backend_available(CaptureBackend backend) noexcept;
}  // namespace tl::capture
