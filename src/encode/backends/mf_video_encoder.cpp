#include "mf_video_encoder.hpp"

#if TL_PLATFORM_WINDOWS

#include <codecapi.h>
#include <d3d11_4.h>
#include <dxgi.h>
#include <mferror.h>
#include <mftransform.h>

#include <cstddef>
#include <cstring>
#include <new>

#include "d3d_frame_converter.hpp"
#include "mf_platform.hpp"
#include "telinha/core/arena.hpp"
#include "telinha/core/log.hpp"

namespace tl::encode {
namespace {

using Microsoft::WRL::ComPtr;

constexpr std::uint32_t kSurfaceCount = 4;
constexpr std::uint32_t kPendingCapacity = 8;
constexpr std::uint32_t kInputCreditTimeoutMs = 250;
constexpr std::uint32_t kDrainTimeoutMs = 500;
constexpr std::uint32_t kParameterSetCapacity = 512;
constexpr std::uint32_t kMaxCandidates = 16;

const GUID kUnsupportedSubtype = {0, 0, 0, {0, 0, 0, 0, 0, 0, 0, 0}};

[[nodiscard]] GUID output_subtype(VideoCodec codec) noexcept
{
    switch (codec) {
        case VideoCodec::Hevc: return MFVideoFormat_HEVC;
        case VideoCodec::H264: return MFVideoFormat_H264;
        case VideoCodec::Av1:
        case VideoCodec::Unknown: break;
    }
    return kUnsupportedSubtype;
}

[[nodiscard]] eAVEncCommonRateControlMode rate_control_mode(RateControlMode mode) noexcept
{
    switch (mode) {
        case RateControlMode::VariableBitrate:
            return eAVEncCommonRateControlMode_PeakConstrainedVBR;
        case RateControlMode::ConstantQuality: return eAVEncCommonRateControlMode_Quality;
        case RateControlMode::ConstantBitrate: break;
    }
    return eAVEncCommonRateControlMode_CBR;
}

[[nodiscard]] std::uint32_t rounded_up_even(std::uint32_t value) noexcept
{
    return (value + 1u) & ~1u;
}

[[nodiscard]] ULONG quality_from_quantizer(std::uint32_t quantizer) noexcept
{
    const std::uint32_t clamped = quantizer > 51 ? 51 : quantizer;
    return static_cast<ULONG>(100 - (clamped * 100 + 25) / 51);
}

HRESULT set_codec_ui32(ICodecAPI* api, const GUID& id, ULONG value) noexcept
{
    if (api == nullptr) {
        return E_POINTER;
    }
    VARIANT variant;
    VariantInit(&variant);
    variant.vt = VT_UI4;
    variant.ulVal = value;
    const HRESULT hr = api->SetValue(&id, &variant);
    (void)VariantClear(&variant);
    return hr;
}

HRESULT set_codec_bool(ICodecAPI* api, const GUID& id, bool value) noexcept
{
    if (api == nullptr) {
        return E_POINTER;
    }
    VARIANT variant;
    VariantInit(&variant);
    variant.vt = VT_BOOL;
    variant.boolVal = value ? VARIANT_TRUE : VARIANT_FALSE;
    const HRESULT hr = api->SetValue(&id, &variant);
    (void)VariantClear(&variant);
    return hr;
}

[[nodiscard]] std::uint32_t adapter_vendor_id(ID3D11Device* device) noexcept
{
    if (device == nullptr) {
        return 0;
    }
    ComPtr<IDXGIDevice> dxgi_device;
    if (FAILED(device->QueryInterface(IID_PPV_ARGS(dxgi_device.GetAddressOf())))) {
        return 0;
    }
    ComPtr<IDXGIAdapter> adapter;
    if (FAILED(dxgi_device->GetAdapter(adapter.GetAddressOf()))) {
        return 0;
    }
    DXGI_ADAPTER_DESC desc{};
    if (FAILED(adapter->GetDesc(&desc))) {
        return 0;
    }
    return desc.VendorId;
}

[[nodiscard]] bool hex_nibble(wchar_t character, std::uint32_t& out) noexcept
{
    if (character >= L'0' && character <= L'9') {
        out = static_cast<std::uint32_t>(character - L'0');
        return true;
    }
    if (character >= L'a' && character <= L'f') {
        out = static_cast<std::uint32_t>(character - L'a') + 10u;
        return true;
    }
    if (character >= L'A' && character <= L'F') {
        out = static_cast<std::uint32_t>(character - L'A') + 10u;
        return true;
    }
    return false;
}

[[nodiscard]] bool activate_vendor_id(IMFActivate* activate, std::uint32_t& out) noexcept
{
    wchar_t text[32] = {};
    UINT32 written = 0;
    if (FAILED(activate->GetString(MFT_ENUM_HARDWARE_VENDOR_ID_Attribute, text, 32, &written))) {
        return false;
    }
    if (written < 8) {
        return false;
    }
    if (text[0] != L'V' || text[1] != L'E' || text[2] != L'N' || text[3] != L'_') {
        return false;
    }

    std::uint32_t value = 0;
    for (std::uint32_t index = 4; index < 8; ++index) {
        std::uint32_t nibble = 0;
        if (!hex_nibble(text[index], nibble)) {
            return false;
        }
        value = (value << 4) | nibble;
    }
    out = value;
    return true;
}

[[nodiscard]] int activate_score(IMFActivate* activate, std::uint32_t vendor_id) noexcept
{
    if (vendor_id == 0) {
        return 1;
    }
    std::uint32_t reported = 0;
    if (!activate_vendor_id(activate, reported)) {
        return 1;
    }
    return reported == vendor_id ? 2 : 0;
}

[[nodiscard]] bool starts_with_parameter_set(const std::byte* data, std::uint32_t length,
                                             VideoCodec codec) noexcept
{
    for (std::uint32_t offset = 0; offset + 3 < length && offset < 8; ++offset) {
        if (std::to_integer<unsigned int>(data[offset]) != 0u ||
            std::to_integer<unsigned int>(data[offset + 1]) != 0u ||
            std::to_integer<unsigned int>(data[offset + 2]) != 1u) {
            continue;
        }
        const auto header = std::to_integer<unsigned int>(data[offset + 3]);
        if (codec == VideoCodec::Hevc) {
            const unsigned int type = (header >> 1) & 0x3Fu;
            return type >= 32u && type <= 34u;
        }
        return (header & 0x1Fu) == 7u;
    }
    return false;
}

class MediaFoundationEncoder final : public VideoEncoder {
public:
    MediaFoundationEncoder(const VideoEncoderConfig& config, ID3D11Device* device,
                           bool require_hardware) noexcept
        : config_(config), device_(device), require_hardware_(require_hardware)
    {}

