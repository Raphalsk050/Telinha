#include <d3d11.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mftransform.h>
#include <windows.h>
#include <wrl/client.h>

#include <cstring>
#include <new>
#include <utility>

#include "telinha/core/clock.hpp"
#include "telinha/core/log.hpp"
#include "telinha/receive/backends.hpp"

namespace tl::receive {
namespace {

using Microsoft::WRL::ComPtr;

constexpr std::uint32_t kInputPoolSize = 8;
constexpr std::uint32_t kDefaultInputBytes = 2u * 1024u * 1024u;
constexpr std::uint32_t kMaxOutputsPerDrain = 16;
constexpr std::uint32_t kInputRetries = 10;
constexpr DWORD kInputRetryWaitMs = 2;
constexpr Nanoseconds kHundredNanoseconds = 100;

struct InputStamp {
    LONGLONG sample_time = -1;
    std::uint64_t frame_index = 0;
    Nanoseconds remote_time_ns = 0;
    bool keyframe = false;
};

[[nodiscard]] ULONG reference_count(IUnknown* object) noexcept
{
    object->AddRef();
    return object->Release();
}

class MediaFoundationDecoder final : public VideoDecoder {
public:
    MediaFoundationDecoder(const VideoDecoderConfig& config, ID3D11Device* device) noexcept
        : config_(config), device_(device)
    {}

    ~MediaFoundationDecoder() override { stop(); }

    Outcome start() override
    {
        if (config_.codec != transport::WireVideoCodec::H264) {
            return fail(Status::NotSupported, "MediaFoundationDecoder::start: codec");
        }
        if (config_.width == 0 || config_.height == 0) {
            return fail(Status::InvalidArgument, "MediaFoundationDecoder::start: dimensoes");
        }

        const HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
        if (SUCCEEDED(initialized)) {
            com_initialized_ = true;
        } else if (initialized != RPC_E_CHANGED_MODE) {
            return fail(Status::PlatformError, "MediaFoundationDecoder::start: CoInitializeEx",
                        initialized);
        }

        HRESULT result = MFStartup(MF_VERSION, MFSTARTUP_LITE);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "MediaFoundationDecoder::start: MFStartup", result);
        }
        mf_started_ = true;

        if (device_ != nullptr) {
            const Outcome manager = create_device_manager();
            if (!manager.ok()) {
                TL_LOG_WARN("receptor: sem aceleracao de decode (%s), seguindo em software",
                            to_string(manager.status()));
            }
        }

        TL_TRY(create_transform());
        TL_TRY(configure_types());
        TL_TRY(create_input_pool());

        if (manager_) {
            TL_TRY(create_share_texture());
        }

        transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0);
        transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);

        started_ = true;
        return ok();
    }

    void stop() noexcept override
    {
        release();
        ready_.Reset();
        started_ = false;

        if (transform_) {
            transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        }

        for (std::uint32_t index = 0; index < kInputPoolSize; ++index) {
            input_samples_[index].Reset();
            input_buffers_[index].Reset();
        }

        share_texture_.Reset();
        context_.Reset();
        transform_.Reset();
        manager_.Reset();

        if (mf_started_) {
            MFShutdown();
            mf_started_ = false;
        }
        if (com_initialized_) {
            CoUninitialize();
            com_initialized_ = false;
        }
    }

    Outcome submit(const transport::EncodedVideoFrame& frame) override
    {
        if (!started_) {
            return fail(Status::Unavailable, "MediaFoundationDecoder::submit");
        }
        if (!frame.valid()) {
            return fail(Status::InvalidArgument, "MediaFoundationDecoder::submit");
        }

        const std::size_t bytes = frame.bitstream.size_bytes();
        if (bytes > input_capacity_) {
            return fail(Status::OutOfRange, "MediaFoundationDecoder::submit: pacote grande demais");
        }

        TL_TRY(drain_outputs());

        const std::uint32_t slot = input_slot();
        IMFMediaBuffer* buffer = input_buffers_[slot].Get();
        BYTE* destination = nullptr;
        DWORD capacity = 0;
        HRESULT result = buffer->Lock(&destination, &capacity, nullptr);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "MediaFoundationDecoder::submit: Lock", result);
        }

        std::memcpy(destination, frame.bitstream.data(), bytes);
        buffer->Unlock();
        buffer->SetCurrentLength(static_cast<DWORD>(bytes));

        const bool keyframe = frame.kind == transport::WireFrameKind::Key;
        const auto sample_time = static_cast<LONGLONG>(frame.capture_time_ns / kHundredNanoseconds);
        IMFSample* sample = input_samples_[slot].Get();
        sample->SetSampleTime(sample_time);
        sample->SetSampleDuration(static_cast<LONGLONG>(frame_duration_ns_ / kHundredNanoseconds));
        sample->SetUINT32(MFSampleExtension_CleanPoint, keyframe ? 1u : 0u);

        InputStamp& stamp = stamps_[next_stamp_ % kInputPoolSize];
        ++next_stamp_;
        stamp.sample_time = sample_time;
        stamp.frame_index = frame.frame_index;
        stamp.remote_time_ns = frame.capture_time_ns;
        stamp.keyframe = keyframe;

        result = transform_->ProcessInput(0, sample, 0);
        for (std::uint32_t attempt = 0; result == MF_E_NOTACCEPTING && attempt < kInputRetries;
             ++attempt) {
            ::Sleep(kInputRetryWaitMs);
            TL_TRY(drain_outputs());
            result = transform_->ProcessInput(0, sample, 0);
        }
        if (result == MF_E_NOTACCEPTING) {
            return fail(Status::Full, "MediaFoundationDecoder::submit: nao aceita");
        }
        if (FAILED(result)) {
            return fail(Status::PlatformError, "MediaFoundationDecoder::submit: ProcessInput",
                        result);
        }
        return ok();
    }

    Outcome poll(DecodedVideoFrame& out, std::uint32_t timeout_ms) override
    {
        (void)timeout_ms;
        out = DecodedVideoFrame{};

        if (!started_) {
            return fail(Status::Unavailable, "MediaFoundationDecoder::poll");
        }
        if (held_sample_) {
            return fail(Status::Full, "MediaFoundationDecoder::poll: quadro nao liberado");
        }

        TL_TRY(drain_outputs());
        if (!ready_) {
            return fail(Status::WouldBlock, "MediaFoundationDecoder::poll");
        }

        const ComPtr<IMFSample> sample = std::move(ready_);

        ComPtr<IMFMediaBuffer> buffer;
        if (FAILED(sample->GetBufferByIndex(0, &buffer))) {
            return fail(Status::PlatformError, "MediaFoundationDecoder::poll: GetBufferByIndex");
        }

        describe(sample.Get(), out);

        ComPtr<IMFDXGIBuffer> dxgi_buffer;
        if (share_texture_ && SUCCEEDED(buffer.As(&dxgi_buffer))) {
            ComPtr<ID3D11Texture2D> texture;
            UINT subresource = 0;
            if (SUCCEEDED(dxgi_buffer->GetResource(IID_PPV_ARGS(&texture))) &&
                SUCCEEDED(dxgi_buffer->GetSubresourceIndex(&subresource))) {
                const D3D11_BOX visible = {0, 0, 0, config_.width, config_.height, 1};
                context_->CopySubresourceRegion(share_texture_.Get(), 0, 0, 0, 0, texture.Get(),
                                                subresource, &visible);
                out.memory = FrameMemory::GpuTexture;
                out.gpu_texture = share_texture_.Get();
                out.gpu_subresource = 0;
                return ok();
            }
        }

        ComPtr<IMF2DBuffer2> planar;
        if (SUCCEEDED(buffer.As(&planar))) {
            BYTE* scanline = nullptr;
            LONG pitch = 0;
            BYTE* begin = nullptr;
            DWORD span = 0;
            if (SUCCEEDED(planar->Lock2DSize(MF2DBuffer_LockFlags_Read, &scanline, &pitch, &begin,
                                             &span))) {
                held_planar_ = planar;
                held_sample_ = sample;
                out.memory = FrameMemory::CpuPlanar;
                out.plane_count = 2;
                out.plane_data[0] = reinterpret_cast<const std::byte*>(scanline);
                out.plane_pitch[0] = static_cast<std::uint32_t>(pitch < 0 ? -pitch : pitch);
                out.plane_data[1] =
                    out.plane_data[0] + static_cast<std::size_t>(out.plane_pitch[0]) * out.height;
                out.plane_pitch[1] = out.plane_pitch[0];
                return ok();
            }
        }

        BYTE* linear = nullptr;
        DWORD linear_length = 0;
        if (SUCCEEDED(buffer->Lock(&linear, nullptr, &linear_length))) {
            held_buffer_ = buffer;
            held_sample_ = sample;
            out.memory = FrameMemory::CpuPlanar;
            out.plane_count = 2;
            out.plane_data[0] = reinterpret_cast<const std::byte*>(linear);
            out.plane_pitch[0] = out.width;
            out.plane_data[1] =
                out.plane_data[0] + static_cast<std::size_t>(out.width) * out.height;
            out.plane_pitch[1] = out.width;
            return ok();
        }

        return fail(Status::PlatformError, "MediaFoundationDecoder::poll: sem acesso ao quadro");
    }

    void release() noexcept override
    {
        if (held_planar_) {
            held_planar_->Unlock2D();
            held_planar_.Reset();
        }
        if (held_buffer_) {
            held_buffer_->Unlock();
            held_buffer_.Reset();
        }
        held_sample_.Reset();
    }

    void flush() noexcept override
    {
        release();
        ready_.Reset();
        if (transform_) {
            transform_->ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
        }
    }

    VideoDecoderInfo info() const noexcept override
    {
        VideoDecoderInfo result;
        result.codec = config_.codec;
        result.backend =
            manager_ ? VideoDecoderBackend::D3D11VideoDevice : VideoDecoderBackend::MediaFoundation;
        result.width = config_.width;
        result.height = config_.height;
        result.output_format = capture::PixelFormat::NV12;
        result.native_device = device_;
        result.outputs_gpu_surfaces = share_texture_ != nullptr;
        return result;
    }

