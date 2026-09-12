#pragma once

#include <cstdint>

#include "../cursor_shape.hpp"
#include "../dirty_region_builder.hpp"
#include "d3d11_capture_device.hpp"
#include "telinha/capture/capture_source.hpp"
#include "telinha/core/arena.hpp"

namespace tl::capture::win {

class DesktopDuplicationSource final : public CaptureSource {
public:
    static constexpr std::uint32_t kRingCapacity = 3;
    static constexpr std::uint32_t kMaxCursorExtent = 256;
    static constexpr std::uint32_t kMetadataHeadroomBytes = 4096;

    DesktopDuplicationSource(const CaptureTarget& target, const CaptureOptions& options) noexcept;
    ~DesktopDuplicationSource() override;

    Outcome start() noexcept override;
    void stop() noexcept override;
    [[nodiscard]] Outcome acquire(CapturedFrame& out, std::uint32_t timeout_ms) noexcept override;
    void release() noexcept override;
    [[nodiscard]] CaptureSourceInfo info() const noexcept override;

private:
    void resolve_layout_from_monitor() noexcept;
    [[nodiscard]] Outcome create_duplication() noexcept;
    void destroy_duplication() noexcept;
    [[nodiscard]] Outcome adopt_duplication_layout(bool& layout_changed) noexcept;
    [[nodiscard]] Outcome pull_frame(CapturedFrame& out, std::uint32_t timeout_ms) noexcept;
    void collect_dirty_metadata(const DXGI_OUTDUPL_FRAME_INFO& info) noexcept;
    void collect_cursor(const DXGI_OUTDUPL_FRAME_INFO& info) noexcept;
    void publish(CapturedFrame& out, const DXGI_OUTDUPL_FRAME_INFO& info, ID3D11Texture2D* texture,
                 bool content_changed) noexcept;

    CaptureTarget target_;
    CaptureOptions options_;
    HMONITOR monitor_ = nullptr;

    D3D11CaptureDevice device_;
    ComPtr<IDXGIOutputDuplication> duplication_;
    D3D11TextureRing ring_;
    ArenaStorage storage_;
    DirtyRegionBuilder dirty_;
    CursorTracker cursor_;

    std::byte* metadata_ = nullptr;
    std::uint32_t metadata_capacity_ = 0;
    std::byte* pointer_shape_ = nullptr;
    std::uint32_t pointer_shape_capacity_ = 0;

    SurfaceLayout layout_;
    SurfaceRotation rotation_ = SurfaceRotation::None;
    std::uint32_t refresh_millihertz_ = 0;

    ComPtr<ID3D11Texture2D> last_texture_;
    TextureHandle leased_handle_;
    std::uint64_t frame_index_ = 0;
    std::uint64_t frames_without_metadata_ = 0;
    bool leased_ = false;
    bool leased_from_ring_ = false;
    bool started_ = false;
};

}  // namespace tl::capture::win