    ~MediaFoundationEncoder() override { stop(); }

    [[nodiscard]] Outcome initialize() noexcept;

    Outcome start() override;
    void stop() noexcept override;

    [[nodiscard]] Outcome submit(const capture::ClassifiedFrame& frame) override;
    [[nodiscard]] Outcome poll(transport::EncodedVideoFrame& out,
                               std::uint32_t timeout_ms) override;
    void release() noexcept override;

    void request_keyframe() noexcept override { keyframe_pending_ = true; }

    [[nodiscard]] Outcome set_target_bitrate(std::uint32_t bits_per_second) noexcept override;

    [[nodiscard]] VideoEncoderInfo info() const noexcept override;

private:
    [[nodiscard]] Outcome activate_transform() noexcept;
    [[nodiscard]] Outcome bind_device() noexcept;
    [[nodiscard]] Outcome negotiate_types() noexcept;
    [[nodiscard]] Outcome renegotiate_output() noexcept;
    [[nodiscard]] Outcome apply_codec_api() noexcept;
    [[nodiscard]] Outcome create_input_samples() noexcept;
    [[nodiscard]] Outcome create_output_sample() noexcept;
    [[nodiscard]] Outcome create_staging_texture() noexcept;
    [[nodiscard]] Outcome begin_streaming() noexcept;
    [[nodiscard]] Outcome stage_to_cpu(std::uint32_t surface_index,
                                       std::uint32_t sample_index) noexcept;
    [[nodiscard]] Outcome process_input(IMFSample* sample) noexcept;
    void cache_parameter_sets() noexcept;
    void teardown() noexcept;

    VideoEncoderConfig config_;
    ID3D11Device* device_ = nullptr;
    bool require_hardware_ = true;

    D3dFrameConverter converter_;
    ArenaStorage arena_storage_;

    ComPtr<ID3D11DeviceContext> context_;
    ComPtr<IMFActivate> activate_;
    ComPtr<IMFTransform> transform_;
    ComPtr<ICodecAPI> codec_api_;
    ComPtr<IMFDXGIDeviceManager> device_manager_;
    ComPtr<AsyncMftPump> pump_;

    ComPtr<IMFSample> input_samples_[kSurfaceCount];
    ComPtr<ID3D11Texture2D> staging_;
    ComPtr<IMFSample> output_sample_;

    ComPtr<IMFSample> locked_sample_;
    ComPtr<IMFMediaBuffer> locked_buffer_;

    Nanoseconds pending_capture_ns_[kPendingCapacity] = {};
    std::uint64_t pending_frame_index_[kPendingCapacity] = {};
    std::uint32_t pending_head_ = 0;
    std::uint32_t pending_count_ = 0;

    std::byte* scratch_ = nullptr;
    std::uint32_t scratch_capacity_ = 0;
    std::byte parameter_sets_[kParameterSetCapacity] = {};
    std::uint32_t parameter_set_size_ = 0;

    DWORD input_stream_id_ = 0;
    DWORD output_stream_id_ = 0;
    LONGLONG frame_duration_hns_ = 0;

    std::uint32_t encode_width_ = 0;
    std::uint32_t encode_height_ = 0;
    std::uint32_t nv12_size_ = 0;
    std::uint32_t next_cpu_sample_ = 0;
    std::uint32_t device_manager_token_ = 0;

    std::uint64_t frame_index_ = 0;
    VideoEncoderBackend reported_backend_ = VideoEncoderBackend::MediaFoundation;

