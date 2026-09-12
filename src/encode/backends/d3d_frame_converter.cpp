#include "d3d_frame_converter.hpp"

#if TL_PLATFORM_WINDOWS

namespace tl::encode {
namespace {

using Microsoft::WRL::ComPtr;

[[nodiscard]] Outcome from_hresult(HRESULT hr, const char* context) noexcept
{
    return hr >= 0 ? ok() : fail(Status::PlatformError, context, static_cast<std::int32_t>(hr));
}

[[nodiscard]] D3D11_VIDEO_PROCESSOR_ROTATION to_processor_rotation(
    capture::SurfaceRotation rotation) noexcept
{
    switch (rotation) {
        case capture::SurfaceRotation::Clockwise90: return D3D11_VIDEO_PROCESSOR_ROTATION_90;
        case capture::SurfaceRotation::Clockwise180: return D3D11_VIDEO_PROCESSOR_ROTATION_180;
        case capture::SurfaceRotation::Clockwise270: return D3D11_VIDEO_PROCESSOR_ROTATION_270;
        case capture::SurfaceRotation::None: break;
    }
    return D3D11_VIDEO_PROCESSOR_ROTATION_IDENTITY;
}

}  // namespace

D3dFrameConverter::~D3dFrameConverter()
{
    shutdown();
}

Outcome D3dFrameConverter::initialize(ID3D11Device* device, std::uint32_t output_width,
                                      std::uint32_t output_height,
                                      std::uint32_t surface_count) noexcept
{
    shutdown();

    if (device == nullptr || output_width == 0 || output_height == 0 || surface_count == 0 ||
        surface_count > kMaxOutputSurfaces) {
        return fail(Status::InvalidArgument, "D3dFrameConverter::initialize");
    }
    if ((output_width & 1u) != 0 || (output_height & 1u) != 0) {
        return fail(Status::InvalidArgument,
                    "D3dFrameConverter::initialize: NV12 needs even dimensions");
    }

    device_ = device;
    device_->GetImmediateContext(context_.GetAddressOf());

    HRESULT hr = device_.As(&video_device_);
    if (hr < 0) {
        shutdown();
        return from_hresult(hr, "D3dFrameConverter: ID3D11VideoDevice unavailable");
    }
    hr = context_.As(&video_context_);
    if (hr < 0) {
        shutdown();
        return from_hresult(hr, "D3dFrameConverter: ID3D11VideoContext unavailable");
    }

    D3D11_TEXTURE2D_DESC desc{};
    desc.Width = output_width;
    desc.Height = output_height;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_NV12;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_DEFAULT;
    desc.BindFlags = D3D11_BIND_RENDER_TARGET;

    for (std::uint32_t i = 0; i < surface_count; ++i) {
        hr = device_->CreateTexture2D(&desc, nullptr, outputs_[i].ReleaseAndGetAddressOf());
        if (hr < 0) {
            shutdown();
            return from_hresult(hr, "D3dFrameConverter: NV12 surface creation failed");
        }
    }

    output_width_ = output_width;
    output_height_ = output_height;
    surface_count_ = surface_count;
    next_surface_ = 0;
    return ok();
}

void D3dFrameConverter::shutdown() noexcept
{
    for (std::uint32_t i = 0; i < kMaxOutputSurfaces; ++i) {
        output_views_[i].Reset();
        outputs_[i].Reset();
    }
    processor_.Reset();
    enumerator_.Reset();
    video_context_.Reset();
    video_device_.Reset();
    context_.Reset();
    device_.Reset();

    output_width_ = 0;
    output_height_ = 0;
    surface_count_ = 0;
    next_surface_ = 0;
    enumerated_width_ = 0;
    enumerated_height_ = 0;
}

Outcome D3dFrameConverter::ensure_enumerator(std::uint32_t source_width,
                                             std::uint32_t source_height) noexcept
{
    if (processor_ != nullptr && enumerated_width_ == source_width &&
        enumerated_height_ == source_height) {
        return ok();
    }

    for (std::uint32_t i = 0; i < kMaxOutputSurfaces; ++i) {
        output_views_[i].Reset();
    }
    processor_.Reset();
    enumerator_.Reset();

    D3D11_VIDEO_PROCESSOR_CONTENT_DESC content{};
    content.InputFrameFormat = D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE;
    content.InputWidth = source_width;
    content.InputHeight = source_height;
    content.OutputWidth = output_width_;
    content.OutputHeight = output_height_;
    content.Usage = D3D11_VIDEO_USAGE_PLAYBACK_NORMAL;

    HRESULT hr = video_device_->CreateVideoProcessorEnumerator(
        &content, enumerator_.ReleaseAndGetAddressOf());
    if (hr < 0) {
        return from_hresult(hr, "D3dFrameConverter: processor enumerator creation failed");
    }

    hr = video_device_->CreateVideoProcessor(enumerator_.Get(), 0,
                                             processor_.ReleaseAndGetAddressOf());
    if (hr < 0) {
        enumerator_.Reset();
        return from_hresult(hr, "D3dFrameConverter: video processor creation failed");
    }

    video_context_->VideoProcessorSetStreamFrameFormat(processor_.Get(), 0,
                                                       D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
    video_context_->VideoProcessorSetStreamAutoProcessingMode(processor_.Get(), 0, FALSE);
    video_context_->VideoProcessorSetStreamOutputRate(
        processor_.Get(), 0, D3D11_VIDEO_PROCESSOR_OUTPUT_RATE_NORMAL, FALSE, nullptr);

    ComPtr<ID3D11VideoContext1> video_context1;
    if (video_context_.As(&video_context1) >= 0) {
        video_context1->VideoProcessorSetStreamColorSpace1(processor_.Get(), 0,
                                                           DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
        video_context1->VideoProcessorSetOutputColorSpace1(
            processor_.Get(), DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709);
    } else {
        D3D11_VIDEO_PROCESSOR_COLOR_SPACE input_space{};
        input_space.RGB_Range = 0;
        input_space.YCbCr_Matrix = 1;
        input_space.Nominal_Range = static_cast<UINT>(D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_0_255);
        video_context_->VideoProcessorSetStreamColorSpace(processor_.Get(), 0, &input_space);

        D3D11_VIDEO_PROCESSOR_COLOR_SPACE output_space{};
        output_space.RGB_Range = 1;
        output_space.YCbCr_Matrix = 1;
        output_space.Nominal_Range = static_cast<UINT>(D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_16_235);
        video_context_->VideoProcessorSetOutputColorSpace(processor_.Get(), &output_space);
    }

    const RECT output_rect{0, 0, static_cast<LONG>(output_width_),
                           static_cast<LONG>(output_height_)};
    video_context_->VideoProcessorSetStreamDestRect(processor_.Get(), 0, TRUE, &output_rect);
    video_context_->VideoProcessorSetOutputTargetRect(processor_.Get(), TRUE, &output_rect);

    enumerated_width_ = source_width;
    enumerated_height_ = source_height;
    return ok();
}

Outcome D3dFrameConverter::convert(ID3D11Texture2D* source, capture::SurfaceRotation rotation,
                                   std::uint32_t& out_surface_index) noexcept
{
    out_surface_index = 0;

    if (source == nullptr) {
        return fail(Status::InvalidArgument, "D3dFrameConverter::convert: null source");
    }
    if (video_device_ == nullptr || surface_count_ == 0) {
        return fail(Status::Unavailable, "D3dFrameConverter::convert: not initialized");
    }

    D3D11_TEXTURE2D_DESC source_desc{};
    source->GetDesc(&source_desc);
    TL_TRY(ensure_enumerator(source_desc.Width, source_desc.Height));

    const std::uint32_t slot = next_surface_;
    next_surface_ = (next_surface_ + 1) % surface_count_;

    if (output_views_[slot] == nullptr) {
        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC view_desc{};
        view_desc.ViewDimension = D3D11_VPOV_DIMENSION_TEXTURE2D;
        const HRESULT hr = video_device_->CreateVideoProcessorOutputView(
            outputs_[slot].Get(), enumerator_.Get(), &view_desc,
            output_views_[slot].ReleaseAndGetAddressOf());
        if (hr < 0) {
            return from_hresult(hr, "D3dFrameConverter: output view creation failed");
        }
    }

    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC input_desc{};
    input_desc.ViewDimension = D3D11_VPIV_DIMENSION_TEXTURE2D;
    input_desc.Texture2D.MipSlice = 0;
    input_desc.Texture2D.ArraySlice = 0;

    ComPtr<ID3D11VideoProcessorInputView> input_view;
    HRESULT hr = video_device_->CreateVideoProcessorInputView(
        source, enumerator_.Get(), &input_desc, input_view.GetAddressOf());
    if (hr < 0) {
        return from_hresult(hr, "D3dFrameConverter: input view creation failed");
    }

    const D3D11_VIDEO_PROCESSOR_ROTATION processor_rotation = to_processor_rotation(rotation);
    video_context_->VideoProcessorSetStreamRotation(
        processor_.Get(), 0, processor_rotation != D3D11_VIDEO_PROCESSOR_ROTATION_IDENTITY,
        processor_rotation);

    const RECT source_rect{0, 0, static_cast<LONG>(source_desc.Width),
                           static_cast<LONG>(source_desc.Height)};
    video_context_->VideoProcessorSetStreamSourceRect(processor_.Get(), 0, TRUE, &source_rect);

    D3D11_VIDEO_PROCESSOR_STREAM stream{};
    stream.Enable = TRUE;
    stream.OutputIndex = 0;
    stream.InputFrameOrField = 0;
    stream.pInputSurface = input_view.Get();

    hr = video_context_->VideoProcessorBlt(processor_.Get(), output_views_[slot].Get(), 0, 1,
                                           &stream);
    if (hr < 0) {
        return from_hresult(hr, "D3dFrameConverter: VideoProcessorBlt failed");
    }

    out_surface_index = slot;
    return ok();
}

}  // namespace tl::encode

#endif
