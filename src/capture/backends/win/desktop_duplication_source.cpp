#include "desktop_duplication_source.hpp"

#include "target_enumeration.hpp"

namespace tl::capture::win {
namespace {

inline constexpr std::size_t kArenaSlackBytes = 64u * 1024u;

class FrameGuard {
public:
    explicit FrameGuard(IDXGIOutputDuplication* duplication) noexcept : duplication_(duplication) {}

    FrameGuard(const FrameGuard&) = delete;
    FrameGuard& operator=(const FrameGuard&) = delete;

    ~FrameGuard()
    {
        if (duplication_ != nullptr) {
            duplication_->ReleaseFrame();
        }
    }

private:
    IDXGIOutputDuplication* duplication_;
};

[[nodiscard]] CursorShapeKind cursor_kind_from_dxgi(UINT type) noexcept
{
    switch (type) {
        case DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME: return CursorShapeKind::Monochrome;
        case DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR: return CursorShapeKind::Color;
        case DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR: return CursorShapeKind::MaskedColor;
        default: break;
    }
    return CursorShapeKind::None;
}

[[nodiscard]] Rect rect_from_win32(const RECT& source) noexcept
{
    return Rect{source.left, source.top, source.right, source.bottom};
}

}  // namespace

DesktopDuplicationSource::DesktopDuplicationSource(const CaptureTarget& target,
                                                   const CaptureOptions& options) noexcept
    : target_(target), options_(options)
{
    if (target_.kind == CaptureTargetKind::Monitor && target_.handle != 0) {
        monitor_ = reinterpret_cast<HMONITOR>(static_cast<std::uintptr_t>(target_.handle));
        resolve_layout_from_monitor();
    }
}

void DesktopDuplicationSource::resolve_layout_from_monitor() noexcept
{
    CaptureTargetInfo description;
    if (!describe_monitor(monitor_, description).ok()) {
        return;
    }

    rotation_ = description.rotation;
    refresh_millihertz_ = description.refresh_millihertz;

    const bool swaps_axes =
        rotation_ == SurfaceRotation::Clockwise90 || rotation_ == SurfaceRotation::Clockwise270;
    layout_.width = swaps_axes ? description.height : description.width;
    layout_.height = swaps_axes ? description.width : description.height;
    layout_.format = PixelFormat::B8G8R8A8Unorm;
}

DesktopDuplicationSource::~DesktopDuplicationSource()
{
    stop();
}

Outcome DesktopDuplicationSource::start() noexcept
{
    if (started_) {
        return ok();
    }
    if (target_.kind != CaptureTargetKind::Monitor || target_.handle == 0) {
        return fail(Status::InvalidArgument, "desktop duplication needs a monitor target");
    }

    monitor_ = reinterpret_cast<HMONITOR>(static_cast<std::uintptr_t>(target_.handle));
    if (!layout_.valid()) {
        resolve_layout_from_monitor();
    }

    TL_TRY(device_.create_for_monitor(monitor_));

    metadata_capacity_ =
        options_.max_dirty_rects * static_cast<std::uint32_t>(sizeof(RECT)) +
        options_.max_move_rects * static_cast<std::uint32_t>(sizeof(DXGI_OUTDUPL_MOVE_RECT)) +
        kMetadataHeadroomBytes;
    pointer_shape_capacity_ = kMaxCursorExtent * kMaxCursorExtent * 4u;

    const std::size_t dirty_bytes =
        static_cast<std::size_t>(options_.max_dirty_rects) * (sizeof(Rect) + 4 * sizeof(int)) +
        static_cast<std::size_t>(options_.max_move_rects) * sizeof(MoveRect);
    const std::size_t cursor_bytes =
        static_cast<std::size_t>(kMaxCursorExtent) * kMaxCursorExtent * 4u;

    if (!storage_.reserve(metadata_capacity_ + pointer_shape_capacity_ + dirty_bytes +
                          cursor_bytes + kArenaSlackBytes)) {
        stop();
        return fail(Status::OutOfMemory, "desktop duplication arena");
    }

    LinearArena& arena = storage_.arena();
    metadata_ = arena.allocate_array_aligned<std::byte>(metadata_capacity_, kCacheLineSize);
    pointer_shape_ =
        arena.allocate_array_aligned<std::byte>(pointer_shape_capacity_, kCacheLineSize);
    if (metadata_ == nullptr || pointer_shape_ == nullptr) {
        stop();
        return fail(Status::OutOfMemory, "desktop duplication scratch");
    }

    if (!cursor_.initialize(arena, kMaxCursorExtent, kMaxCursorExtent)) {
        stop();
        return fail(Status::OutOfMemory, "desktop duplication cursor");
    }

    const Outcome duplicated = create_duplication();
    if (!duplicated.ok()) {
        stop();
        return duplicated;
    }

    DirtyRegionBuilder::Limits limits;
    limits.max_dirty_rects = options_.max_dirty_rects;
    limits.max_move_rects = options_.max_move_rects;
    if (!dirty_.initialize(arena, layout_.width, layout_.height, limits)) {
        stop();
        return fail(Status::OutOfMemory, "desktop duplication dirty region");
    }

    const Outcome ring = ring_.initialize(arena, device_.device(), kRingCapacity);
    if (!ring.ok()) {
        stop();
        return ring;
    }
    ring_.set_layout(layout_);

    frame_index_ = 0;
    leased_ = false;
    leased_from_ring_ = false;
    started_ = true;
    arena.update_high_water();
    return ok();
}

void DesktopDuplicationSource::stop() noexcept
{
    release();
    last_texture_.Reset();
    destroy_duplication();
    ring_.destroy();
    device_.destroy();
    metadata_ = nullptr;
    pointer_shape_ = nullptr;
    metadata_capacity_ = 0;
    pointer_shape_capacity_ = 0;
    started_ = false;
}

void DesktopDuplicationSource::destroy_duplication() noexcept
{
    duplication_.Reset();
}

Outcome DesktopDuplicationSource::create_duplication() noexcept
{
    destroy_duplication();

    if (!device_.valid() || device_.output() == nullptr) {
        TL_TRY(device_.create_for_monitor(monitor_));
    }

    HRESULT hr = device_.output()->DuplicateOutput(device_.device(), &duplication_);
    if (hr == DXGI_ERROR_NOT_CURRENTLY_AVAILABLE) {
        return fail(Status::Unavailable, "DuplicateOutput: duplication limit reached",
                    static_cast<std::int32_t>(hr));
    }
    if (hr == DXGI_ERROR_NOT_FOUND || hr == DXGI_ERROR_DEVICE_REMOVED) {
        TL_TRY(device_.create_for_monitor(monitor_));
        hr = device_.output()->DuplicateOutput(device_.device(), &duplication_);
    }
    if (FAILED(hr)) {
        return outcome_from_hresult(hr, "DuplicateOutput");
    }

    bool layout_changed = false;
    return adopt_duplication_layout(layout_changed);
}

Outcome DesktopDuplicationSource::adopt_duplication_layout(bool& layout_changed) noexcept
{
    layout_changed = false;

    DXGI_OUTDUPL_DESC description = {};
    duplication_->GetDesc(&description);

    const SurfaceRotation rotation = rotation_from_dxgi(description.Rotation);
    const bool swaps_axes =
        rotation == SurfaceRotation::Clockwise90 || rotation == SurfaceRotation::Clockwise270;

    SurfaceLayout observed;
    observed.width = swaps_axes ? description.ModeDesc.Height : description.ModeDesc.Width;
    observed.height = swaps_axes ? description.ModeDesc.Width : description.ModeDesc.Height;
    observed.format = pixel_format_from_dxgi(description.ModeDesc.Format);
    if (observed.format == PixelFormat::Unknown) {
        observed.format = PixelFormat::B8G8R8A8Unorm;
    }

    if (!observed.valid()) {
        return fail(Status::Unavailable, "duplicated output reports an empty mode");
    }

    if (observed != layout_ || rotation != rotation_) {
        layout_changed = layout_ != SurfaceLayout{};
        layout_ = observed;
        rotation_ = rotation;
        ring_.set_layout(layout_);
        if (dirty_.initialized()) {
            dirty_.resize(layout_.width, layout_.height);
        }
    }

    if (description.ModeDesc.RefreshRate.Denominator != 0) {
        refresh_millihertz_ = static_cast<std::uint32_t>(
            (static_cast<std::uint64_t>(description.ModeDesc.RefreshRate.Numerator) * 1000ull) /
            description.ModeDesc.RefreshRate.Denominator);
    }
    return ok();
}

void DesktopDuplicationSource::release() noexcept
{
    if (leased_ && leased_from_ring_) {
        ring_.give_back(leased_handle_);
    }
    leased_ = false;
    leased_from_ring_ = false;
    leased_handle_ = TextureHandle{};
}

CaptureSourceInfo DesktopDuplicationSource::info() const noexcept
{
    CaptureSourceInfo out;
    out.target = target_;
    out.backend = CaptureBackend::DesktopDuplication;
    out.width = layout_.width;
    out.height = layout_.height;
    out.format = layout_.format;
    out.rotation = rotation_;
    out.refresh_millihertz = refresh_millihertz_;
    out.process_id = 0;
    out.native_device = device_.device();
    return out;
}

void DesktopDuplicationSource::collect_cursor(const DXGI_OUTDUPL_FRAME_INFO& info) noexcept
{
    if (!options_.include_cursor) {
        cursor_.set_absent();
        return;
    }

    if (info.LastMouseUpdateTime.QuadPart != 0) {
        cursor_.set_position(info.PointerPosition.Position.x, info.PointerPosition.Position.y,
                             info.PointerPosition.Visible != FALSE);
    }

    if (info.PointerShapeBufferSize == 0) {
        return;
    }
    if (info.PointerShapeBufferSize > pointer_shape_capacity_) {
        return;
    }

    UINT required = 0;
    DXGI_OUTDUPL_POINTER_SHAPE_INFO shape = {};
    const HRESULT hr = duplication_->GetFramePointerShape(pointer_shape_capacity_, pointer_shape_,
                                                          &required, &shape);
    if (FAILED(hr)) {
        return;
    }

    const CursorShapeKind kind = cursor_kind_from_dxgi(shape.Type);
    if (kind == CursorShapeKind::None) {
        return;
    }

    (void)cursor_.store_shape(kind, shape.Width, shape.Height, shape.Pitch,
                              static_cast<std::uint32_t>(shape.HotSpot.x),
                              static_cast<std::uint32_t>(shape.HotSpot.y),
                              Span<const std::byte>(pointer_shape_, required));
}

void DesktopDuplicationSource::collect_dirty_metadata(const DXGI_OUTDUPL_FRAME_INFO& info) noexcept
{
    dirty_.begin_frame();

    if (info.TotalMetadataBufferSize == 0 || info.TotalMetadataBufferSize > metadata_capacity_) {
        dirty_.force_full_surface_without_metadata();
        dirty_.finish();
        return;
    }

    UINT move_bytes = 0;
    HRESULT hr = duplication_->GetFrameMoveRects(
        metadata_capacity_, reinterpret_cast<DXGI_OUTDUPL_MOVE_RECT*>(metadata_), &move_bytes);
    if (FAILED(hr)) {
        dirty_.force_full_surface_without_metadata();
        dirty_.finish();
        return;
    }

    UINT dirty_bytes = 0;
    hr = duplication_->GetFrameDirtyRects(metadata_capacity_ - move_bytes,
                                          reinterpret_cast<RECT*>(metadata_ + move_bytes),
                                          &dirty_bytes);
    if (FAILED(hr)) {
        dirty_.force_full_surface_without_metadata();
        dirty_.finish();
        return;
    }

    const auto* moves = reinterpret_cast<const DXGI_OUTDUPL_MOVE_RECT*>(metadata_);
    const UINT move_count = move_bytes / static_cast<UINT>(sizeof(DXGI_OUTDUPL_MOVE_RECT));
    for (UINT i = 0; i < move_count; ++i) {
        MoveRect move;
        move.destination = rect_from_win32(moves[i].DestinationRect);
        move.source_x = moves[i].SourcePoint.x;
        move.source_y = moves[i].SourcePoint.y;
        dirty_.add_move(move);
    }

    const auto* rects = reinterpret_cast<const RECT*>(metadata_ + move_bytes);
    const UINT dirty_count = dirty_bytes / static_cast<UINT>(sizeof(RECT));
    for (UINT i = 0; i < dirty_count; ++i) {
        dirty_.add_dirty(rect_from_win32(rects[i]));
    }

    if (move_count == 0 && dirty_count == 0) {
        dirty_.force_full_surface();
    }

    dirty_.finish();
}

void DesktopDuplicationSource::publish(CapturedFrame& out, const DXGI_OUTDUPL_FRAME_INFO& info,
                                       ID3D11Texture2D* texture, bool content_changed) noexcept
{
    out = CapturedFrame{};
    out.surface.memory = SurfaceMemory::GpuTexture;
    out.surface.gpu_texture = texture;
    out.surface.width = layout_.width;
    out.surface.height = layout_.height;
    out.surface.format = layout_.format;
    out.surface.rotation = rotation_;

    out.metadata.frame_index = frame_index_;
    out.metadata.present_time_ns =
        info.LastPresentTime.QuadPart > 0
            ? ticks_to_ns(static_cast<std::uint64_t>(info.LastPresentTime.QuadPart))
            : 0;
    out.metadata.acquire_time_ns = now_ns();
    out.metadata.accumulated_frames = info.AccumulatedFrames;
    out.metadata.content_changed = content_changed;
    out.metadata.cursor_changed =
        cursor_.shape_changed_since_last_frame() || info.LastMouseUpdateTime.QuadPart != 0;
    out.metadata.full_surface_dirty = content_changed && dirty_.full_surface();

    if (content_changed) {
        out.dirty_rects = dirty_.dirty_rects();
        out.move_rects = dirty_.move_rects();
    }
    out.cursor = cursor_.state();

    cursor_.end_frame();
    ++frame_index_;
}

Outcome DesktopDuplicationSource::pull_frame(CapturedFrame& out, std::uint32_t timeout_ms) noexcept
{
    DXGI_OUTDUPL_FRAME_INFO info = {};
    ComPtr<IDXGIResource> resource;

    const HRESULT hr = duplication_->AcquireNextFrame(timeout_ms, &info, &resource);
    if (hr == DXGI_ERROR_WAIT_TIMEOUT) {
        return fail(Status::Timeout, "AcquireNextFrame");
    }
    if (FAILED(hr)) {
        return outcome_from_hresult(hr, "AcquireNextFrame");
    }

    const FrameGuard guard(duplication_.Get());

    collect_cursor(info);

    const bool content_changed = info.LastPresentTime.QuadPart != 0;
    if (!content_changed) {
        if (last_texture_ == nullptr) {
            return fail(Status::Timeout, "cursor update without a previous frame");
        }
        publish(out, info, last_texture_.Get(), false);
        leased_ = true;
        leased_from_ring_ = false;
        return ok();
    }

    ComPtr<ID3D11Texture2D> desktop;
    HRESULT cast = resource.As(&desktop);
    if (FAILED(cast)) {
        return outcome_from_hresult(cast, "desktop image is not a texture");
    }

    D3D11_TEXTURE2D_DESC desktop_description = {};
    desktop->GetDesc(&desktop_description);

    SurfaceLayout observed;
    observed.width = desktop_description.Width;
    observed.height = desktop_description.Height;
    observed.format = pixel_format_from_dxgi(desktop_description.Format);
    if (observed != layout_) {
        layout_ = observed;
        ring_.set_layout(layout_);
        dirty_.resize(layout_.width, layout_.height);
        last_texture_.Reset();
        return fail(Status::ConfigurationChanged, "duplicated surface layout changed");
    }

    collect_dirty_metadata(info);

    ID3D11Texture2D* destination = nullptr;
    TextureHandle handle;
    TL_TRY(ring_.acquire(destination, handle));

    device_.context()->CopyResource(destination, desktop.Get());

    last_texture_ = destination;
    leased_handle_ = handle;
    leased_ = true;
    leased_from_ring_ = true;
    publish(out, info, destination, true);
    return ok();
}

Outcome DesktopDuplicationSource::acquire(CapturedFrame& out, std::uint32_t timeout_ms) noexcept
{
    if (!started_) {
        return fail(Status::Unavailable, "desktop duplication not started");
    }
    if (leased_) {
        return fail(Status::AlreadyExists, "frame already leased");
    }

    if (duplication_ == nullptr) {
        TL_TRY(create_duplication());
    }

    const Outcome pulled = pull_frame(out, timeout_ms);
    if (pulled.status() != Status::DeviceLost) {
        return pulled;
    }

    destroy_duplication();
    last_texture_.Reset();

    const SurfaceLayout previous_layout = layout_;
    const SurfaceRotation previous_rotation = rotation_;
    const Outcome recreated = create_duplication();
    if (!recreated.ok()) {
        return recreated;
    }

    if (layout_ != previous_layout || rotation_ != previous_rotation) {
        return fail(Status::ConfigurationChanged, "duplicated output changed mode");
    }
    return fail(Status::DeviceLost, "duplication recreated after access loss");
}

}  // namespace tl::capture::win