    bool async_ = false;
    bool gpu_input_ = false;
    bool provides_samples_ = false;
    bool streaming_ = false;
    bool started_ = false;
    bool keyframe_pending_ = true;
    bool locked_ = false;
};

Outcome MediaFoundationEncoder::initialize() noexcept
{
    if (device_ == nullptr) {
        return fail(Status::InvalidArgument, "media foundation: a D3D11 device is required");
    }
    if (!config_.valid()) {
        return fail(Status::InvalidArgument, "media foundation: the configuration is invalid");
    }
    if (output_subtype(config_.codec) == kUnsupportedSubtype) {
        return fail(Status::NotSupported, "media foundation: this codec has no encoder here");
    }
    if (!MediaFoundationRuntime::instance().ready()) {
        return fail(Status::Unavailable, "media foundation: the platform failed to start");
    }

    reported_backend_ =
        require_hardware_ ? VideoEncoderBackend::MediaFoundation : VideoEncoderBackend::Software;

    encode_width_ = rounded_up_even(config_.width);
    encode_height_ = rounded_up_even(config_.height);
    nv12_size_ = encode_width_ * encode_height_ * 3u / 2u;
    frame_duration_hns_ =
        static_cast<LONGLONG>(10'000'000ull * 1000ull / config_.framerate_millihertz);

    device_->GetImmediateContext(context_.GetAddressOf());
    if (context_ == nullptr) {
        return fail(Status::Unavailable, "media foundation: the device has no immediate context");
    }

    ComPtr<ID3D11Multithread> multithread;
    if (SUCCEEDED(context_.As(&multithread))) {
        (void)multithread->SetMultithreadProtected(TRUE);
    }

    scratch_capacity_ = nv12_size_;
    if (!arena_storage_.reserve(scratch_capacity_ + kParameterSetCapacity)) {
        return fail(Status::OutOfMemory, "media foundation: scratch reservation failed");
    }
    scratch_ = arena_storage_.arena().allocate_array<std::byte>(scratch_capacity_);
    if (scratch_ == nullptr) {
        return fail(Status::OutOfMemory, "media foundation: scratch allocation failed");
    }
    return ok();
}

Outcome MediaFoundationEncoder::activate_transform() noexcept
{
    MFT_REGISTER_TYPE_INFO input_info{MFMediaType_Video, MFVideoFormat_NV12};
    MFT_REGISTER_TYPE_INFO output_info{MFMediaType_Video, output_subtype(config_.codec)};

    UINT32 flags = MFT_ENUM_FLAG_SORTANDFILTER;
    flags |= require_hardware_ ? (MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_ASYNCMFT)
                               : (MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_ASYNCMFT);

    IMFActivate** found = nullptr;
    UINT32 count = 0;
    const HRESULT hr =
        MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags, &input_info, &output_info, &found, &count);
    if (FAILED(hr)) {
        return from_hresult(hr, "media foundation: MFTEnumEx failed");
    }
    if (count == 0 || found == nullptr) {
        CoTaskMemFree(found);
        return fail(Status::Unavailable, "media foundation: no encoder matched this format");
    }

    const std::uint32_t vendor_id = require_hardware_ ? adapter_vendor_id(device_) : 0;
    const std::uint32_t considered = count < kMaxCandidates ? count : kMaxCandidates;

    Outcome last = fail(Status::Unavailable, "media foundation: no encoder accepted the device");
    for (int wanted = 2; wanted >= 0 && transform_ == nullptr; --wanted) {
        for (std::uint32_t index = 0; index < considered; ++index) {
            if (activate_score(found[index], vendor_id) != wanted) {
                continue;
            }
            ComPtr<IMFTransform> candidate;
            const HRESULT activated =
                found[index]->ActivateObject(IID_PPV_ARGS(candidate.GetAddressOf()));
            if (FAILED(activated)) {
                last = from_hresult(activated, "media foundation: ActivateObject failed");
                continue;
            }
            activate_ = found[index];
            transform_ = candidate;
            break;
        }
    }

    for (UINT32 index = 0; index < count; ++index) {
        if (found[index] != nullptr) {
            found[index]->Release();
        }
    }
    CoTaskMemFree(found);

    if (transform_ == nullptr) {
        return last;
    }

