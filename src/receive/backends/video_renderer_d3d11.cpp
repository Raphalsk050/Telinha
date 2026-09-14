#include <d3d11_1.h>
#include <d3dcompiler.h>
#include <dxgi1_5.h>
#include <windows.h>
#include <wrl/client.h>

#include <cstring>
#include <new>

#include "telinha/core/clock.hpp"
#include "telinha/core/log.hpp"
#include "telinha/receive/backends.hpp"

namespace tl::receive {
namespace {

using Microsoft::WRL::ComPtr;

constexpr wchar_t kWindowClass[] = L"TelinhaViewer";

constexpr char kShaderSource[] = R"(
struct VsOut {
    float4 position : SV_POSITION;
    float2 uv : TEXCOORD0;
};

VsOut vs_main(uint id : SV_VertexID)
{
    VsOut output;
    output.uv = float2((id << 1) & 2, id & 2);
    output.position = float4(output.uv * float2(2.0f, -2.0f) + float2(-1.0f, 1.0f), 0.0f, 1.0f);
    return output;
}

Texture2D<float> luma : register(t0);
Texture2D<float2> chroma : register(t1);
SamplerState linear_sampler : register(s0);

float4 ps_main(VsOut input) : SV_TARGET
{
    float y = luma.Sample(linear_sampler, input.uv);
    float2 cbcr = chroma.Sample(linear_sampler, input.uv) - 0.5f;

    y = (y - 0.0627451f) * 1.1643836f;

    float r = y + 1.7927410f * cbcr.y;
    float g = y - 0.2132486f * cbcr.x - 0.5329093f * cbcr.y;
    float b = y + 2.1124018f * cbcr.x;

    return float4(saturate(float3(r, g, b)), 1.0f);
}
)";

struct WindowState {
    bool close_requested = false;
    bool size_changed = false;
    bool fullscreen = false;
    WINDOWPLACEMENT placement = {};
    std::uint32_t width = 0;
    std::uint32_t height = 0;
};

void set_window_fullscreen(HWND window, WindowState& state, bool enabled) noexcept
{
    if (window == nullptr || enabled == state.fullscreen) {
        return;
    }

    if (enabled) {
        MONITORINFO monitor = {};
        monitor.cbSize = sizeof(monitor);
        state.placement.length = sizeof(state.placement);
        if (GetWindowPlacement(window, &state.placement) == FALSE ||
            GetMonitorInfoW(MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST), &monitor) ==
                FALSE) {
            return;
        }
        SetWindowLongPtrW(window, GWL_STYLE, static_cast<LONG_PTR>(WS_POPUP | WS_VISIBLE));
        SetWindowPos(window, HWND_TOP, monitor.rcMonitor.left, monitor.rcMonitor.top,
                     monitor.rcMonitor.right - monitor.rcMonitor.left,
                     monitor.rcMonitor.bottom - monitor.rcMonitor.top,
                     SWP_FRAMECHANGED | SWP_NOOWNERZORDER | SWP_SHOWWINDOW);
    } else {
        SetWindowLongPtrW(window, GWL_STYLE,
                          static_cast<LONG_PTR>(WS_OVERLAPPEDWINDOW | WS_VISIBLE));
        SetWindowPlacement(window, &state.placement);
        SetWindowPos(window, nullptr, 0, 0, 0, 0,
                     SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED);
    }
    state.fullscreen = enabled;
}

