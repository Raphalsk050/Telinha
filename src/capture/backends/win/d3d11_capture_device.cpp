#include "d3d11_capture_device.hpp"

#include <d3d11_4.h>

#include <iterator>

namespace tl::capture::win {
namespace {

constexpr D3D_FEATURE_LEVEL kFeatureLevels[] = {
    D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1,
    D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_9_3,  D3D_FEATURE_LEVEL_9_1,
};

void enable_multithread_protection(ID3D11DeviceContext* context) noexcept
{
    ComPtr<ID3D11Multithread> multithread;
    if (SUCCEEDED(context->QueryInterface(IID_PPV_ARGS(&multithread)))) {
        multithread->SetMultithreadProtected(TRUE);
    }
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
        description.BindFlags = D3D11_BIND_SHADER_RESOURCE;
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

}  // namespace tl::capture::win