    DWORD input_streams = 0;
    DWORD output_streams = 0;
    if (SUCCEEDED(transform_->GetStreamCount(&input_streams, &output_streams)) &&
        input_streams > 0 && output_streams > 0) {
        const HRESULT ids = transform_->GetStreamIDs(1, &input_stream_id_, 1, &output_stream_id_);
        if (ids == E_NOTIMPL) {
            input_stream_id_ = 0;
            output_stream_id_ = 0;
        }
    }
    return ok();
}

Outcome MediaFoundationEncoder::bind_device() noexcept
{
    ComPtr<IMFAttributes> attributes;
    if (FAILED(transform_->GetAttributes(attributes.GetAddressOf()))) {
        return ok();
    }

    UINT32 is_async = 0;
    if (SUCCEEDED(attributes->GetUINT32(MF_TRANSFORM_ASYNC, &is_async)) && is_async != 0) {
        const HRESULT unlocked = attributes->SetUINT32(MF_TRANSFORM_ASYNC_UNLOCK, TRUE);
        if (FAILED(unlocked)) {
            return from_hresult(unlocked, "media foundation: the async transform stayed locked");
        }
        async_ = true;
    }

    if (config_.low_latency) {
        (void)attributes->SetUINT32(MF_LOW_LATENCY, TRUE);
    }

    UINT32 d3d_aware = 0;
    if (FAILED(attributes->GetUINT32(MF_SA_D3D11_AWARE, &d3d_aware)) || d3d_aware == 0) {
        return ok();
    }

    const HRESULT created =
        MFCreateDXGIDeviceManager(&device_manager_token_, device_manager_.GetAddressOf());
    if (FAILED(created)) {
        return ok();
    }
    if (FAILED(device_manager_->ResetDevice(device_, device_manager_token_))) {
        device_manager_.Reset();
        return ok();
    }

    const HRESULT bound = transform_->ProcessMessage(
        MFT_MESSAGE_SET_D3D_MANAGER, reinterpret_cast<ULONG_PTR>(device_manager_.Get()));
    if (FAILED(bound)) {
        device_manager_.Reset();
        return ok();
    }

    gpu_input_ = true;
    return ok();
}

Outcome MediaFoundationEncoder::negotiate_types() noexcept
{
    ComPtr<IMFMediaType> output_type;
    TL_TRY(from_hresult(MFCreateMediaType(output_type.GetAddressOf()),
                        "media foundation: MFCreateMediaType failed"));

    TL_TRY(from_hresult(output_type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video),
                        "media foundation: the output major type was rejected"));
    TL_TRY(from_hresult(output_type->SetGUID(MF_MT_SUBTYPE, output_subtype(config_.codec)),
                        "media foundation: the output subtype was rejected"));
    TL_TRY(from_hresult(output_type->SetUINT32(MF_MT_AVG_BITRATE, config_.target_bitrate_bps),
                        "media foundation: the output bitrate was rejected"));
    TL_TRY(from_hresult(
        MFSetAttributeSize(output_type.Get(), MF_MT_FRAME_SIZE, encode_width_, encode_height_),
        "media foundation: the output frame size was rejected"));
    TL_TRY(from_hresult(MFSetAttributeRatio(output_type.Get(), MF_MT_FRAME_RATE,
                                            config_.framerate_millihertz, 1000),
                        "media foundation: the output frame rate was rejected"));
    TL_TRY(from_hresult(output_type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive),
                        "media foundation: the output scan mode was rejected"));
    TL_TRY(from_hresult(MFSetAttributeRatio(output_type.Get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1),
                        "media foundation: the output aspect ratio was rejected"));

    if (config_.codec == VideoCodec::H264) {
        (void)output_type->SetUINT32(MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High);
    }
    if (config_.gop_length != 0) {
        (void)output_type->SetUINT32(MF_MT_MAX_KEYFRAME_SPACING, config_.gop_length);
    }

    TL_TRY(from_hresult(transform_->SetOutputType(output_stream_id_, output_type.Get(), 0),
                        "media foundation: the encoder refused the output type"));

    for (DWORD index = 0;; ++index) {
        ComPtr<IMFMediaType> candidate;
        const HRESULT available =
            transform_->GetInputAvailableType(input_stream_id_, index, candidate.GetAddressOf());
        if (available == MF_E_NO_MORE_TYPES) {
            break;
        }
        if (FAILED(available)) {
            break;
        }

        GUID subtype = kUnsupportedSubtype;
        if (FAILED(candidate->GetGUID(MF_MT_SUBTYPE, &subtype)) || subtype != MFVideoFormat_NV12) {
            continue;
        }

        (void)MFSetAttributeSize(candidate.Get(), MF_MT_FRAME_SIZE, encode_width_, encode_height_);
        (void)MFSetAttributeRatio(candidate.Get(), MF_MT_FRAME_RATE, config_.framerate_millihertz,
                                  1000);
        (void)candidate->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive);
        (void)MFSetAttributeRatio(candidate.Get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1);
        if (!gpu_input_) {
            (void)candidate->SetUINT32(MF_MT_DEFAULT_STRIDE, encode_width_);
        }

        if (SUCCEEDED(transform_->SetInputType(input_stream_id_, candidate.Get(), 0))) {
            return ok();
        }
    }

    ComPtr<IMFMediaType> input_type;
    TL_TRY(from_hresult(MFCreateMediaType(input_type.GetAddressOf()),
                        "media foundation: MFCreateMediaType failed"));
    (void)input_type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
    (void)input_type->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12);
    (void)MFSetAttributeSize(input_type.Get(), MF_MT_FRAME_SIZE, encode_width_, encode_height_);
    (void)MFSetAttributeRatio(input_type.Get(), MF_MT_FRAME_RATE, config_.framerate_millihertz,
                              1000);
    (void)input_type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive);
    (void)MFSetAttributeRatio(input_type.Get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1);
    if (!gpu_input_) {
        (void)input_type->SetUINT32(MF_MT_DEFAULT_STRIDE, encode_width_);
    }

    return from_hresult(transform_->SetInputType(input_stream_id_, input_type.Get(), 0),
                        "media foundation: the encoder refused NV12 input");
}

Outcome MediaFoundationEncoder::renegotiate_output() noexcept
{
    for (DWORD index = 0;; ++index) {
        ComPtr<IMFMediaType> candidate;
        const HRESULT available =
            transform_->GetOutputAvailableType(output_stream_id_, index, candidate.GetAddressOf());
        if (FAILED(available)) {
            break;
        }
        if (SUCCEEDED(transform_->SetOutputType(output_stream_id_, candidate.Get(), 0))) {
            cache_parameter_sets();
            return ok();
        }
    }
    return fail(Status::ConfigurationChanged,
                "media foundation: no output type survived the format change");
}

