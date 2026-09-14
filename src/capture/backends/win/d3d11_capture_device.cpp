#include "d3d11_capture_device.hpp"

#include <d3d11_4.h>
#include <d3dcompiler.h>

#include <iterator>
#include <new>

#include "../cursor_shape.hpp"
#include "telinha/core/log.hpp"

namespace tl::capture::win {
namespace {

constexpr D3D_FEATURE_LEVEL kFeatureLevels[] = {
    D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_9_3,  D3D_FEATURE_LEVEL_9_1,
};

constexpr char kCursorShader[] = R"(
Texture2D<float4> shape_texture : register(t0);
Texture2D<float4> background_texture : register(t1);

cbuffer CursorParameters : register(b0)
{
    int2 origin;
    int2 extent;
    uint rotation;
    uint mode;
    uint2 reserved;
};

float4 vertex_main(uint id : SV_VertexID) : SV_Position
{
    float2 corner = float2((id << 1) & 2, id & 2);
    return float4(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

float4 pixel_main(float4 position : SV_Position) : SV_Target
{
    int2 destination = int2(position.xy);
    int2 relative = destination - origin;
    int2 texel = relative;
    if (rotation == 1u) {
        texel = int2(extent.y - 1 - relative.y, relative.x);
    } else if (rotation == 2u) {
        texel = int2(extent.x - 1 - relative.x, extent.y - 1 - relative.y);
    } else if (rotation == 3u) {
        texel = int2(relative.y, extent.x - 1 - relative.x);
    }

    float4 cursor = shape_texture.Load(int3(texel, 0));
    float4 background = background_texture.Load(int3(destination, 0));
    if (mode == 0u) {
        return float4(lerp(background.rgb, cursor.rgb, cursor.a), background.a);
    }

    uint3 screen = (uint3)(background.rgb * 255.0 + 0.5);
    uint3 value = (uint3)(cursor.rgb * 255.0 + 0.5);
    uint3 keep = cursor.a > 0.5 ? uint3(255, 255, 255) : uint3(0, 0, 0);
    return float4(float3((screen & keep) ^ value) / 255.0, background.a);
}
)";

struct CursorConstants {
    std::int32_t origin_x = 0;
    std::int32_t origin_y = 0;
    std::int32_t extent_x = 0;
    std::int32_t extent_y = 0;
    std::uint32_t rotation = 0;
    std::uint32_t mode = 0;
    std::uint32_t reserved[2] = {};
};

using ShaderCompiler = decltype(&D3DCompile);

void enable_multithread_protection(ID3D11DeviceContext* context) noexcept
{
    ComPtr<ID3D11Multithread> multithread;
    if (SUCCEEDED(context->QueryInterface(IID_PPV_ARGS(&multithread)))) {
        multithread->SetMultithreadProtected(TRUE);
    }
}

[[nodiscard]] ShaderCompiler shader_compiler() noexcept
{
    static const HMODULE library =
        LoadLibraryExW(L"d3dcompiler_47.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (library == nullptr) {
        return nullptr;
    }
    return reinterpret_cast<ShaderCompiler>(GetProcAddress(library, "D3DCompile"));
}

[[nodiscard]] Outcome compile_shader(ShaderCompiler compile, const char* entry, const char* profile,
                                     ComPtr<ID3DBlob>& out) noexcept
{
    ComPtr<ID3DBlob> errors;
    const HRESULT hr =
        compile(kCursorShader, sizeof(kCursorShader) - 1, "telinha_cursor", nullptr, nullptr, entry,
                profile, D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, &out, &errors);
    if (FAILED(hr)) {
        if (errors) {
            TL_LOG_WARN("captura: shader do cursor recusado: %.*s",
                        static_cast<int>(errors->GetBufferSize()),
                        static_cast<const char*>(errors->GetBufferPointer()));
        }
        return outcome_from_hresult(hr, "cursor: D3DCompile");
    }
    return ok();
}

[[nodiscard]] bool mask_bit(const std::byte* row, std::uint32_t x) noexcept
{
    const auto mask = static_cast<std::byte>(0x80u >> (x & 7u));
    return (row[x >> 3] & mask) != std::byte{0};
}

}  // namespace

Outcome find_output_for_monitor(HMONITOR monitor, ComPtr<IDXGIAdapter1>& adapter,
                                ComPtr<IDXGIOutput1>& output) noexcept
{
    adapter.Reset();
    output.Reset();

    ComPtr<IDXGIFactory1> factory;
    HRESULT hr = CreateDXGIFactory1(IID_PPV_ARGS(&factory));
    if (FAILED(hr)) {
        return outcome_from_hresult(hr, "CreateDXGIFactory1");
    }

    for (UINT adapter_index = 0;; ++adapter_index) {
        ComPtr<IDXGIAdapter1> candidate_adapter;
        hr = factory->EnumAdapters1(adapter_index, &candidate_adapter);
        if (hr == DXGI_ERROR_NOT_FOUND) {
            break;
        }
        if (FAILED(hr)) {
            return outcome_from_hresult(hr, "EnumAdapters1");
        }

        for (UINT output_index = 0;; ++output_index) {
            ComPtr<IDXGIOutput> candidate_output;
            hr = candidate_adapter->EnumOutputs(output_index, &candidate_output);
            if (hr == DXGI_ERROR_NOT_FOUND) {
                break;
            }
            if (FAILED(hr)) {
                return outcome_from_hresult(hr, "EnumOutputs");
            }

            DXGI_OUTPUT_DESC description = {};
            if (FAILED(candidate_output->GetDesc(&description))) {
                continue;
            }
            if (description.Monitor != monitor) {
                continue;
            }

            ComPtr<IDXGIOutput1> output_one;
            hr = candidate_output.As(&output_one);
            if (FAILED(hr)) {
                return outcome_from_hresult(hr, "IDXGIOutput1");
            }
            adapter = candidate_adapter;
            output = output_one;
            return ok();
        }
    }

    return fail(Status::TargetGone, "monitor is not attached to any adapter output");
}

Outcome D3D11CaptureDevice::create_on_adapter(IDXGIAdapter1* adapter) noexcept
{
    UINT flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
#if defined(TELINHA_ENABLE_ASSERTS)
    flags |= D3D11_CREATE_DEVICE_DEBUG;
#endif

    const D3D_DRIVER_TYPE driver_type =
        adapter != nullptr ? D3D_DRIVER_TYPE_UNKNOWN : D3D_DRIVER_TYPE_HARDWARE;

    D3D_FEATURE_LEVEL achieved = D3D_FEATURE_LEVEL_9_1;
    HRESULT hr = D3D11CreateDevice(adapter, driver_type, nullptr, flags, kFeatureLevels,
                                   static_cast<UINT>(std::size(kFeatureLevels)), D3D11_SDK_VERSION,
                                   &device_, &achieved, &context_);

#if defined(TELINHA_ENABLE_ASSERTS)
    if (hr == DXGI_ERROR_SDK_COMPONENT_MISSING || hr == E_FAIL) {
        flags &= ~static_cast<UINT>(D3D11_CREATE_DEVICE_DEBUG);
        hr = D3D11CreateDevice(adapter, driver_type, nullptr, flags, kFeatureLevels,
                               static_cast<UINT>(std::size(kFeatureLevels)), D3D11_SDK_VERSION,
                               &device_, &achieved, &context_);
    }
#endif

    if (FAILED(hr)) {
        destroy();
        return outcome_from_hresult(hr, "D3D11CreateDevice");
    }

    enable_multithread_protection(context_.Get());
    return ok();
}

Outcome D3D11CaptureDevice::create_for_monitor(HMONITOR monitor) noexcept
{
    destroy();
    TL_TRY(find_output_for_monitor(monitor, adapter_, output_));
    return create_on_adapter(adapter_.Get());
}

Outcome D3D11CaptureDevice::create_default() noexcept
{
    destroy();
    return create_on_adapter(nullptr);
}

void D3D11CaptureDevice::destroy() noexcept
{
    output_.Reset();
    adapter_.Reset();
    if (context_) {
        context_->ClearState();
        context_->Flush();
    }
    context_.Reset();
    device_.Reset();
}

Outcome D3D11TextureRing::initialize(LinearArena& arena, ID3D11Device* device,
                                     std::uint32_t capacity) noexcept
{
    if (device == nullptr || capacity == 0) {
        return fail(Status::InvalidArgument, "texture ring");
    }

    textures_ = arena.allocate_array<ID3D11Texture2D*>(capacity);
    if (textures_ == nullptr) {
        return fail(Status::OutOfMemory, "texture ring storage");
    }
    for (std::uint32_t i = 0; i < capacity; ++i) {
        textures_[i] = nullptr;
    }
    if (!policy_.initialize(arena, capacity)) {
        textures_ = nullptr;
        return fail(Status::OutOfMemory, "texture ring policy");
    }

    device_ = device;
    return ok();
}

void D3D11TextureRing::destroy() noexcept
{
    if (textures_ != nullptr) {
        for (std::uint32_t i = 0; i < policy_.capacity(); ++i) {
            if (textures_[i] != nullptr) {
                textures_[i]->Release();
                textures_[i] = nullptr;
            }
        }
    }
    device_ = nullptr;
}

void D3D11TextureRing::set_layout(const SurfaceLayout& layout) noexcept
{
    policy_.set_layout(layout);
}

void D3D11TextureRing::retire(std::uint32_t slot) noexcept
{
    if (textures_ != nullptr && slot < policy_.capacity() && textures_[slot] != nullptr) {
        textures_[slot]->Release();
        textures_[slot] = nullptr;
    }
}

Outcome D3D11TextureRing::acquire(ID3D11Texture2D*& out, TextureHandle& handle) noexcept
{
    out = nullptr;
    handle = TextureHandle{};

    if (device_ == nullptr) {
        return fail(Status::Unavailable, "texture ring uninitialized");
    }

    const TextureLease lease = policy_.lease();
    if (lease.outcome == LeaseOutcome::Exhausted) {
        return fail(Status::Full, "texture ring exhausted");
    }

    if (lease.retire_stale) {
        retire(lease.handle.index);
    }

    if (lease.outcome == LeaseOutcome::NeedsCreation) {
        const SurfaceLayout& layout = policy_.layout();

        D3D11_TEXTURE2D_DESC description = {};
        description.Width = layout.width;
        description.Height = layout.height;
        description.MipLevels = 1;
        description.ArraySize = 1;
        description.Format = dxgi_format_from_pixel_format(layout.format);
        description.SampleDesc.Count = 1;
        description.SampleDesc.Quality = 0;
        description.Usage = D3D11_USAGE_DEFAULT;
        description.BindFlags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;
        description.CPUAccessFlags = 0;
        description.MiscFlags = 0;

        if (description.Format == DXGI_FORMAT_UNKNOWN) {
            (void)policy_.give_back(lease.handle);
            return fail(Status::NotSupported, "texture ring pixel format");
        }

        ID3D11Texture2D* created = nullptr;
        const HRESULT hr = device_->CreateTexture2D(&description, nullptr, &created);
        if (FAILED(hr)) {
            (void)policy_.give_back(lease.handle);
            return outcome_from_hresult(hr, "CreateTexture2D");
        }

        retire(lease.handle.index);
        textures_[lease.handle.index] = created;
        policy_.mark_created(lease.handle);
    }

    out = textures_[lease.handle.index];
    handle = lease.handle;
    return ok();
}

void D3D11TextureRing::give_back(TextureHandle handle) noexcept
{
    (void)policy_.give_back(handle);
}

Outcome D3D11CursorCompositor::prepare(ID3D11Device* device, PixelFormat format) noexcept
{
    if (device == nullptr) {
        destroy();
        return fail(Status::InvalidArgument, "cursor: sem dispositivo");
    }

    if (device_.Get() != device) {
        destroy();
        const Outcome created = create_pipeline(device);
        if (!created.ok()) {
            destroy();
            return created;
        }
    }

    if (format != PixelFormat::B8G8R8A8Unorm && format != PixelFormat::R8G8B8A8Unorm) {
        format_ = PixelFormat::Unknown;
        return fail(Status::NotSupported, "cursor: formato de tela sem suporte");
    }
    format_ = format;
    return ok();
}

Outcome D3D11CursorCompositor::create_pipeline(ID3D11Device* device) noexcept
{
    if (device->GetFeatureLevel() < D3D_FEATURE_LEVEL_10_0) {
        return fail(Status::NotSupported, "cursor: a placa de video nao tem shader model 4");
    }

    const ShaderCompiler compile = shader_compiler();
    if (compile == nullptr) {
        return fail(Status::NotSupported, "cursor: d3dcompiler_47.dll indisponivel");
    }

    ComPtr<ID3DBlob> vertex_code;
    ComPtr<ID3DBlob> pixel_code;
    TL_TRY(compile_shader(compile, "vertex_main", "vs_4_0", vertex_code));
    TL_TRY(compile_shader(compile, "pixel_main", "ps_4_0", pixel_code));

    HRESULT hr = device->CreateVertexShader(vertex_code->GetBufferPointer(),
                                            vertex_code->GetBufferSize(), nullptr, &vertex_shader_);
    if (FAILED(hr)) {
        return outcome_from_hresult(hr, "cursor: CreateVertexShader");
    }
    hr = device->CreatePixelShader(pixel_code->GetBufferPointer(), pixel_code->GetBufferSize(),
                                   nullptr, &pixel_shader_);
    if (FAILED(hr)) {
        return outcome_from_hresult(hr, "cursor: CreatePixelShader");
    }

    D3D11_BUFFER_DESC buffer = {};
    buffer.ByteWidth = static_cast<UINT>(sizeof(CursorConstants));
    buffer.Usage = D3D11_USAGE_DEFAULT;
    buffer.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
    hr = device->CreateBuffer(&buffer, nullptr, &constants_);
    if (FAILED(hr)) {
        return outcome_from_hresult(hr, "cursor: CreateBuffer");
    }

    D3D11_TEXTURE2D_DESC description = {};
    description.Width = kMaxExtent;
    description.Height = kMaxExtent;
    description.MipLevels = 1;
    description.ArraySize = 1;
    description.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
    description.SampleDesc.Count = 1;
    description.Usage = D3D11_USAGE_DEFAULT;
    description.BindFlags = D3D11_BIND_SHADER_RESOURCE;
    hr = device->CreateTexture2D(&description, nullptr, &shape_);
    if (FAILED(hr)) {
        return outcome_from_hresult(hr, "cursor: CreateTexture2D");
    }
    hr = device->CreateShaderResourceView(shape_.Get(), nullptr, &shape_view_);
    if (FAILED(hr)) {
        return outcome_from_hresult(hr, "cursor: CreateShaderResourceView");
    }

    scratch_.reset(new (std::nothrow)
                       std::byte[static_cast<std::size_t>(kMaxExtent) * kMaxExtent * 4u]);
    if (!scratch_) {
        return fail(Status::OutOfMemory, "cursor: sem memoria para o formato do ponteiro");
    }

    device->GetImmediateContext(context_.ReleaseAndGetAddressOf());
    device_ = device;
    return ok();
}

void D3D11CursorCompositor::destroy() noexcept
{
    shape_view_.Reset();
    shape_.Reset();
    constants_.Reset();
    pixel_shader_.Reset();
    vertex_shader_.Reset();
    context_.Reset();
    device_.Reset();
    scratch_.reset();
    format_ = PixelFormat::Unknown;
    shape_width_ = 0;
    shape_height_ = 0;
    blend_mode_ = 0;
}

Outcome D3D11CursorCompositor::upload_shape(CursorShapeKind kind, std::uint32_t width,
                                            std::uint32_t reported_height, std::uint32_t pitch,
                                            Span<const std::byte> source) noexcept
{
    shape_width_ = 0;
    shape_height_ = 0;

    if (!ready()) {
        return fail(Status::Unavailable, "cursor: compositor sem preparo");
    }

    const std::uint32_t height = cursor_visible_height(kind, reported_height);
    if (kind == CursorShapeKind::None || width == 0 || height == 0 || width > kMaxExtent ||
        height > kMaxExtent || source.data() == nullptr) {
        return fail(Status::OutOfRange, "cursor: formato de ponteiro fora do limite");
    }
    if (source.size() < static_cast<std::size_t>(pitch) * reported_height) {
        return fail(Status::OutOfRange, "cursor: formato de ponteiro truncado");
    }

    const void* pixels = source.data();
    UINT row_pitch = pitch;
    if (kind == CursorShapeKind::Monochrome) {
        if (pitch < (width + 7u) / 8u) {
            return fail(Status::InvalidArgument, "cursor: passo invalido no formato do ponteiro");
        }
        for (std::uint32_t y = 0; y < height; ++y) {
            const std::byte* and_row = source.data() + static_cast<std::size_t>(y) * pitch;
            const std::byte* xor_row = source.data() + static_cast<std::size_t>(y + height) * pitch;
            std::byte* texel = scratch_.get() + static_cast<std::size_t>(y) * width * 4u;
            for (std::uint32_t x = 0; x < width; ++x) {
                const std::byte value = mask_bit(xor_row, x) ? std::byte{0xFF} : std::byte{0x00};
                texel[0] = value;
                texel[1] = value;
                texel[2] = value;
                texel[3] = mask_bit(and_row, x) ? std::byte{0xFF} : std::byte{0x00};
                texel += 4;
            }
        }
        pixels = scratch_.get();
        row_pitch = width * 4u;
    } else if (pitch < width * 4u) {
        return fail(Status::InvalidArgument, "cursor: passo invalido no formato do ponteiro");
    }

    const D3D11_BOX box = {0, 0, 0, width, height, 1};
    context_->UpdateSubresource(shape_.Get(), 0, &box, pixels, row_pitch, 0);

    shape_width_ = width;
    shape_height_ = height;
    blend_mode_ = kind == CursorShapeKind::Color ? 0u : 1u;
    return ok();
}

Outcome D3D11CursorCompositor::draw(ID3D11Texture2D* target, ID3D11ShaderResourceView* background,
                                    const CursorPlacement& placement) noexcept
{
    const Rect& area = placement.area;
    const Rect& visible = placement.visible;
    if (!ready() || !has_shape() || target == nullptr || background == nullptr) {
        return fail(Status::Unavailable, "cursor: nada para desenhar");
    }
    if (visible.empty() || visible.left < 0 || visible.top < 0 ||
        area.width() > static_cast<std::int32_t>(kMaxExtent) ||
        area.height() > static_cast<std::int32_t>(kMaxExtent)) {
        return fail(Status::OutOfRange, "cursor: area fora do limite");
    }

    ComPtr<ID3D11RenderTargetView> view;
    const HRESULT hr = device_->CreateRenderTargetView(target, nullptr, &view);
    if (FAILED(hr)) {
        return outcome_from_hresult(hr, "cursor: CreateRenderTargetView");
    }

    CursorConstants constants;
    constants.origin_x = area.left;
    constants.origin_y = area.top;
    constants.extent_x = area.width();
    constants.extent_y = area.height();
    constants.rotation = static_cast<std::uint32_t>(placement.rotation);
    constants.mode = blend_mode_;
    context_->UpdateSubresource(constants_.Get(), 0, nullptr, &constants, 0, 0);

    D3D11_VIEWPORT viewport = {};
    viewport.TopLeftX = static_cast<FLOAT>(visible.left);
    viewport.TopLeftY = static_cast<FLOAT>(visible.top);
    viewport.Width = static_cast<FLOAT>(visible.width());
    viewport.Height = static_cast<FLOAT>(visible.height());
    viewport.MinDepth = 0.0f;
    viewport.MaxDepth = 1.0f;

    ID3D11RenderTargetView* const targets[1] = {view.Get()};
    ID3D11ShaderResourceView* const inputs[2] = {shape_view_.Get(), background};
    ID3D11Buffer* const buffers[1] = {constants_.Get()};

    context_->OMSetRenderTargets(1, targets, nullptr);
    context_->OMSetBlendState(nullptr, nullptr, 0xFFFFFFFFu);
    context_->OMSetDepthStencilState(nullptr, 0);
    context_->RSSetState(nullptr);
    context_->RSSetViewports(1, &viewport);
    context_->IASetInputLayout(nullptr);
    context_->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    context_->VSSetShader(vertex_shader_.Get(), nullptr, 0);
    context_->PSSetShader(pixel_shader_.Get(), nullptr, 0);
    context_->PSSetConstantBuffers(0, 1, buffers);
    context_->PSSetShaderResources(0, 2, inputs);
    context_->Draw(3, 0);

    ID3D11ShaderResourceView* const released[2] = {nullptr, nullptr};
    context_->PSSetShaderResources(0, 2, released);
    context_->OMSetRenderTargets(0, nullptr, nullptr);
    return ok();
}

}  // namespace tl::capture::win
