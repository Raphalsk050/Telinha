#include "graphics_capture_source.hpp"

#if defined(TELINHA_ENABLE_WGC)

#include <objbase.h>

#include <atomic>
#include <cstdint>
#include <new>

#include "../dirty_region_builder.hpp"
#include "d3d11_capture_device.hpp"
#include "target_enumeration.hpp"
#include "telinha/core/arena.hpp"

#if defined(_MSC_VER)
#pragma warning(push)
#pragma warning(disable : 4265 4355 4459 4623 4625 4626 5026 5027 5204 5205 5220)
#endif

#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <winrt/Windows.Foundation.Metadata.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>
#include <winrt/Windows.Graphics.DirectX.h>
#include <winrt/Windows.Graphics.h>

#if defined(_MSC_VER)
#pragma warning(pop)
#endif

namespace tl::capture::win {
namespace {

namespace wgc = winrt::Windows::Graphics::Capture;
namespace wgdx = winrt::Windows::Graphics::DirectX;
namespace wgdx11 = winrt::Windows::Graphics::DirectX::Direct3D11;

using DxgiInterfaceAccess = ::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess;

inline constexpr std::size_t kArenaSlackBytes = 64u * 1024u;
inline constexpr std::uint32_t kFramePoolDepth = 2;
inline constexpr std::uint32_t kRingCapacity = 3;

[[nodiscard]] bool property_present(const wchar_t* type, const wchar_t* property) noexcept
{
    try {
        return winrt::Windows::Foundation::Metadata::ApiInformation::IsPropertyPresent(type,
                                                                                       property);
    } catch (...) {
        return false;
    }
}

class GraphicsCaptureSource final : public CaptureSource {
public:
    GraphicsCaptureSource(const CaptureTarget& target, const CaptureOptions& options) noexcept
        : target_(target), options_(options)
    {
        if (target_.kind == CaptureTargetKind::Window && target_.handle != 0) {
            window_ = reinterpret_cast<HWND>(static_cast<std::uintptr_t>(target_.handle));
            CaptureTargetInfo description;
            if (describe_window(window_, description).ok()) {
                layout_.width = description.width;
                layout_.height = description.height;
                layout_.format = PixelFormat::B8G8R8A8Unorm;
                process_id_ = description.process_id;
            }
        }
    }

    ~GraphicsCaptureSource() override { stop(); }

    Outcome start() noexcept override
    {
        if (started_) {
            return ok();
        }
        if (target_.kind != CaptureTargetKind::Window || target_.handle == 0) {
            return fail(Status::InvalidArgument, "graphics capture needs a window target");
        }

        window_ = reinterpret_cast<HWND>(static_cast<std::uintptr_t>(target_.handle));
        if (IsWindow(window_) == 0) {
            return fail(Status::TargetGone, "window no longer exists");
        }

        const HRESULT com = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
        if (SUCCEEDED(com)) {
            owns_com_ = true;
            com_thread_ = GetCurrentThreadId();
        } else if (com != RPC_E_CHANGED_MODE) {
            return outcome_from_hresult(com, "CoInitializeEx");
        }

        const Outcome prepared = prepare();
        if (!prepared.ok()) {
            stop();
            return prepared;
        }
        started_ = true;
        return ok();
    }

    void stop() noexcept override
    {
        release();
        teardown_capture();

        last_texture_.Reset();
        ring_.destroy();
        device_.destroy();

        if (frame_ready_ != nullptr) {
            CloseHandle(frame_ready_);
            frame_ready_ = nullptr;
        }
        if (owns_com_ && com_thread_ == GetCurrentThreadId()) {
            CoUninitialize();
        }
        owns_com_ = false;
        started_ = false;
    }

    [[nodiscard]] Outcome acquire(CapturedFrame& out, std::uint32_t timeout_ms) noexcept override
    {
        if (!started_) {
            return fail(Status::Unavailable, "graphics capture not started");
        }
        if (leased_) {
            return fail(Status::AlreadyExists, "frame already leased");
        }
        if (closed_.load(std::memory_order_acquire) || IsWindow(window_) == 0) {
            return fail(Status::TargetGone, "captured window closed");
        }
        if (IsIconic(window_) != 0) {
            return fail(Status::Unavailable, "captured window is minimized");
        }

        try {
            return pull_frame(out, timeout_ms);
        } catch (const winrt::hresult_error& error) {
            return outcome_from_hresult(error.code(), "graphics capture acquire");
        } catch (...) {
            return fail(Status::Unknown, "graphics capture acquire");
        }
    }