Outcome MediaFoundationEncoder::apply_codec_api() noexcept
{
    if (FAILED(transform_.As(&codec_api_))) {
        codec_api_.Reset();
        return ok();
    }

    (void)set_codec_ui32(codec_api_.Get(), CODECAPI_AVEncCommonRateControlMode,
                         static_cast<ULONG>(rate_control_mode(config_.rate_control)));
    (void)set_codec_ui32(codec_api_.Get(), CODECAPI_AVEncCommonMeanBitRate,
                         config_.target_bitrate_bps);

    if (config_.rate_control == RateControlMode::VariableBitrate) {
        (void)set_codec_ui32(codec_api_.Get(), CODECAPI_AVEncCommonMaxBitRate,
                             config_.max_bitrate_bps);
    }
    if (config_.rate_control == RateControlMode::ConstantQuality) {
        (void)set_codec_ui32(codec_api_.Get(), CODECAPI_AVEncCommonQuality,
                             quality_from_quantizer(config_.constant_quality));
    }

    (void)set_codec_ui32(codec_api_.Get(), CODECAPI_AVEncMPVDefaultBPictureCount, 0);
    (void)set_codec_bool(codec_api_.Get(), CODECAPI_AVEncCommonLowLatency, config_.low_latency);
    (void)set_codec_bool(codec_api_.Get(), CODECAPI_AVEncCommonRealTime, config_.low_latency);
    (void)set_codec_ui32(codec_api_.Get(), CODECAPI_AVEncVideoMaxNumRefFrame,
                         config_.max_reference_frames);
    if (config_.gop_length != 0) {
        (void)set_codec_ui32(codec_api_.Get(), CODECAPI_AVEncMPVGOPSize, config_.gop_length);
    }
    return ok();
}

Outcome MediaFoundationEncoder::create_staging_texture() noexcept
{
    D3D11_TEXTURE2D_DESC desc{};
    desc.Width = encode_width_;
    desc.Height = encode_height_;
    desc.MipLevels = 1;
    desc.ArraySize = 1;
    desc.Format = DXGI_FORMAT_NV12;
    desc.SampleDesc.Count = 1;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;

    return from_hresult(device_->CreateTexture2D(&desc, nullptr, staging_.ReleaseAndGetAddressOf()),
                        "media foundation: the staging texture was refused");
}

Outcome MediaFoundationEncoder::create_input_samples() noexcept
{
    for (std::uint32_t index = 0; index < kSurfaceCount; ++index) {
        ComPtr<IMFSample> sample;
        TL_TRY(from_hresult(MFCreateSample(sample.GetAddressOf()),
                            "media foundation: MFCreateSample failed"));

        ComPtr<IMFMediaBuffer> buffer;
        if (gpu_input_) {
            ID3D11Texture2D* surface = converter_.surface(index);
            if (surface == nullptr) {
                return fail(Status::Unavailable, "media foundation: the converter has no surface");
            }
            TL_TRY(from_hresult(MFCreateDXGISurfaceBuffer(__uuidof(ID3D11Texture2D), surface, 0,
                                                          FALSE, buffer.GetAddressOf()),
                                "media foundation: wrapping the NV12 surface failed"));

            ComPtr<IMF2DBuffer> two_dimensional;
            DWORD contiguous = 0;
            if (SUCCEEDED(buffer.As(&two_dimensional)) &&
                SUCCEEDED(two_dimensional->GetContiguousLength(&contiguous))) {
                (void)buffer->SetCurrentLength(contiguous);
            }
        } else {
            TL_TRY(from_hresult(MFCreateMemoryBuffer(nv12_size_, buffer.GetAddressOf()),
                                "media foundation: MFCreateMemoryBuffer failed"));
            (void)buffer->SetCurrentLength(nv12_size_);
        }

        TL_TRY(from_hresult(sample->AddBuffer(buffer.Get()), "media foundation: AddBuffer failed"));
        input_samples_[index] = sample;
    }
    return ok();
}

Outcome MediaFoundationEncoder::create_output_sample() noexcept
{
    MFT_OUTPUT_STREAM_INFO stream_info{};
    TL_TRY(from_hresult(transform_->GetOutputStreamInfo(output_stream_id_, &stream_info),
                        "media foundation: GetOutputStreamInfo failed"));

    provides_samples_ = (stream_info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES |
                                                MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES)) != 0;
    if (provides_samples_) {
        return ok();
    }

    DWORD size = stream_info.cbSize;
    if (size < nv12_size_) {
        size = nv12_size_;
    }

    ComPtr<IMFSample> sample;
    TL_TRY(from_hresult(MFCreateSample(sample.GetAddressOf()),
                        "media foundation: MFCreateSample failed"));

    ComPtr<IMFMediaBuffer> buffer;
    TL_TRY(from_hresult(MFCreateMemoryBuffer(size, buffer.GetAddressOf()),
                        "media foundation: the output buffer was refused"));
    TL_TRY(from_hresult(sample->AddBuffer(buffer.Get()), "media foundation: AddBuffer failed"));

    output_sample_ = sample;
    return ok();
}