private:
    [[nodiscard]] std::uint32_t input_slot() noexcept
    {
        std::uint32_t chosen = next_input_ % kInputPoolSize;
        for (std::uint32_t probe = 0; probe < kInputPoolSize; ++probe) {
            const std::uint32_t candidate = (next_input_ + probe) % kInputPoolSize;
            if (reference_count(input_samples_[candidate].Get()) <= 1 &&
                reference_count(input_buffers_[candidate].Get()) <= 2) {
                chosen = candidate;
                break;
            }
        }
        next_input_ = chosen + 1;
        return chosen;
    }

    Outcome pull_output() noexcept
    {
        MFT_OUTPUT_DATA_BUFFER output = {};
        DWORD status = 0;

        ComPtr<IMFSample> allocated;
        if (!provides_samples_) {
            TL_TRY(allocate_output_sample(allocated));
            output.pSample = allocated.Get();
        }

        const HRESULT result = transform_->ProcessOutput(0, 1, &output, &status);

        if (output.pEvents != nullptr) {
            output.pEvents->Release();
            output.pEvents = nullptr;
        }

        ComPtr<IMFSample> sample;
        if (provides_samples_) {
            sample.Attach(output.pSample);
        } else {
            sample = allocated;
        }

        if (result == MF_E_TRANSFORM_NEED_MORE_INPUT) {
            return fail(Status::WouldBlock, "MediaFoundationDecoder: sem quadro pronto");
        }
        if (result == MF_E_TRANSFORM_STREAM_CHANGE) {
            TL_TRY(configure_output_type());
            if (manager_) {
                share_texture_.Reset();
                TL_TRY(create_share_texture());
            }
            return fail(Status::ConfigurationChanged, "MediaFoundationDecoder: formato mudou");
        }
        if (FAILED(result)) {
            return fail(Status::PlatformError, "MediaFoundationDecoder: ProcessOutput", result);
        }
        if (!sample) {
            return fail(Status::WouldBlock, "MediaFoundationDecoder: sem amostra");
        }

        ready_ = std::move(sample);
        return ok();
    }

    Outcome drain_outputs() noexcept
    {
        for (std::uint32_t pulled = 0; pulled < kMaxOutputsPerDrain; ++pulled) {
            const Outcome produced = pull_output();
            if (produced.status() == Status::WouldBlock) {
                return ok();
            }
            if (!produced.ok() && produced.status() != Status::ConfigurationChanged) {
                return produced;
            }
        }
        return ok();
    }

    void describe(IMFSample* sample, DecodedVideoFrame& out) const noexcept
    {
        out.width = config_.width;
        out.height = config_.height;
        out.format = capture::PixelFormat::NV12;
        out.decode_end_ns = now_ns();

        const InputStamp* match = &stamps_[(next_stamp_ + kInputPoolSize - 1) % kInputPoolSize];
        LONGLONG sample_time = 0;
        if (SUCCEEDED(sample->GetSampleTime(&sample_time))) {
            for (const InputStamp& stamp : stamps_) {
                if (stamp.sample_time == sample_time) {
                    match = &stamp;
                    break;
                }
            }
        }
        out.frame_index = match->frame_index;
        out.remote_time_ns = match->remote_time_ns;
        out.keyframe = match->keyframe;
    }

    Outcome create_device_manager()
    {
        UINT token = 0;
        HRESULT result = MFCreateDXGIDeviceManager(&token, &manager_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_device_manager: MFCreateDXGIDeviceManager",
                        result);
        }

        result = manager_->ResetDevice(device_, token);
        if (FAILED(result)) {
            manager_.Reset();
            return fail(Status::PlatformError, "create_device_manager: ResetDevice", result);
        }

        device_->GetImmediateContext(&context_);
        return ok();
    }

    Outcome create_transform()
    {
        MFT_REGISTER_TYPE_INFO input_info = {MFMediaType_Video, MFVideoFormat_H264};
        MFT_REGISTER_TYPE_INFO output_info = {MFMediaType_Video, MFVideoFormat_NV12};

        const UINT32 hardware_flags =
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER;
        const UINT32 software_flags = MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER;

        const UINT32 attempts[2] = {hardware_flags, software_flags};

        for (const UINT32 flags : attempts) {
            if (flags == hardware_flags && !manager_) {
                continue;
            }

            IMFActivate** activations = nullptr;
            UINT32 count = 0;
            const HRESULT enumerated = MFTEnumEx(MFT_CATEGORY_VIDEO_DECODER, flags, &input_info,
                                                 &output_info, &activations, &count);
            if (FAILED(enumerated) || count == 0) {
                if (activations != nullptr) {
                    CoTaskMemFree(activations);
                }
                continue;
            }

            HRESULT activated = E_FAIL;
            for (UINT32 index = 0; index < count && FAILED(activated); ++index) {
                activated = activations[index]->ActivateObject(IID_PPV_ARGS(&transform_));
            }
            for (UINT32 index = 0; index < count; ++index) {
                activations[index]->Release();
            }
            CoTaskMemFree(activations);

            if (SUCCEEDED(activated)) {
                if (flags == hardware_flags && manager_) {
                    const HRESULT bound = transform_->ProcessMessage(
                        MFT_MESSAGE_SET_D3D_MANAGER, reinterpret_cast<ULONG_PTR>(manager_.Get()));
                    if (FAILED(bound)) {
                        TL_LOG_WARN("receptor: MFT recusou o device D3D11, decode em software");
                        manager_.Reset();
                        context_.Reset();
                    }
                } else {
                    manager_.Reset();
                    context_.Reset();
                }
                return ok();
            }
        }

        return fail(Status::Unavailable, "create_transform: nenhum decoder H264 disponivel");
    }

    Outcome configure_types()
    {
        if (config_.low_latency) {
            ComPtr<IMFAttributes> attributes;
            if (SUCCEEDED(transform_->GetAttributes(&attributes))) {
                attributes->SetUINT32(MF_LOW_LATENCY, TRUE);
            }
        }

        ComPtr<IMFMediaType> input_type;
        HRESULT result = MFCreateMediaType(&input_type);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "configure_types: MFCreateMediaType", result);
        }

        input_type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
        input_type->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_H264);
        input_type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive);
        MFSetAttributeSize(input_type.Get(), MF_MT_FRAME_SIZE, config_.width, config_.height);
        MFSetAttributeRatio(input_type.Get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1);

        result = transform_->SetInputType(0, input_type.Get(), 0);
        if (FAILED(result)) {
            return fail(Status::NotSupported, "configure_types: SetInputType", result);
        }

        TL_TRY(configure_output_type());

        MFT_OUTPUT_STREAM_INFO stream_info = {};
        if (SUCCEEDED(transform_->GetOutputStreamInfo(0, &stream_info))) {
            provides_samples_ =
                (stream_info.dwFlags &
                 (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES)) != 0;
            output_bytes_ = stream_info.cbSize;
        }
        if (output_bytes_ == 0) {
            output_bytes_ = config_.width * config_.height * 3 / 2;
        }
        return ok();
    }

    Outcome configure_output_type()
    {
        for (DWORD index = 0;; ++index) {
            ComPtr<IMFMediaType> candidate;
            const HRESULT available = transform_->GetOutputAvailableType(0, index, &candidate);
            if (available == MF_E_NO_MORE_TYPES) {
                break;
            }
            if (FAILED(available)) {
                return fail(Status::PlatformError, "configure_output_type: GetOutputAvailableType",
                            available);
            }

            GUID subtype = {};
            if (FAILED(candidate->GetGUID(MF_MT_SUBTYPE, &subtype)) ||
                subtype != MFVideoFormat_NV12) {
                continue;
            }

            if (SUCCEEDED(transform_->SetOutputType(0, candidate.Get(), 0))) {
                return ok();
            }
        }
        return fail(Status::NotSupported, "configure_output_type: sem NV12");
    }

    Outcome create_input_pool()
    {
        input_capacity_ = kDefaultInputBytes;

        for (std::uint32_t index = 0; index < kInputPoolSize; ++index) {
            HRESULT result =
                MFCreateMemoryBuffer(static_cast<DWORD>(input_capacity_), &input_buffers_[index]);
            if (FAILED(result)) {
                return fail(Status::OutOfMemory, "create_input_pool: MFCreateMemoryBuffer", result);
            }

            result = MFCreateSample(&input_samples_[index]);
            if (FAILED(result)) {
                return fail(Status::OutOfMemory, "create_input_pool: MFCreateSample", result);
            }

            result = input_samples_[index]->AddBuffer(input_buffers_[index].Get());
            if (FAILED(result)) {
                return fail(Status::PlatformError, "create_input_pool: AddBuffer", result);
            }
        }

        frame_duration_ns_ = kNanosecondsPerSecond / 60;
        return ok();
    }

    Outcome allocate_output_sample(ComPtr<IMFSample>& out)
    {
        ComPtr<IMFMediaBuffer> buffer;
        HRESULT result = MFCreateMemoryBuffer(static_cast<DWORD>(output_bytes_), &buffer);
        if (FAILED(result)) {
            return fail(Status::OutOfMemory, "allocate_output_sample: MFCreateMemoryBuffer",
                        result);
        }

        result = MFCreateSample(&out);
        if (FAILED(result)) {
            return fail(Status::OutOfMemory, "allocate_output_sample: MFCreateSample", result);
        }

        result = out->AddBuffer(buffer.Get());
        if (FAILED(result)) {
            return fail(Status::PlatformError, "allocate_output_sample: AddBuffer", result);
        }
        return ok();
    }

    Outcome create_share_texture()
    {
        if (device_ == nullptr) {
            return ok();
        }

        D3D11_TEXTURE2D_DESC description = {};
        description.Width = config_.width;
        description.Height = config_.height;
        description.MipLevels = 1;
        description.ArraySize = 1;
        description.Format = DXGI_FORMAT_NV12;
        description.SampleDesc.Count = 1;
        description.Usage = D3D11_USAGE_DEFAULT;
        description.BindFlags = D3D11_BIND_SHADER_RESOURCE;

        const HRESULT result = device_->CreateTexture2D(&description, nullptr, &share_texture_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "create_share_texture: CreateTexture2D", result);
        }
        return ok();
    }

    VideoDecoderConfig config_;
    ID3D11Device* device_ = nullptr;
    ComPtr<ID3D11DeviceContext> context_;
    ComPtr<IMFDXGIDeviceManager> manager_;
    ComPtr<IMFTransform> transform_;
    ComPtr<ID3D11Texture2D> share_texture_;

    ComPtr<IMFSample> input_samples_[kInputPoolSize];
    ComPtr<IMFMediaBuffer> input_buffers_[kInputPoolSize];
    InputStamp stamps_[kInputPoolSize];

    ComPtr<IMFSample> ready_;
    ComPtr<IMFSample> held_sample_;
    ComPtr<IMFMediaBuffer> held_buffer_;
    ComPtr<IMF2DBuffer2> held_planar_;

    std::size_t input_capacity_ = 0;
    std::size_t output_bytes_ = 0;
    Nanoseconds frame_duration_ns_ = 0;
    std::uint32_t next_input_ = 0;
    std::uint32_t next_stamp_ = 0;
    bool provides_samples_ = false;
    bool com_initialized_ = false;
    bool mf_started_ = false;
    bool started_ = false;
};

}  // namespace

Result<std::unique_ptr<VideoDecoder>> create_media_foundation_decoder(
    const VideoDecoderConfig& config, void* native_device)
{
    std::unique_ptr<VideoDecoder> decoder(new (std::nothrow) MediaFoundationDecoder(
        config, static_cast<ID3D11Device*>(native_device)));
    if (!decoder) {
        return Error{Status::OutOfMemory, "create_media_foundation_decoder"};
    }
    return decoder;
}

}  // namespace tl::receive
