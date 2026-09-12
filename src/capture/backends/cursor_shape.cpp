#include "cursor_shape.hpp"

#include <cstring>

namespace tl::capture {
namespace {

inline constexpr std::uint32_t kBytesPerPixel = 4;

void store_pixel(std::byte* destination, std::byte blue, std::byte green, std::byte red,
                 std::byte alpha) noexcept
{
    destination[0] = blue;
    destination[1] = green;
    destination[2] = red;
    destination[3] = alpha;
}

void store_opaque_black(std::byte* destination) noexcept
{
    store_pixel(destination, std::byte{0x00}, std::byte{0x00}, std::byte{0x00}, std::byte{0xFF});
}

void store_opaque_white(std::byte* destination) noexcept
{
    store_pixel(destination, std::byte{0xFF}, std::byte{0xFF}, std::byte{0xFF}, std::byte{0xFF});
}

void store_transparent(std::byte* destination) noexcept
{
    store_pixel(destination, std::byte{0x00}, std::byte{0x00}, std::byte{0x00}, std::byte{0x00});
}

[[nodiscard]] bool bit_set(const std::byte* row, std::uint32_t x) noexcept
{
    const std::byte mask = static_cast<std::byte>(0x80u >> (x & 7u));
    return (row[x >> 3] & mask) != std::byte{0};
}

[[nodiscard]] Outcome decode_monochrome(std::uint32_t width, std::uint32_t reported_height,
                                        std::uint32_t source_pitch, Span<const std::byte> source,
                                        Span<std::byte> destination, bool& inverted) noexcept
{
    const std::uint32_t height = reported_height / 2;
    if (height == 0) {
        return fail(Status::InvalidArgument, "monochrome cursor height");
    }
    if (source_pitch < (width + 7u) / 8u) {
        return fail(Status::InvalidArgument, "monochrome cursor pitch");
    }
    if (source.size() < static_cast<std::size_t>(source_pitch) * reported_height) {
        return fail(Status::OutOfRange, "monochrome cursor source");
    }

    const std::size_t destination_pitch = static_cast<std::size_t>(width) * kBytesPerPixel;
    for (std::uint32_t y = 0; y < height; ++y) {
        const std::byte* and_row = source.data() + static_cast<std::size_t>(y) * source_pitch;
        const std::byte* xor_row =
            source.data() + static_cast<std::size_t>(y + height) * source_pitch;
        std::byte* out = destination.data() + static_cast<std::size_t>(y) * destination_pitch;

        for (std::uint32_t x = 0; x < width; ++x) {
            std::byte* target = out + static_cast<std::size_t>(x) * kBytesPerPixel;
            const bool masked = bit_set(and_row, x);
            const bool inverts = bit_set(xor_row, x);

            if (!masked) {
                if (inverts) {
                    store_opaque_white(target);
                } else {
                    store_opaque_black(target);
                }
            } else if (!inverts) {
                store_transparent(target);
            } else {
                inverted = true;
                store_opaque_white(target);
            }
        }
    }
    return ok();
}

[[nodiscard]] Outcome decode_color(std::uint32_t width, std::uint32_t height,
                                   std::uint32_t source_pitch, Span<const std::byte> source,
                                   Span<std::byte> destination) noexcept
{
    const std::size_t destination_pitch = static_cast<std::size_t>(width) * kBytesPerPixel;
    if (source_pitch < destination_pitch) {
        return fail(Status::InvalidArgument, "color cursor pitch");
    }
    if (source.size() < static_cast<std::size_t>(source_pitch) * height) {
        return fail(Status::OutOfRange, "color cursor source");
    }

    for (std::uint32_t y = 0; y < height; ++y) {
        std::memcpy(destination.data() + static_cast<std::size_t>(y) * destination_pitch,
                    source.data() + static_cast<std::size_t>(y) * source_pitch, destination_pitch);
    }
    return ok();
}

[[nodiscard]] Outcome decode_masked_color(std::uint32_t width, std::uint32_t height,
                                          std::uint32_t source_pitch, Span<const std::byte> source,
                                          Span<std::byte> destination, bool& inverted) noexcept
{
    const std::size_t destination_pitch = static_cast<std::size_t>(width) * kBytesPerPixel;
    if (source_pitch < destination_pitch) {
        return fail(Status::InvalidArgument, "masked color cursor pitch");
    }
    if (source.size() < static_cast<std::size_t>(source_pitch) * height) {
        return fail(Status::OutOfRange, "masked color cursor source");
    }

    for (std::uint32_t y = 0; y < height; ++y) {
        const std::byte* in = source.data() + static_cast<std::size_t>(y) * source_pitch;
        std::byte* out = destination.data() + static_cast<std::size_t>(y) * destination_pitch;

        for (std::uint32_t x = 0; x < width; ++x) {
            const std::byte* pixel = in + static_cast<std::size_t>(x) * kBytesPerPixel;
            std::byte* target = out + static_cast<std::size_t>(x) * kBytesPerPixel;

            if (pixel[3] == std::byte{0x00}) {
                store_pixel(target, pixel[0], pixel[1], pixel[2], std::byte{0xFF});
            } else {
                inverted = true;
                store_opaque_white(target);
            }
        }
    }
    return ok();
}

}  // namespace

Result<DecodedCursorShape> decode_cursor_shape(CursorShapeKind kind, std::uint32_t width,
                                               std::uint32_t reported_height,
                                               std::uint32_t source_pitch,
                                               Span<const std::byte> source,
                                               Span<std::byte> destination) noexcept
{
    if (kind == CursorShapeKind::None) {
        return Error{Status::InvalidArgument, "cursor shape kind"};
    }
    if (width == 0 || reported_height == 0) {
        return Error{Status::InvalidArgument, "cursor shape extent"};
    }
    if (source.data() == nullptr || destination.data() == nullptr) {
        return Error{Status::InvalidArgument, "cursor shape buffer"};
    }
    if (destination.size() < cursor_decoded_bytes(kind, width, reported_height)) {
        return Error{Status::OutOfRange, "cursor shape destination"};
    }

    const std::uint32_t height = cursor_visible_height(kind, reported_height);
    bool inverted = false;
    Outcome outcome = ok();

    switch (kind) {
        case CursorShapeKind::Monochrome:
            outcome = decode_monochrome(width, reported_height, source_pitch, source, destination,
                                        inverted);
            break;
        case CursorShapeKind::Color:
            outcome = decode_color(width, height, source_pitch, source, destination);
            break;
        case CursorShapeKind::MaskedColor:
            outcome =
                decode_masked_color(width, height, source_pitch, source, destination, inverted);
            break;
        case CursorShapeKind::None: break;
    }

    if (!outcome.ok()) {
        return outcome.error();
    }

    DecodedCursorShape decoded;
    decoded.width = width;
    decoded.height = height;
    decoded.row_pitch = width * kBytesPerPixel;
    decoded.contains_inverted_pixels = inverted;
    return decoded;
}

bool CursorTracker::initialize(LinearArena& arena, std::uint32_t max_width,
                               std::uint32_t max_height) noexcept
{
    if (max_width == 0 || max_height == 0) {
        return false;
    }

    const std::size_t capacity = static_cast<std::size_t>(max_width) * max_height * kBytesPerPixel;
    pixels_ = arena.allocate_array_aligned<std::byte>(capacity, kCacheLineSize);
    if (pixels_ == nullptr) {
        return false;
    }
    capacity_ = capacity;
    return true;
}

Outcome CursorTracker::store_shape(CursorShapeKind kind, std::uint32_t width,
                                   std::uint32_t reported_height, std::uint32_t source_pitch,
                                   std::uint32_t hotspot_x, std::uint32_t hotspot_y,
                                   Span<const std::byte> source) noexcept
{
    if (!initialized()) {
        return fail(Status::Unavailable, "cursor tracker uninitialized");
    }
    if (cursor_decoded_bytes(kind, width, reported_height) > capacity_) {
        ++rejected_shapes_;
        return fail(Status::OutOfRange, "cursor shape exceeds reserved buffer");
    }

    const Result<DecodedCursorShape> decoded = decode_cursor_shape(
        kind, width, reported_height, source_pitch, source, Span<std::byte>(pixels_, capacity_));
    if (!decoded.ok()) {
        ++rejected_shapes_;
        return Outcome{decoded.error()};
    }

    decoded_ = decoded.value();
    kind_ = kind;
    hotspot_x_ = hotspot_x;
    hotspot_y_ = hotspot_y;
    ++shape_generation_;
    shape_changed_ = true;
    return ok();
}

void CursorTracker::set_position(std::int32_t x, std::int32_t y, bool visible) noexcept
{
    x_ = x;
    y_ = y;
    visible_ = visible;
    position_valid_ = true;
}

void CursorTracker::set_absent() noexcept
{
    visible_ = false;
}

CursorState CursorTracker::state() const noexcept
{
    CursorState out;
    out.x = x_;
    out.y = y_;
    out.hotspot_x = hotspot_x_;
    out.hotspot_y = hotspot_y_;
    out.width = decoded_.width;
    out.height = decoded_.height;
    out.shape_generation = shape_generation_;
    out.kind = kind_;
    out.visible = visible_ && decoded_.width != 0 && decoded_.height != 0;
    out.position_valid = position_valid_;
    return out;
}

Span<const std::byte> CursorTracker::pixels() const noexcept
{
    if (decoded_.width == 0 || decoded_.height == 0) {
        return Span<const std::byte>();
    }
    return Span<const std::byte>(pixels_,
                                 static_cast<std::size_t>(decoded_.row_pitch) * decoded_.height);
}

Rect CursorTracker::bounds() const noexcept
{
    if (!visible_ || decoded_.width == 0 || decoded_.height == 0) {
        return Rect{};
    }
    return Rect{x_, y_, x_ + static_cast<std::int32_t>(decoded_.width),
                y_ + static_cast<std::int32_t>(decoded_.height)};
}

}  // namespace tl::capture