void MediaFoundationEncoder::cache_parameter_sets() noexcept
{
    parameter_set_size_ = 0;

    ComPtr<IMFMediaType> current;
    if (FAILED(transform_->GetOutputCurrentType(output_stream_id_, current.GetAddressOf()))) {
        return;
    }

    UINT32 size = 0;
    if (FAILED(current->GetBlobSize(MF_MT_MPEG_SEQUENCE_HEADER, &size)) || size == 0 ||
        size > kParameterSetCapacity) {
        return;
    }

    UINT32 written = 0;
    if (FAILED(current->GetBlob(MF_MT_MPEG_SEQUENCE_HEADER,
                                reinterpret_cast<UINT8*>(parameter_sets_), size, &written))) {
        return;
    }
    parameter_set_size_ = written;
}

Outcome MediaFoundationEncoder::begin_streaming() noexcept
{
    if (async_) {
        pump_.Attach(AsyncMftPump::create());
        if (pump_ == nullptr) {
            return fail(Status::OutOfMemory, "media foundation: the event pump failed to allocate");
        }
        TL_TRY(pump_->start(transform_.Get()));
    }

    TL_TRY(from_hresult(transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0),
                        "media foundation: BEGIN_STREAMING was refused"));
    TL_TRY(from_hresult(transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0),
                        "media foundation: START_OF_STREAM was refused"));
    streaming_ = true;
    return ok();
}

Outcome MediaFoundationEncoder::start()
{
    if (started_) {
        return ok();
    }

    TL_TRY(converter_.initialize(device_, encode_width_, encode_height_, kSurfaceCount));
    TL_TRY(activate_transform());
    TL_TRY(bind_device());
    TL_TRY(negotiate_types());
    TL_TRY(apply_codec_api());

    if (!gpu_input_) {
        TL_TRY(create_staging_texture());
    }
    TL_TRY(create_input_samples());
    TL_TRY(create_output_sample());

    cache_parameter_sets();
    TL_TRY(begin_streaming());

    pending_head_ = 0;
    pending_count_ = 0;
    next_cpu_sample_ = 0;
    frame_index_ = 0;
    keyframe_pending_ = true;
    started_ = true;

    TL_LOG_INFO("media foundation encoder ready: %ux%u, %s input, %s transform", encode_width_,
                encode_height_, gpu_input_ ? "gpu" : "cpu", async_ ? "async" : "sync");
    return ok();
}

void MediaFoundationEncoder::teardown() noexcept
{
    if (transform_ != nullptr && streaming_) {
        (void)transform_->ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0);
        if (async_ && pump_ != nullptr) {
            (void)pump_->wait_drained(kDrainTimeoutMs);
        }
        (void)transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
        (void)transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        (void)transform_->ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
    }
    streaming_ = false;

    if (pump_ != nullptr) {
        pump_->detach();
    }

    if (transform_ != nullptr) {
        ComPtr<IMFShutdown> shutdown;
        if (SUCCEEDED(transform_.As(&shutdown))) {
            (void)shutdown->Shutdown();
        }
    }

    for (std::uint32_t index = 0; index < kSurfaceCount; ++index) {
        input_samples_[index].Reset();
    }
    output_sample_.Reset();
    staging_.Reset();
    codec_api_.Reset();
    transform_.Reset();
    device_manager_.Reset();
    pump_.Reset();

    if (activate_ != nullptr) {
        (void)activate_->ShutdownObject();
        activate_.Reset();
    }
}

void MediaFoundationEncoder::stop() noexcept
{
    release();
    teardown();
    converter_.shutdown();

    started_ = false;
    async_ = false;
    gpu_input_ = false;
    provides_samples_ = false;
    pending_head_ = 0;
    pending_count_ = 0;
    parameter_set_size_ = 0;
}

Outcome MediaFoundationEncoder::stage_to_cpu(std::uint32_t surface_index,
                                             std::uint32_t sample_index) noexcept
{
    ID3D11Texture2D* source = converter_.surface(surface_index);
    if (source == nullptr || staging_ == nullptr) {
        return fail(Status::Unavailable, "media foundation: the staging path is not ready");
    }

    context_->CopyResource(staging_.Get(), source);

    D3D11_MAPPED_SUBRESOURCE mapped{};
    const HRESULT locked = context_->Map(staging_.Get(), 0, D3D11_MAP_READ, 0, &mapped);
    if (FAILED(locked)) {
        return from_hresult(locked, "media foundation: mapping the staging texture failed");
    }

    ComPtr<IMFMediaBuffer> buffer;
    HRESULT hr = input_samples_[sample_index]->GetBufferByIndex(0, buffer.GetAddressOf());

    BYTE* destination = nullptr;
    DWORD capacity = 0;
    if (SUCCEEDED(hr)) {
        hr = buffer->Lock(&destination, &capacity, nullptr);
    }
    if (FAILED(hr) || capacity < nv12_size_) {
        if (SUCCEEDED(hr)) {
            (void)buffer->Unlock();
            hr = E_UNEXPECTED;
        }
        context_->Unmap(staging_.Get(), 0);
        return from_hresult(hr, "media foundation: the input buffer could not be filled");
    }

    const auto* source_bytes = static_cast<const BYTE*>(mapped.pData);
    MFCopyImage(destination, static_cast<LONG>(encode_width_), source_bytes,
                static_cast<LONG>(mapped.RowPitch), encode_width_, encode_height_);
    MFCopyImage(destination + static_cast<std::size_t>(encode_width_) * encode_height_,
                static_cast<LONG>(encode_width_),
                source_bytes + static_cast<std::size_t>(mapped.RowPitch) * encode_height_,
                static_cast<LONG>(mapped.RowPitch), encode_width_, encode_height_ / 2u);

    (void)buffer->SetCurrentLength(nv12_size_);
    (void)buffer->Unlock();
    context_->Unmap(staging_.Get(), 0);
    return ok();
}