    void release() noexcept override
    {
        if (leased_) {
            ring_.give_back(leased_handle_);
        }
        leased_ = false;
        leased_handle_ = TextureHandle{};
    }

    [[nodiscard]] CaptureSourceInfo info() const noexcept override
    {
        CaptureSourceInfo out;
        out.target = target_;
        out.backend = CaptureBackend::GraphicsCapture;
        out.width = layout_.width;
        out.height = layout_.height;
        out.format = layout_.format;
        out.rotation = SurfaceRotation::None;
        out.refresh_millihertz = 0;
        out.process_id = process_id_;
        out.native_device = device_.device();
        return out;
    }

private:
    Outcome prepare() noexcept
    {
        try {
            if (!wgc::GraphicsCaptureSession::IsSupported()) {
                return fail(Status::NotSupported, "Windows Graphics Capture is unavailable");
            }
        } catch (...) {
            return fail(Status::NotSupported, "Windows Graphics Capture is unavailable");
        }

        CaptureTargetInfo description;
        TL_TRY(describe_window(window_, description));
        process_id_ = description.process_id;

        TL_TRY(device_.create_default());

        ComPtr<IDXGIDevice> dxgi_device;
        HRESULT hr = device_.device()->QueryInterface(IID_PPV_ARGS(&dxgi_device));
        if (FAILED(hr)) {
            return outcome_from_hresult(hr, "IDXGIDevice");
        }

        try {
            winrt::com_ptr<::IInspectable> inspectable;
            hr = CreateDirect3D11DeviceFromDXGIDevice(dxgi_device.Get(), inspectable.put());
            if (FAILED(hr)) {
                return outcome_from_hresult(hr, "CreateDirect3D11DeviceFromDXGIDevice");
            }
            winrt_device_ = inspectable.as<wgdx11::IDirect3DDevice>();

            auto interop = winrt::get_activation_factory<wgc::GraphicsCaptureItem,
                                                         ::IGraphicsCaptureItemInterop>();
            hr = interop->CreateForWindow(window_, winrt::guid_of<wgc::GraphicsCaptureItem>(),
                                          winrt::put_abi(item_));
            if (FAILED(hr)) {
                return outcome_from_hresult(hr, "CreateForWindow");
            }

            content_size_ = item_.Size();
            if (content_size_.Width <= 0 || content_size_.Height <= 0) {
                return fail(Status::Unavailable, "captured window has no content yet");
            }

            layout_.width = static_cast<std::uint32_t>(content_size_.Width);
            layout_.height = static_cast<std::uint32_t>(content_size_.Height);
            layout_.format = PixelFormat::B8G8R8A8Unorm;

            TL_TRY(reserve_storage());

            frame_ready_ = CreateEventW(nullptr, FALSE, FALSE, nullptr);
            if (frame_ready_ == nullptr) {
                return fail(Status::Unavailable, "CreateEventW");
            }

            pool_ = wgc::Direct3D11CaptureFramePool::CreateFreeThreaded(
                winrt_device_, wgdx::DirectXPixelFormat::B8G8R8A8UIntNormalized,
                static_cast<std::int32_t>(kFramePoolDepth), content_size_);

            frame_token_ = pool_.FrameArrived({this, &GraphicsCaptureSource::on_frame_arrived});
            closed_token_ = item_.Closed({this, &GraphicsCaptureSource::on_item_closed});

            session_ = pool_.CreateCaptureSession(item_);
            if (property_present(L"Windows.Graphics.Capture.GraphicsCaptureSession",
                                 L"IsCursorCaptureEnabled")) {
                session_.IsCursorCaptureEnabled(options_.include_cursor);
            }
            if (property_present(L"Windows.Graphics.Capture.GraphicsCaptureSession",
                                 L"IsBorderRequired")) {
                try {
                    session_.IsBorderRequired(options_.draw_border);
                } catch (...) {
                }
            }
            session_.StartCapture();
        } catch (const winrt::hresult_error& error) {
            return outcome_from_hresult(error.code(), "graphics capture start");
        } catch (...) {
            return fail(Status::Unknown, "graphics capture start");
        }

        frame_index_ = 0;
        leased_ = false;
        return ok();
    }

