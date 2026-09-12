#pragma once

#include <cstddef>
#include <cstdint>

#include "telinha/capture/capture_target.hpp"
#include "telinha/capture/pixel_format.hpp"
#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/rect.hpp"
#include "telinha/core/span.hpp"

namespace tl::capture {
enum class SurfaceMemory : std::uint8_t {
    None = 0,
    GpuTexture,
    CpuLinear,
};

struct FrameSurface {
    SurfaceMemory memory = SurfaceMemory::None;
    void* gpu_texture = nullptr;
    std::byte* cpu_data = nullptr;
    std::uint32_t row_pitch = 0;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    PixelFormat format = PixelFormat::Unknown;
    SurfaceRotation rotation = SurfaceRotation::None;

    [[nodiscard]] bool valid() const noexcept { return memory != SurfaceMemory::None; }
};

struct MoveRect {
    Rect destination;
    std::int32_t source_x = 0;
    std::int32_t source_y = 0;
};

enum class CursorShapeKind : std::uint8_t {
    None = 0,
    Monochrome,
    Color,
    MaskedColor,
};

struct CursorState {
    std::int32_t x = 0;
    std::int32_t y = 0;
    std::uint32_t hotspot_x = 0;
    std::uint32_t hotspot_y = 0;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    std::uint64_t shape_generation = 0;
    CursorShapeKind kind = CursorShapeKind::None;
    bool visible = false;
    bool position_valid = false;
};

struct FrameMetadata {
    std::uint64_t frame_index = 0;
    Nanoseconds present_time_ns = 0;
    Nanoseconds acquire_time_ns = 0;
    std::uint32_t accumulated_frames = 0;
    bool content_changed = false;
    bool cursor_changed = false;
    bool full_surface_dirty = false;
};

struct CapturedFrame {
    FrameSurface surface;
    FrameMetadata metadata;
    RectSoA dirty_rects;
    Span<const MoveRect> move_rects;
    CursorState cursor;

    [[nodiscard]] bool valid() const noexcept { return surface.valid(); }
};
}  // namespace tl::capture