Outcome MediaFoundationEncoder::process_input(IMFSample* sample) noexcept
{
    if (async_) {
        TL_TRY(pump_->wait_input_credit(kInputCreditTimeoutMs));
    }

    const HRESULT hr = transform_->ProcessInput(input_stream_id_, sample, 0);
    if (hr == MF_E_NOTACCEPTING) {
        return fail(Status::Full, "media foundation: the encoder is not accepting input");
    }
    return from_hresult(hr, "media foundation: ProcessInput failed");
}

Outcome MediaFoundationEncoder::submit(const capture::ClassifiedFrame& frame)
{
    if (!started_) {
        return fail(Status::Unavailable, "media foundation: not started");
    }
    if (frame.frame == nullptr) {
        return fail(Status::InvalidArgument, "media foundation: the classified frame is empty");
    }

    const capture::FrameSurface& surface = frame.frame->surface;
    if (surface.memory != capture::SurfaceMemory::GpuTexture || surface.gpu_texture == nullptr) {
        return fail(Status::NotSupported, "media foundation: a GPU texture is required");
    }
    if (pending_count_ == kPendingCapacity) {
        return fail(Status::Full, "media foundation: too many frames are still in flight");
    }

    std::uint32_t surface_index = 0;
    TL_TRY(converter_.convert(static_cast<ID3D11Texture2D*>(surface.gpu_texture), surface.rotation,
                              surface_index));

    std::uint32_t sample_index = surface_index;
    if (!gpu_input_) {
        sample_index = next_cpu_sample_;
        TL_TRY(stage_to_cpu(surface_index, sample_index));
    }

    IMFSample* sample = input_samples_[sample_index].Get();
    (void)sample->SetSampleTime(static_cast<LONGLONG>(frame.frame->metadata.present_time_ns / 100));
    (void)sample->SetSampleDuration(frame_duration_hns_);

    if (keyframe_pending_) {
        (void)sample->SetUINT32(MFSampleExtension_ForceKeyFrame, TRUE);
        (void)set_codec_ui32(codec_api_.Get(), CODECAPI_AVEncVideoForceKeyFrame, 1);
    } else {
        (void)sample->DeleteItem(MFSampleExtension_ForceKeyFrame);
    }

    TL_TRY(process_input(sample));

    const std::uint32_t slot = (pending_head_ + pending_count_) % kPendingCapacity;
    pending_capture_ns_[slot] = frame.frame->metadata.present_time_ns;
    pending_frame_index_[slot] = frame_index_;
    ++pending_count_;

    if (!gpu_input_) {
        next_cpu_sample_ = (next_cpu_sample_ + 1) % kSurfaceCount;
    }
    ++frame_index_;
    keyframe_pending_ = false;
    return ok();
}

Outcome MediaFoundationEncoder::poll(transport::EncodedVideoFrame& out, std::uint32_t timeout_ms)
{
    out = transport::EncodedVideoFrame{};

    if (!started_) {
        return fail(Status::Unavailable, "media foundation: not started");
    }
    if (locked_) {
        return fail(Status::AlreadyExists,
                    "media foundation: the previous packet was not released");
    }
    if (pending_count_ == 0) {
        return fail(Status::Timeout, "media foundation: no encoded frame is pending");
    }

    if (async_) {
        TL_TRY(pump_->wait_output_credit(timeout_ms));
    }

    if (!provides_samples_) {
        ComPtr<IMFMediaBuffer> reused;
        if (SUCCEEDED(output_sample_->GetBufferByIndex(0, reused.GetAddressOf()))) {
            (void)reused->SetCurrentLength(0);
        }
    }

    MFT_OUTPUT_DATA_BUFFER data{};
    data.dwStreamID = output_stream_id_;
    data.pSample = provides_samples_ ? nullptr : output_sample_.Get();

    DWORD produced = 0;
    const HRESULT hr = transform_->ProcessOutput(0, 1, &data, &produced);

    if (data.pEvents != nullptr) {
        data.pEvents->Release();
        data.pEvents = nullptr;
    }

    ComPtr<IMFSample> sample;
    if (provides_samples_ && data.pSample != nullptr) {
        sample.Attach(data.pSample);
    } else if (!provides_samples_) {
        sample = output_sample_;
    }

    if (hr == MF_E_TRANSFORM_NEED_MORE_INPUT) {
        return fail(Status::Timeout, "media foundation: no encoded frame is ready");
    }
    if (hr == MF_E_TRANSFORM_STREAM_CHANGE) {
        TL_TRY(renegotiate_output());
        return fail(Status::ConfigurationChanged,
                    "media foundation: the encoder changed its output type");
    }
    if (FAILED(hr)) {
        return from_hresult(hr, "media foundation: ProcessOutput failed");
    }
    if (sample == nullptr) {
        return fail(Status::Unavailable, "media foundation: the encoder produced no sample");
    }

    ComPtr<IMFMediaBuffer> contiguous;
    TL_TRY(from_hresult(sample->ConvertToContiguousBuffer(contiguous.GetAddressOf()),
                        "media foundation: ConvertToContiguousBuffer failed"));

    BYTE* payload = nullptr;
    DWORD length = 0;
    TL_TRY(from_hresult(contiguous->Lock(&payload, nullptr, &length),
                        "media foundation: locking the encoded buffer failed"));

    locked_sample_ = sample;
    locked_buffer_ = contiguous;
    locked_ = true;

    UINT32 clean_point = 0;
    const bool key = SUCCEEDED(sample->GetUINT32(MFSampleExtension_CleanPoint, &clean_point)) &&
                     clean_point != 0;

    const auto* bitstream = reinterpret_cast<const std::byte*>(payload);
    std::uint32_t bitstream_size = length;

    if (key && config_.repeat_parameter_sets && parameter_set_size_ != 0 &&
        !starts_with_parameter_set(bitstream, bitstream_size, config_.codec) &&
        static_cast<std::size_t>(parameter_set_size_) + bitstream_size <= scratch_capacity_) {
        std::memcpy(scratch_, parameter_sets_, parameter_set_size_);
        std::memcpy(scratch_ + parameter_set_size_, bitstream, bitstream_size);
        bitstream = scratch_;
        bitstream_size += parameter_set_size_;
    }

    out.bitstream = Span<const std::byte>(bitstream, bitstream_size);
    out.capture_time_ns = pending_capture_ns_[pending_head_];
    out.encode_end_time_ns = now_ns();
    out.frame_index = pending_frame_index_[pending_head_];
    out.width = encode_width_;
    out.height = encode_height_;
    out.average_qp = 0;
    out.codec = to_wire_codec(config_.codec);
    out.kind = key ? transport::WireFrameKind::Key : transport::WireFrameKind::Delta;
    out.temporal_index = 0;
    out.spatial_index = 0;

    pending_head_ = (pending_head_ + 1) % kPendingCapacity;
    --pending_count_;
    return ok();
}