LRESULT CALLBACK window_procedure(HWND window, UINT message, WPARAM wparam, LPARAM lparam)
{
    auto* state = reinterpret_cast<WindowState*>(GetWindowLongPtrW(window, GWLP_USERDATA));

    switch (message) {
        case WM_CREATE: {
            const auto* create = reinterpret_cast<const CREATESTRUCTW*>(lparam);
            SetWindowLongPtrW(window, GWLP_USERDATA,
                              reinterpret_cast<LONG_PTR>(create->lpCreateParams));
            return 0;
        }
        case WM_CLOSE:
            if (state != nullptr) {
                state->close_requested = true;
            }
            return 0;
        case WM_DESTROY: PostQuitMessage(0); return 0;
        case WM_SIZE:
            if (state != nullptr && wparam != SIZE_MINIMIZED) {
                state->width = static_cast<std::uint32_t>(LOWORD(lparam));
                state->height = static_cast<std::uint32_t>(HIWORD(lparam));
                state->size_changed = true;
            }
            return 0;
        case WM_KEYDOWN:
            if (state != nullptr && wparam == VK_F11) {
                set_window_fullscreen(window, *state, !state->fullscreen);
            } else if (state != nullptr && wparam == VK_ESCAPE) {
                set_window_fullscreen(window, *state, false);
            }
            return 0;
        case WM_SYSKEYDOWN:
            if (state != nullptr && wparam == VK_RETURN) {
                set_window_fullscreen(window, *state, !state->fullscreen);
                return 0;
            }
            break;
        case WM_SYSCHAR:
            if (wparam == VK_RETURN) {
                return 0;
            }
            break;
        case WM_LBUTTONDBLCLK:
            if (state != nullptr) {
                set_window_fullscreen(window, *state, !state->fullscreen);
            }
            return 0;
        default: break;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}

class D3D11VideoRenderer final : public VideoRenderer {
public:
    explicit D3D11VideoRenderer(const VideoRendererConfig& config) noexcept : config_(config) {}

    ~D3D11VideoRenderer() override { stop(); }

    Outcome start() override
    {
        TL_TRY(create_window());
        TL_TRY(create_device());
        TL_TRY(create_swapchain());
        TL_TRY(create_pipeline());
        stats_.reset();
        started_ = true;
        return ok();
    }

    void stop() noexcept override
    {
        started_ = false;
        render_target_.Reset();
        swapchain_.Reset();
        chroma_view_.Reset();
        luma_view_.Reset();
        staging_.Reset();
        sampler_.Reset();
        pixel_shader_.Reset();
        vertex_shader_.Reset();
        context_.Reset();
        device_.Reset();
        if (window_ != nullptr) {
            DestroyWindow(window_);
            window_ = nullptr;
        }
        if (class_registered_) {
            UnregisterClassW(kWindowClass, GetModuleHandleW(nullptr));
            class_registered_ = false;
        }
    }

    Outcome pump(bool& close_requested) noexcept override
    {
        close_requested = false;
        if (!started_) {
            return fail(Status::Unavailable, "D3D11VideoRenderer::pump");
        }

        MSG message;
        while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE) != 0) {
            if (message.message == WM_QUIT) {
                state_.close_requested = true;
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        if (state_.size_changed) {
            state_.size_changed = false;
            const Outcome resized = resize_swapchain();
            if (!resized.ok()) {
                return resized;
            }
        }

        close_requested = state_.close_requested;
        return ok();
    }

    Outcome present(const DecodedVideoFrame& frame) override
    {
        if (!started_) {
            return fail(Status::Unavailable, "D3D11VideoRenderer::present");
        }
        if (frame.format != capture::PixelFormat::NV12) {
            return fail(Status::NotSupported, "D3D11VideoRenderer::present: formato");
        }

        const Nanoseconds begin = now_ns();

        TL_TRY(bind_source(frame));

        D3D11_VIEWPORT viewport = {};
        compute_viewport(frame.width, frame.height, viewport);

        const float clear[4] = {0.0f, 0.0f, 0.0f, 1.0f};
        ID3D11RenderTargetView* targets[1] = {render_target_.Get()};
        context_->OMSetRenderTargets(1, targets, nullptr);
        context_->ClearRenderTargetView(render_target_.Get(), clear);
        context_->RSSetViewports(1, &viewport);

        context_->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        context_->IASetInputLayout(nullptr);
        context_->VSSetShader(vertex_shader_.Get(), nullptr, 0);
        context_->PSSetShader(pixel_shader_.Get(), nullptr, 0);

        ID3D11ShaderResourceView* views[2] = {luma_view_.Get(), chroma_view_.Get()};
        ID3D11SamplerState* samplers[1] = {sampler_.Get()};
        context_->PSSetShaderResources(0, 2, views);
        context_->PSSetSamplers(0, 1, samplers);

        context_->Draw(3, 0);

        const UINT interval = config_.vertical_sync ? 1u : 0u;
        UINT flags = 0;
        if (!config_.vertical_sync && tearing_supported_) {
            flags |= DXGI_PRESENT_ALLOW_TEARING;
        }

        const HRESULT presented = swapchain_->Present(interval, flags);

        ID3D11ShaderResourceView* cleared[2] = {nullptr, nullptr};
        context_->PSSetShaderResources(0, 2, cleared);

        if (presented == DXGI_ERROR_DEVICE_REMOVED || presented == DXGI_ERROR_DEVICE_RESET) {
            ++stats_.swapchain_resets;
            return fail(Status::DeviceLost, "D3D11VideoRenderer::present", presented);
        }
        if (FAILED(presented)) {
            return fail(Status::PlatformError, "D3D11VideoRenderer::present", presented);
        }

        const Nanoseconds end = now_ns();
        stats_.present_ns.record(end - begin);
        if (last_present_ns_ != 0) {
            stats_.present_interval_ns.record(end - last_present_ns_);
        }
        last_present_ns_ = end;
        ++stats_.frames_presented;
        return ok();
    }

    VideoRendererInfo info() const noexcept override
    {
        VideoRendererInfo result;
        result.backend = RendererBackend::Direct3D11;
        result.width = state_.width != 0 ? state_.width : config_.width;
        result.height = state_.height != 0 ? state_.height : config_.height;
        result.native_device = device_.Get();
        result.accepts_gpu_surfaces = true;
        return result;
    }

    const RendererStats& stats() const noexcept override { return stats_; }

    void set_fullscreen(bool enabled) noexcept override
    {
        set_window_fullscreen(window_, state_, enabled);
    }

    bool fullscreen() const noexcept override { return state_.fullscreen; }

private:
    Outcome create_window()
    {
        const HINSTANCE instance = GetModuleHandleW(nullptr);

        WNDCLASSEXW description = {};
        description.cbSize = sizeof(description);
        description.style = CS_HREDRAW | CS_VREDRAW | CS_OWNDC | CS_DBLCLKS;
        description.lpfnWndProc = window_procedure;
        description.hInstance = instance;
        description.hCursor = LoadCursorW(nullptr, IDC_ARROW);
        description.hbrBackground = static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH));
        description.lpszClassName = kWindowClass;

        if (RegisterClassExW(&description) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
            return fail(Status::PlatformError, "create_window: RegisterClassExW",
                        static_cast<std::int32_t>(GetLastError()));
        }
        class_registered_ = true;

        wchar_t title[kWindowTitleCapacity] = {};
        const char* source = config_.title[0] == '\0' ? "Telinha" : config_.title;
        MultiByteToWideChar(CP_UTF8, 0, source, -1, title, kWindowTitleCapacity);

        RECT bounds = {0, 0, static_cast<LONG>(config_.width), static_cast<LONG>(config_.height)};
        AdjustWindowRect(&bounds, WS_OVERLAPPEDWINDOW, FALSE);

        state_.width = config_.width;
        state_.height = config_.height;

        window_ = CreateWindowExW(0, kWindowClass, title, WS_OVERLAPPEDWINDOW, CW_USEDEFAULT,
                                  CW_USEDEFAULT, bounds.right - bounds.left,
                                  bounds.bottom - bounds.top, nullptr, nullptr, instance, &state_);
        if (window_ == nullptr) {
            return fail(Status::PlatformError, "create_window: CreateWindowExW",
                        static_cast<std::int32_t>(GetLastError()));
        }

        ShowWindow(window_, SW_SHOW);
        if (IsWindowVisible(window_) == FALSE) {
            ShowWindow(window_, SW_SHOW);
        }
        if (config_.start_fullscreen) {
            set_window_fullscreen(window_, state_, true);
        }
        UpdateWindow(window_);
        return ok();
    }

    Outcome create_device()
    {
        UINT flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT;
        const D3D_FEATURE_LEVEL levels[] = {D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0};

        D3D_FEATURE_LEVEL achieved = D3D_FEATURE_LEVEL_11_0;
        HRESULT created =
            D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, flags, levels,
                              ARRAYSIZE(levels), D3D11_SDK_VERSION, &device_, &achieved, &context_);
        if (FAILED(created)) {
            flags &= ~static_cast<UINT>(D3D11_CREATE_DEVICE_VIDEO_SUPPORT);
            created = D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, flags, levels,
                                        ARRAYSIZE(levels), D3D11_SDK_VERSION, &device_, &achieved,
                                        &context_);
        }
        if (FAILED(created)) {
            return fail(Status::PlatformError, "create_device: D3D11CreateDevice", created);
        }

        ComPtr<ID3D10Multithread> multithread;
        if (SUCCEEDED(context_.As(&multithread))) {
            multithread->SetMultithreadProtected(TRUE);
        }
        return ok();
    }

    Outcome create_swapchain()
    {
        ComPtr<IDXGIDevice> dxgi_device;
        HRESULT result = device_.As(&dxgi_device);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_swapchain: IDXGIDevice", result);
        }

        ComPtr<IDXGIAdapter> adapter;
        result = dxgi_device->GetAdapter(&adapter);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_swapchain: GetAdapter", result);
        }

        ComPtr<IDXGIFactory2> factory;
        result = adapter->GetParent(IID_PPV_ARGS(&factory));
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_swapchain: IDXGIFactory2", result);
        }

        ComPtr<IDXGIFactory5> factory5;
        if (SUCCEEDED(factory.As(&factory5))) {
            BOOL allowed = FALSE;
            if (SUCCEEDED(factory5->CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                                                        &allowed, sizeof(allowed)))) {
                tearing_supported_ = allowed != FALSE;
            }
        }

        DXGI_SWAP_CHAIN_DESC1 description = {};
        description.Width = state_.width;
        description.Height = state_.height;
        description.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
        description.SampleDesc.Count = 1;
        description.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
        description.BufferCount = config_.swapchain_buffers < 2 ? 2 : config_.swapchain_buffers;
        description.Scaling = DXGI_SCALING_STRETCH;
        description.SwapEffect = DXGI_SWAP_EFFECT_FLIP_DISCARD;
        description.AlphaMode = DXGI_ALPHA_MODE_IGNORE;
        if (tearing_supported_ && !config_.vertical_sync) {
            description.Flags = DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING;
        }
        swapchain_flags_ = description.Flags;

        result = factory->CreateSwapChainForHwnd(device_.Get(), window_, &description, nullptr,
                                                 nullptr, &swapchain_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_swapchain: CreateSwapChainForHwnd", result);
        }

        factory->MakeWindowAssociation(window_, DXGI_MWA_NO_ALT_ENTER);
        return create_render_target();
    }

    Outcome create_render_target()
    {
        render_target_.Reset();

        ComPtr<ID3D11Texture2D> back_buffer;
        HRESULT result = swapchain_->GetBuffer(0, IID_PPV_ARGS(&back_buffer));
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_render_target: GetBuffer", result);
        }

        result = device_->CreateRenderTargetView(back_buffer.Get(), nullptr, &render_target_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_render_target: CreateRenderTargetView",
                        result);
        }
        return ok();
    }

    Outcome resize_swapchain()
    {
        if (!swapchain_ || state_.width == 0 || state_.height == 0) {
            return ok();
        }

        render_target_.Reset();
        context_->OMSetRenderTargets(0, nullptr, nullptr);

        const HRESULT result = swapchain_->ResizeBuffers(0, state_.width, state_.height,
                                                         DXGI_FORMAT_UNKNOWN, swapchain_flags_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "resize_swapchain: ResizeBuffers", result);
        }

        ++stats_.swapchain_resets;
        return create_render_target();
    }

    Outcome create_pipeline()
    {
        ComPtr<ID3DBlob> vertex_blob;
        ComPtr<ID3DBlob> pixel_blob;
        ComPtr<ID3DBlob> errors;

        UINT compile_flags = D3DCOMPILE_OPTIMIZATION_LEVEL3 | D3DCOMPILE_ENABLE_STRICTNESS;

        HRESULT result =
            D3DCompile(kShaderSource, sizeof(kShaderSource) - 1, nullptr, nullptr, nullptr,
                       "vs_main", "vs_5_0", compile_flags, 0, &vertex_blob, &errors);
        if (FAILED(result)) {
            if (errors) {
                TL_LOG_ERROR("receptor: shader de vertice falhou: %s",
                             static_cast<const char*>(errors->GetBufferPointer()));
            }
            return fail(Status::PlatformError, "create_pipeline: vs", result);
        }

        errors.Reset();
        result = D3DCompile(kShaderSource, sizeof(kShaderSource) - 1, nullptr, nullptr, nullptr,
                            "ps_main", "ps_5_0", compile_flags, 0, &pixel_blob, &errors);
        if (FAILED(result)) {
            if (errors) {
                TL_LOG_ERROR("receptor: shader de pixel falhou: %s",
                             static_cast<const char*>(errors->GetBufferPointer()));
            }
            return fail(Status::PlatformError, "create_pipeline: ps", result);
        }

        result =
            device_->CreateVertexShader(vertex_blob->GetBufferPointer(),
                                        vertex_blob->GetBufferSize(), nullptr, &vertex_shader_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_pipeline: CreateVertexShader", result);
        }

        result = device_->CreatePixelShader(pixel_blob->GetBufferPointer(),
                                            pixel_blob->GetBufferSize(), nullptr, &pixel_shader_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_pipeline: CreatePixelShader", result);
        }

        D3D11_SAMPLER_DESC sampler = {};
        sampler.Filter = D3D11_FILTER_MIN_MAG_MIP_LINEAR;
        sampler.AddressU = D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler.AddressV = D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler.AddressW = D3D11_TEXTURE_ADDRESS_CLAMP;
        sampler.MaxLOD = D3D11_FLOAT32_MAX;

        result = device_->CreateSamplerState(&sampler, &sampler_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_pipeline: CreateSamplerState", result);
        }
        return ok();
    }

    Outcome ensure_staging(std::uint32_t width, std::uint32_t height)
    {
        if (staging_ && staging_width_ == width && staging_height_ == height) {
            return ok();
        }

        luma_view_.Reset();
        chroma_view_.Reset();
        staging_.Reset();

        D3D11_TEXTURE2D_DESC description = {};
        description.Width = width;
        description.Height = height;
        description.MipLevels = 1;
        description.ArraySize = 1;
        description.Format = DXGI_FORMAT_NV12;
        description.SampleDesc.Count = 1;
        description.Usage = D3D11_USAGE_DYNAMIC;
        description.BindFlags = D3D11_BIND_SHADER_RESOURCE;
        description.CPUAccessFlags = D3D11_CPU_ACCESS_WRITE;

        const HRESULT result = device_->CreateTexture2D(&description, nullptr, &staging_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "ensure_staging: CreateTexture2D", result);
        }

        staging_width_ = width;
        staging_height_ = height;
        return create_views(staging_.Get(), 0);
    }

    Outcome create_views(ID3D11Texture2D* texture, std::uint32_t slice)
    {
        luma_view_.Reset();
        chroma_view_.Reset();

        D3D11_SHADER_RESOURCE_VIEW_DESC description = {};
        description.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2DARRAY;
        description.Texture2DArray.MipLevels = 1;
        description.Texture2DArray.ArraySize = 1;
        description.Texture2DArray.FirstArraySlice = slice;

        description.Format = DXGI_FORMAT_R8_UNORM;
        HRESULT result = device_->CreateShaderResourceView(texture, &description, &luma_view_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_views: luma", result);
        }

        description.Format = DXGI_FORMAT_R8G8_UNORM;
        result = device_->CreateShaderResourceView(texture, &description, &chroma_view_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_views: chroma", result);
        }
        return ok();
    }

    Outcome bind_source(const DecodedVideoFrame& frame)
    {
        if (frame.memory == FrameMemory::GpuTexture) {
            auto* texture = static_cast<ID3D11Texture2D*>(frame.gpu_texture);
            if (texture == nullptr) {
                return fail(Status::InvalidArgument, "bind_source: textura nula");
            }
            if (texture != bound_texture_ || frame.gpu_subresource != bound_slice_) {
                TL_TRY(create_views(texture, frame.gpu_subresource));
                bound_texture_ = texture;
                bound_slice_ = frame.gpu_subresource;
            }
            return ok();
        }

        if (frame.memory != FrameMemory::CpuPlanar || frame.plane_count < 2) {
            return fail(Status::NotSupported, "bind_source: memoria do quadro");
        }

        TL_TRY(ensure_staging(frame.width, frame.height));
        bound_texture_ = nullptr;

        D3D11_MAPPED_SUBRESOURCE mapped = {};
        const HRESULT result =
            context_->Map(staging_.Get(), 0, D3D11_MAP_WRITE_DISCARD, 0, &mapped);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "bind_source: Map", result);
        }

        auto* destination = static_cast<std::uint8_t*>(mapped.pData);
        const std::uint32_t row_bytes = frame.width;

        for (std::uint32_t row = 0; row < frame.height; ++row) {
            std::memcpy(destination + static_cast<std::size_t>(row) * mapped.RowPitch,
                        frame.plane_data[0] + static_cast<std::size_t>(row) * frame.plane_pitch[0],
                        row_bytes);
        }

        std::uint8_t* chroma_destination =
            destination + static_cast<std::size_t>(frame.height) * mapped.RowPitch;
        const std::uint32_t chroma_rows = frame.height / 2;

        for (std::uint32_t row = 0; row < chroma_rows; ++row) {
            std::memcpy(chroma_destination + static_cast<std::size_t>(row) * mapped.RowPitch,
                        frame.plane_data[1] + static_cast<std::size_t>(row) * frame.plane_pitch[1],
                        row_bytes);
        }

        context_->Unmap(staging_.Get(), 0);
        return ok();
    }

    void compute_viewport(std::uint32_t source_width, std::uint32_t source_height,
                          D3D11_VIEWPORT& viewport) const noexcept
    {
        const float target_width = static_cast<float>(state_.width);
        const float target_height = static_cast<float>(state_.height);

        if (source_width == 0 || source_height == 0) {
            viewport.Width = target_width;
            viewport.Height = target_height;
            viewport.MaxDepth = 1.0f;
            return;
        }

        const float source_aspect =
            static_cast<float>(source_width) / static_cast<float>(source_height);
        const float target_aspect = target_width / target_height;

        if (source_aspect > target_aspect) {
            viewport.Width = target_width;
            viewport.Height = target_width / source_aspect;
        } else {
            viewport.Height = target_height;
            viewport.Width = target_height * source_aspect;
        }

        viewport.TopLeftX = (target_width - viewport.Width) * 0.5f;
        viewport.TopLeftY = (target_height - viewport.Height) * 0.5f;
        viewport.MaxDepth = 1.0f;
    }

    VideoRendererConfig config_;
    RendererStats stats_;
    WindowState state_;
    HWND window_ = nullptr;

    ComPtr<ID3D11Device> device_;
    ComPtr<ID3D11DeviceContext> context_;
    ComPtr<IDXGISwapChain1> swapchain_;
    ComPtr<ID3D11RenderTargetView> render_target_;
    ComPtr<ID3D11VertexShader> vertex_shader_;
    ComPtr<ID3D11PixelShader> pixel_shader_;
    ComPtr<ID3D11SamplerState> sampler_;
    ComPtr<ID3D11Texture2D> staging_;
    ComPtr<ID3D11ShaderResourceView> luma_view_;
    ComPtr<ID3D11ShaderResourceView> chroma_view_;

    ID3D11Texture2D* bound_texture_ = nullptr;
    std::uint32_t bound_slice_ = 0xFFFFFFFFu;
    std::uint32_t staging_width_ = 0;
    std::uint32_t staging_height_ = 0;
    UINT swapchain_flags_ = 0;
    Nanoseconds last_present_ns_ = 0;
    bool tearing_supported_ = false;
    bool class_registered_ = false;
    bool started_ = false;
};

}  // namespace

Result<std::unique_ptr<VideoRenderer>> create_d3d11_video_renderer(
    const VideoRendererConfig& config)
{
    std::unique_ptr<VideoRenderer> renderer(new (std::nothrow) D3D11VideoRenderer(config));
    if (!renderer) {
        return Error{Status::OutOfMemory, "create_d3d11_video_renderer"};
    }
    return renderer;
}

}  // namespace tl::receive
