#include "synthetic_source.hpp"

#include <chrono>
#include <thread>

namespace tl::capture {
namespace {

inline constexpr std::uint32_t kBytesPerPixel = 4;
inline constexpr std::uint32_t kMinimumExtent = 64;
inline constexpr std::uint32_t kMaximumExtent = 16384;
inline constexpr std::size_t kArenaSlackBytes = 64u * 1024u;

[[nodiscard]] Nanoseconds interval_from_millihertz(std::uint32_t millihertz) noexcept
{
    if (millihertz == 0) {
        return 0;
    }
    return static_cast<Nanoseconds>(1'000'000'000'000ull / millihertz);
}

}  // namespace

SyntheticSource::SyntheticSource(const CaptureTarget& target,
                                 const CaptureOptions& options) noexcept
    : target_(target), options_(options)
{
    width_ = options_.synthetic_width;
    height_ = options_.synthetic_height;
    row_pitch_ = width_ * kBytesPerPixel;
}

Outcome SyntheticSource::start() noexcept
{
    if (started_) {
        return ok();
    }

    if (width_ < kMinimumExtent || height_ < kMinimumExtent || width_ > kMaximumExtent ||
        height_ > kMaximumExtent) {
        return fail(Status::InvalidArgument, "synthetic surface extent");
    }

    row_pitch_ = width_ * kBytesPerPixel;

    const std::size_t surface_bytes = static_cast<std::size_t>(row_pitch_) * height_;
    const std::size_t dirty_bytes =
        static_cast<std::size_t>(options_.max_dirty_rects) * (sizeof(Rect) + 4 * sizeof(int)) +
        static_cast<std::size_t>(options_.max_move_rects) * sizeof(MoveRect);
    const std::size_t cursor_bytes =
        static_cast<std::size_t>(kCursorExtent) * kCursorExtent * kBytesPerPixel;

    if (!storage_.reserve(surface_bytes + dirty_bytes + cursor_bytes + kArenaSlackBytes)) {
        return fail(Status::OutOfMemory, "synthetic arena");
    }

    LinearArena& arena = storage_.arena();
    pixels_ = arena.allocate_array_aligned<std::byte>(surface_bytes, kCacheLineSize);
    if (pixels_ == nullptr) {
        return fail(Status::OutOfMemory, "synthetic surface");
    }

    DirtyRegionBuilder::Limits limits;
    limits.max_dirty_rects = options_.max_dirty_rects;
    limits.max_move_rects = options_.max_move_rects;
    if (!dirty_.initialize(arena, width_, height_, limits)) {
        return fail(Status::OutOfMemory, "synthetic dirty region");
    }
    if (!cursor_.initialize(arena, kCursorExtent, kCursorExtent)) {
        return fail(Status::OutOfMemory, "synthetic cursor");
    }

    paint_background();
    TL_TRY(refresh_cursor_shape());

    frame_interval_ns_ = interval_from_millihertz(options_.synthetic_refresh_millihertz);
    next_present_ns_ = now_ns();
    frame_index_ = 0;
    started_ = true;
    leased_ = false;
    arena.update_high_water();
    return ok();
}

void SyntheticSource::stop() noexcept
{
    started_ = false;
    leased_ = false;
    frame_index_ = 0;
}

void SyntheticSource::release() noexcept
{
    leased_ = false;
}

Outcome SyntheticSource::map_for_readback(FrameSurface& out) noexcept
{
    out = FrameSurface{};
    if (!started_ || pixels_ == nullptr) {
        return fail(Status::Unavailable, "synthetic source not started");
    }

    out.memory = SurfaceMemory::CpuLinear;
    out.cpu_data = pixels_;
    out.row_pitch = row_pitch_;
    out.width = width_;
    out.height = height_;
    out.format = PixelFormat::B8G8R8A8Unorm;
    return ok();
}

void SyntheticSource::unmap_readback() noexcept {}

CaptureSourceInfo SyntheticSource::info() const noexcept
{
    CaptureSourceInfo out;
    out.target = target_;
    out.backend = CaptureBackend::Synthetic;
    out.width = width_;
    out.height = height_;
    out.format = PixelFormat::B8G8R8A8Unorm;
    out.refresh_millihertz = options_.synthetic_refresh_millihertz;
    return out;
}

void SyntheticSource::fill_rect(const Rect& rect, std::uint8_t blue, std::uint8_t green,
                                std::uint8_t red) noexcept
{
    const std::int32_t left = rect.left < 0 ? 0 : rect.left;
    const std::int32_t top = rect.top < 0 ? 0 : rect.top;
    const std::int32_t right = rect.right > static_cast<std::int32_t>(width_)
                                   ? static_cast<std::int32_t>(width_)
                                   : rect.right;
    const std::int32_t bottom = rect.bottom > static_cast<std::int32_t>(height_)
                                    ? static_cast<std::int32_t>(height_)
                                    : rect.bottom;

    for (std::int32_t y = top; y < bottom; ++y) {
        std::byte* row = pixels_ + static_cast<std::size_t>(y) * row_pitch_;
        for (std::int32_t x = left; x < right; ++x) {
            std::byte* pixel = row + static_cast<std::size_t>(x) * kBytesPerPixel;
            pixel[0] = static_cast<std::byte>(blue);
            pixel[1] = static_cast<std::byte>(green);
            pixel[2] = static_cast<std::byte>(red);
            pixel[3] = static_cast<std::byte>(0xFF);
        }
    }
}

void SyntheticSource::paint_background() noexcept
{
    for (std::uint32_t y = 0; y < height_; ++y) {
        std::byte* row = pixels_ + static_cast<std::size_t>(y) * row_pitch_;
        const std::uint8_t green = static_cast<std::uint8_t>((y * 255u) / height_);
        for (std::uint32_t x = 0; x < width_; ++x) {
            std::byte* pixel = row + static_cast<std::size_t>(x) * kBytesPerPixel;
            pixel[0] = static_cast<std::byte>((x * 255u) / width_);
            pixel[1] = static_cast<std::byte>(green);
            pixel[2] = static_cast<std::byte>(0x20);
            pixel[3] = static_cast<std::byte>(0xFF);
        }
    }
}

Rect SyntheticSource::block_bounds(std::uint64_t frame_index) const noexcept
{
    const std::uint32_t span = width_ > kBlockExtent ? width_ - kBlockExtent : 1u;
    const std::uint32_t left = static_cast<std::uint32_t>((frame_index * kBlockStep) % span);
    const std::int32_t top = static_cast<std::int32_t>(height_ / 4u);
    return Rect{static_cast<std::int32_t>(left), top,
                static_cast<std::int32_t>(left + kBlockExtent),
                top + static_cast<std::int32_t>(kBlockExtent)};
}

Rect SyntheticSource::blink_bounds() const noexcept
{
    const std::int32_t left = static_cast<std::int32_t>(width_ - 2u * kBlinkExtent);
    const std::int32_t top = static_cast<std::int32_t>(height_ - 2u * kBlinkExtent);
    return Rect{left, top, left + static_cast<std::int32_t>(kBlinkExtent),
                top + static_cast<std::int32_t>(kBlinkExtent)};
}

Outcome SyntheticSource::refresh_cursor_shape() noexcept
{
    std::byte shape[kCursorExtent * kCursorExtent * kBytesPerPixel];
    const std::uint32_t pitch = kCursorExtent * kBytesPerPixel;

    for (std::uint32_t y = 0; y < kCursorExtent; ++y) {
        for (std::uint32_t x = 0; x < kCursorExtent; ++x) {
            std::byte* pixel = shape + static_cast<std::size_t>(y) * pitch +
                               static_cast<std::size_t>(x) * kBytesPerPixel;
            const bool inside = x + y < kCursorExtent;
            pixel[0] = static_cast<std::byte>(inside ? 0xFF : 0x00);
            pixel[1] = static_cast<std::byte>(inside ? 0xFF : 0x00);
            pixel[2] = static_cast<std::byte>(inside ? 0xFF : 0x00);
            pixel[3] = static_cast<std::byte>(inside ? 0xFF : 0x00);
        }
    }

    return cursor_.store_shape(CursorShapeKind::Color, kCursorExtent, kCursorExtent, pitch,
                               kCursorExtent / 2u, kCursorExtent / 2u,
                               Span<const std::byte>(shape, sizeof(shape)));
}

void SyntheticSource::advance_cursor(std::uint64_t frame_index) noexcept
{
    const std::uint32_t span_x = width_ > kCursorExtent ? width_ - kCursorExtent : 1u;
    const std::uint32_t span_y = height_ > kCursorExtent ? height_ - kCursorExtent : 1u;
    const std::int32_t x = static_cast<std::int32_t>((frame_index * 7ull) % span_x);
    const std::int32_t y = static_cast<std::int32_t>((frame_index * 5ull) % span_y);
    cursor_.set_position(x, y, options_.include_cursor);
}

Outcome SyntheticSource::wait_for_next_frame(std::uint32_t timeout_ms) noexcept
{
    if (frame_interval_ns_ == 0) {
        next_present_ns_ = now_ns();
        return ok();
    }

    const Nanoseconds current = now_ns();
    if (current >= next_present_ns_) {
        return ok();
    }

    const Nanoseconds remaining = next_present_ns_ - current;
    const Nanoseconds budget = static_cast<Nanoseconds>(timeout_ms) * kNanosecondsPerMillisecond;
    if (remaining > budget) {
        return fail(Status::Timeout, "synthetic frame not due");
    }

    std::this_thread::sleep_for(std::chrono::nanoseconds(remaining));
    return ok();
}

Outcome SyntheticSource::acquire(CapturedFrame& out, std::uint32_t timeout_ms) noexcept
{
    if (!started_) {
        return fail(Status::Unavailable, "synthetic source not started");
    }
    if (leased_) {
        return fail(Status::AlreadyExists, "synthetic frame already leased");
    }

    TL_TRY(wait_for_next_frame(timeout_ms));

    const Nanoseconds present_ns = next_present_ns_;
    if (frame_interval_ns_ != 0) {
        next_present_ns_ += frame_interval_ns_;
        const Nanoseconds current = now_ns();
        if (next_present_ns_ < current) {
            next_present_ns_ = current;
        }
    }

    const bool full_surface = frame_index_ % kFullSurfacePeriod == 0;

    dirty_.begin_frame();

    if (full_surface) {
        paint_background();
        dirty_.force_full_surface();
    } else {
        const Rect previous = block_bounds(frame_index_ - 1);
        const Rect current = block_bounds(frame_index_);
        fill_rect(previous, 0x20, 0x20, 0x20);
        fill_rect(current, 0xF0, 0x80, 0x10);

        MoveRect move;
        move.destination = current;
        move.source_x = previous.left;
        move.source_y = previous.top;
        dirty_.add_move(move);

        const Rect blink = blink_bounds();
        const std::uint8_t level =
            static_cast<std::uint8_t>((frame_index_ & 1u) != 0 ? 0xFF : 0x10);
        fill_rect(blink, level, level, level);
        dirty_.add_dirty(blink);
    }

    dirty_.finish();

    if (options_.include_cursor) {
        if (frame_index_ % kCursorShapePeriod == 0 && frame_index_ != 0) {
            (void)refresh_cursor_shape();
        }
        advance_cursor(frame_index_);
    } else {
        cursor_.set_absent();
    }

    out = CapturedFrame{};
    out.surface.memory = SurfaceMemory::CpuLinear;
    out.surface.cpu_data = pixels_;
    out.surface.row_pitch = row_pitch_;
    out.surface.width = width_;
    out.surface.height = height_;
    out.surface.format = PixelFormat::B8G8R8A8Unorm;

    out.metadata.frame_index = frame_index_;
    out.metadata.present_time_ns = present_ns;
    out.metadata.acquire_time_ns = now_ns();
    out.metadata.accumulated_frames = 1;
    out.metadata.content_changed = true;
    out.metadata.cursor_changed =
        options_.include_cursor && cursor_.shape_changed_since_last_frame();
    out.metadata.full_surface_dirty = dirty_.full_surface();
    out.metadata.dirty_metadata_available = true;

    out.dirty_rects = dirty_.dirty_rects();
    out.move_rects = dirty_.move_rects();
    out.cursor = cursor_.state();

    cursor_.end_frame();
    ++frame_index_;
    leased_ = true;
    storage_.arena().update_high_water();
    return ok();
}

}  // namespace tl::capture