void MediaFoundationEncoder::release() noexcept
{
    if (!locked_) {
        return;
    }
    if (locked_buffer_ != nullptr) {
        (void)locked_buffer_->Unlock();
    }
    locked_buffer_.Reset();
    locked_sample_.Reset();
    locked_ = false;
}

Outcome MediaFoundationEncoder::set_target_bitrate(std::uint32_t bits_per_second) noexcept
{
    if (bits_per_second == 0) {
        return fail(Status::InvalidArgument, "media foundation: the bitrate must be positive");
    }
    if (codec_api_ == nullptr) {
        return fail(Status::NotSupported, "media foundation: this encoder has no runtime controls");
    }

    const HRESULT hr =
        set_codec_ui32(codec_api_.Get(), CODECAPI_AVEncCommonMeanBitRate, bits_per_second);
    if (FAILED(hr)) {
        return from_hresult(hr, "media foundation: the bitrate change was refused");
    }
    config_.target_bitrate_bps = bits_per_second;
    return ok();
}

VideoEncoderInfo MediaFoundationEncoder::info() const noexcept
{
    VideoEncoderInfo result;
    result.codec = config_.codec;
    result.backend = reported_backend_;
    result.width = encode_width_;
    result.height = encode_height_;
    result.coding_unit_size = coding_unit_size_for(config_.codec);
    result.accepts_gpu_surfaces = gpu_input_;
    result.supports_dirty_regions = false;
    result.supports_intra_refresh = false;
    return result;
}

}  // namespace

bool media_foundation_available(VideoCodec codec, bool require_hardware) noexcept
{
    const GUID subtype = output_subtype(codec);
    if (subtype == kUnsupportedSubtype) {
        return false;
    }
    if (!MediaFoundationRuntime::instance().ready()) {
        return false;
    }

    MFT_REGISTER_TYPE_INFO input_info{MFMediaType_Video, MFVideoFormat_NV12};
    MFT_REGISTER_TYPE_INFO output_info{MFMediaType_Video, subtype};

    UINT32 flags = MFT_ENUM_FLAG_SORTANDFILTER;
    flags |= require_hardware ? (MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_ASYNCMFT)
                              : (MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_ASYNCMFT);

    IMFActivate** found = nullptr;
    UINT32 count = 0;
    if (FAILED(MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags, &input_info, &output_info, &found,
                         &count))) {
        return false;
    }

    for (UINT32 index = 0; index < count; ++index) {
        if (found[index] != nullptr) {
            found[index]->Release();
        }
    }
    CoTaskMemFree(found);
    return count != 0;
}

Result<std::unique_ptr<VideoEncoder>> create_media_foundation_encoder(
    const VideoEncoderConfig& config, void* native_device, bool require_hardware)
{
    auto* encoder = new (std::nothrow)
        MediaFoundationEncoder(config, static_cast<ID3D11Device*>(native_device), require_hardware);
    if (encoder == nullptr) {
        return Error{Status::OutOfMemory, "MediaFoundationEncoder"};
    }

    std::unique_ptr<VideoEncoder> owned(encoder);
    const Outcome initialized = encoder->initialize();
    if (!initialized.ok()) {
        return Error{initialized.error()};
    }
    return owned;
}

}  // namespace tl::encode

#endif