    Outcome reserve_storage() noexcept
    {
        const std::size_t dirty_bytes =
            static_cast<std::size_t>(options_.max_dirty_rects) * (sizeof(Rect) + 4 * sizeof(int)) +
            static_cast<std::size_t>(options_.max_move_rects) * sizeof(MoveRect);

        if (!storage_.reserve(dirty_bytes + kArenaSlackBytes)) {
            return fail(Status::OutOfMemory, "graphics capture arena");
        }

        LinearArena& arena = storage_.arena();
        DirtyRegionBuilder::Limits limits;
        limits.max_dirty_rects = options_.max_dirty_rects;
        limits.max_move_rects = options_.max_move_rects;
        if (!dirty_.initialize(arena, layout_.width, layout_.height, limits)) {
            return fail(Status::OutOfMemory, "graphics capture dirty region");
        }

        TL_TRY(ring_.initialize(arena, device_.device(), kRingCapacity));
        ring_.set_layout(layout_);
        arena.update_high_water();
        return ok();
    }

    void teardown_capture() noexcept
    {
        try {
            if (session_ != nullptr) {
                session_.Close();
                session_ = nullptr;
            }
            if (pool_ != nullptr) {
                pool_.Close();
            }
            if (pool_ != nullptr && frame_token_) {
                pool_.FrameArrived(frame_token_);
                frame_token_ = {};
            }
            pool_ = nullptr;
            if (item_ != nullptr && closed_token_) {
                item_.Closed(closed_token_);
                closed_token_ = {};
            }
            item_ = nullptr;
            winrt_device_ = nullptr;
        } catch (...) {
        }
    }

    void on_frame_arrived(const wgc::Direct3D11CaptureFramePool&,
                          const winrt::Windows::Foundation::IInspectable&)
    {
        if (frame_ready_ != nullptr) {
            SetEvent(frame_ready_);
        }
    }

    void on_item_closed(const wgc::GraphicsCaptureItem&,
                        const winrt::Windows::Foundation::IInspectable&)
    {
        closed_.store(true, std::memory_order_release);
        if (frame_ready_ != nullptr) {
            SetEvent(frame_ready_);
        }
    }

    Outcome pull_frame(CapturedFrame& out, std::uint32_t timeout_ms)
    {
        wgc::Direct3D11CaptureFrame frame = pool_.TryGetNextFrame();
        if (frame == nullptr) {
            const DWORD waited = WaitForSingleObject(frame_ready_, timeout_ms);
            if (waited == WAIT_TIMEOUT) {
                return fail(Status::Timeout, "no frame arrived");
            }
            if (waited != WAIT_OBJECT_0) {
                return fail(Status::Unavailable, "frame wait failed");
            }
            if (closed_.load(std::memory_order_acquire)) {
                return fail(Status::TargetGone, "captured window closed");
            }
            frame = pool_.TryGetNextFrame();
            if (frame == nullptr) {
                return fail(Status::Timeout, "no frame available");
            }
        }

        const winrt::Windows::Graphics::SizeInt32 size = frame.ContentSize();
        const Nanoseconds present_ns =
            static_cast<Nanoseconds>(frame.SystemRelativeTime().count()) * 100ull;

        if (size.Width != content_size_.Width || size.Height != content_size_.Height) {
            frame.Close();
            return resize_to(size);
        }

        auto access = frame.Surface().as<DxgiInterfaceAccess>();
        ComPtr<ID3D11Texture2D> source;
        const HRESULT hr = access->GetInterface(IID_PPV_ARGS(&source));
        if (FAILED(hr)) {
            frame.Close();
            return outcome_from_hresult(hr, "IDirect3DDxgiInterfaceAccess");
        }

        D3D11_TEXTURE2D_DESC source_description = {};
        source->GetDesc(&source_description);

        SurfaceLayout observed;
        observed.width = source_description.Width;
        observed.height = source_description.Height;
        observed.format = pixel_format_from_dxgi(source_description.Format);
        if (observed != layout_) {
            frame.Close();
            layout_ = observed;
            ring_.set_layout(layout_);
            dirty_.resize(layout_.width, layout_.height);
            last_texture_.Reset();
            return fail(Status::ConfigurationChanged, "captured surface layout changed");
        }

        ID3D11Texture2D* destination = nullptr;
        TextureHandle handle;
        const Outcome acquired = ring_.acquire(destination, handle);
        if (!acquired.ok()) {
            frame.Close();
            return acquired;
        }

        device_.context()->CopyResource(destination, source.Get());
        frame.Close();

        dirty_.begin_frame();
        dirty_.force_full_surface();
        dirty_.finish();

        last_texture_ = destination;
        leased_handle_ = handle;
        leased_ = true;

        out = CapturedFrame{};
        out.surface.memory = SurfaceMemory::GpuTexture;
        out.surface.gpu_texture = destination;
        out.surface.width = layout_.width;
        out.surface.height = layout_.height;
        out.surface.format = layout_.format;
        out.surface.rotation = SurfaceRotation::None;

        out.metadata.frame_index = frame_index_;
        out.metadata.present_time_ns = present_ns;
        out.metadata.acquire_time_ns = now_ns();
        out.metadata.accumulated_frames = 1;
        out.metadata.content_changed = true;
        out.metadata.cursor_changed = false;
        out.metadata.full_surface_dirty = true;

        out.dirty_rects = dirty_.dirty_rects();
        out.move_rects = dirty_.move_rects();

        ++frame_index_;
        return ok();
    }

