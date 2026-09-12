#pragma once

#include <cstddef>
#include <cstdint>

#include "telinha/capture/captured_frame.hpp"
#include "telinha/core/arena.hpp"
#include "telinha/core/result.hpp"
#include "telinha/core/span.hpp"

namespace tl::capture {

[[nodiscard]] constexpr std::uint32_t cursor_visible_height(CursorShapeKind kind,
                                                            std::uint32_t reported_height) noexcept
{
    return kind == CursorShapeKind::Monochrome ? reported_height / 2 : reported_height;
}

[[nodiscard]] constexpr std::size_t cursor_decoded_bytes(CursorShapeKind kind, std::uint32_t width,
                                                         std::uint32_t reported_height) noexcept
{
    return static_cast<std::size_t>(width) * cursor_visible_height(kind, reported_height) * 4u;
}

struct DecodedCursorShape {
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    std::uint32_t row_pitch = 0;
    bool contains_inverted_pixels = false;
};

[[nodiscard]] Result<DecodedCursorShape> decode_cursor_shape(
    CursorShapeKind kind, std::uint32_t width, std::uint32_t reported_height,
    std::uint32_t source_pitch, Span<const std::byte> source, Span<std::byte> destination) noexcept;

class CursorTracker {
public:
    CursorTracker() noexcept = default;

    CursorTracker(const CursorTracker&) = delete;
    CursorTracker& operator=(const CursorTracker&) = delete;

    [[nodiscard]] bool initialize(LinearArena& arena, std::uint32_t max_width,
                                  std::uint32_t max_height) noexcept;

    [[nodiscard]] Outcome store_shape(CursorShapeKind kind, std::uint32_t width,
                                      std::uint32_t reported_height, std::uint32_t source_pitch,
                                      std::uint32_t hotspot_x, std::uint32_t hotspot_y,
                                      Span<const std::byte> source) noexcept;

    void set_position(std::int32_t x, std::int32_t y, bool visible) noexcept;
    void set_absent() noexcept;

    [[nodiscard]] CursorState state() const noexcept;
    [[nodiscard]] Span<const std::byte> pixels() const noexcept;
    [[nodiscard]] Rect bounds() const noexcept;

    [[nodiscard]] bool shape_changed_since_last_frame() const noexcept { return shape_changed_; }
    void end_frame() noexcept { shape_changed_ = false; }

    [[nodiscard]] std::uint64_t shape_generation() const noexcept { return shape_generation_; }
    [[nodiscard]] std::uint64_t rejected_shapes() const noexcept { return rejected_shapes_; }
    [[nodiscard]] std::size_t capacity_bytes() const noexcept { return capacity_; }
    [[nodiscard]] bool initialized() const noexcept { return pixels_ != nullptr; }

private:
    std::byte* pixels_ = nullptr;
    std::size_t capacity_ = 0;
    DecodedCursorShape decoded_;
    CursorShapeKind kind_ = CursorShapeKind::None;
    std::uint32_t hotspot_x_ = 0;
    std::uint32_t hotspot_y_ = 0;
    std::int32_t x_ = 0;
    std::int32_t y_ = 0;
    std::uint64_t shape_generation_ = 0;
    std::uint64_t rejected_shapes_ = 0;
    bool visible_ = false;
    bool position_valid_ = false;
    bool shape_changed_ = false;
};

}  // namespace tl::capture