    Outcome resize_to(const winrt::Windows::Graphics::SizeInt32& size)
    {
        if (size.Width <= 0 || size.Height <= 0) {
            return fail(Status::Unavailable, "captured window has no content");
        }

        content_size_ = size;
        layout_.width = static_cast<std::uint32_t>(size.Width);
        layout_.height = static_cast<std::uint32_t>(size.Height);

        pool_.Recreate(winrt_device_, wgdx::DirectXPixelFormat::B8G8R8A8UIntNormalized,
                       static_cast<std::int32_t>(kFramePoolDepth), content_size_);

        ring_.set_layout(layout_);
        dirty_.resize(layout_.width, layout_.height);
        last_texture_.Reset();
        return fail(Status::ConfigurationChanged, "captured window resized");
    }

    CaptureTarget target_;
    CaptureOptions options_;
    HWND window_ = nullptr;
    std::uint32_t process_id_ = 0;

    D3D11CaptureDevice device_;
    D3D11TextureRing ring_;
    ArenaStorage storage_;
    DirtyRegionBuilder dirty_;

    wgdx11::IDirect3DDevice winrt_device_{nullptr};
    wgc::GraphicsCaptureItem item_{nullptr};
    wgc::Direct3D11CaptureFramePool pool_{nullptr};
    wgc::GraphicsCaptureSession session_{nullptr};
    winrt::event_token frame_token_{};
    winrt::event_token closed_token_{};
    winrt::Windows::Graphics::SizeInt32 content_size_{0, 0};

    HANDLE frame_ready_ = nullptr;
    std::atomic<bool> closed_{false};

    SurfaceLayout layout_;
    ComPtr<ID3D11Texture2D> last_texture_;
    TextureHandle leased_handle_;
    std::uint64_t frame_index_ = 0;
    DWORD com_thread_ = 0;
    bool owns_com_ = false;
    bool leased_ = false;
    bool started_ = false;
};

}  // namespace

bool graphics_capture_supported() noexcept
{
    try {
        return wgc::GraphicsCaptureSession::IsSupported();
    } catch (...) {
        return false;
    }
}

Result<std::unique_ptr<CaptureSource>> create_graphics_capture_source(
    const CaptureTarget& target, const CaptureOptions& options) noexcept
{
    auto* source = new (std::nothrow) GraphicsCaptureSource(target, options);
    if (source == nullptr) {
        return Error{Status::OutOfMemory, "GraphicsCaptureSource"};
    }
    return std::unique_ptr<CaptureSource>(source);
}

}  // namespace tl::capture::win

#else

namespace tl::capture::win {

bool graphics_capture_supported() noexcept
{
    return false;
}

Result<std::unique_ptr<CaptureSource>> create_graphics_capture_source(
    const CaptureTarget&, const CaptureOptions&) noexcept
{
    return Error{Status::NotSupported, "Windows Graphics Capture was not built in"};
}

}  // namespace tl::capture::win

#endif
