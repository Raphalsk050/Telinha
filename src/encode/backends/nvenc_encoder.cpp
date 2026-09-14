#include "nvenc_encoder.hpp"

#if TL_PLATFORM_WINDOWS

#include <cstring>

#include "d3d_frame_converter.hpp"
#include "nvenc_library.hpp"
#include "region_of_interest.hpp"
#include "telinha/core/arena.hpp"
#include "telinha/core/log.hpp"

namespace tl::encode {
namespace {

constexpr std::uint32_t kSurfaceCount = 4;
constexpr std::uint32_t kOutputCount = 4;
constexpr std::size_t kEncoderArenaBytes = 1u << 20;

[[nodiscard]] GUID codec_guid(VideoCodec codec) noexcept
{
    switch (codec) {
        case VideoCodec::Hevc: return NV_ENC_CODEC_HEVC_GUID;
        case VideoCodec::Av1: return NV_ENC_CODEC_AV1_GUID;
        case VideoCodec::H264:
        case VideoCodec::Unknown: break;
    }
    return NV_ENC_CODEC_H264_GUID;
}

[[nodiscard]] NV_ENC_PARAMS_RC_MODE rate_control_mode(RateControlMode mode) noexcept
{
    switch (mode) {
        case RateControlMode::VariableBitrate: return NV_ENC_PARAMS_RC_VBR;
        case RateControlMode::ConstantQuality: return NV_ENC_PARAMS_RC_CONSTQP;
        case RateControlMode::ConstantBitrate: break;
    }
    return NV_ENC_PARAMS_RC_CBR;
}

[[nodiscard]] std::uint32_t rounded_up_even(std::uint32_t value) noexcept
{
    return (value + 1u) & ~1u;
}

class NvencEncoder final : public VideoEncoder {
public:
    NvencEncoder(const VideoEncoderConfig& config, ID3D11Device* device) noexcept
        : config_(config), device_(device)
    {}

    ~NvencEncoder() override { stop(); }

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
    [[nodiscard]] const NV_ENCODE_API_FUNCTION_LIST& api() const noexcept
    {
        return NvencLibrary::instance().functions();
    }

    [[nodiscard]] Outcome open_session() noexcept;
    [[nodiscard]] Outcome configure() noexcept;
    [[nodiscard]] Outcome register_surfaces() noexcept;
    [[nodiscard]] Outcome create_output_buffers() noexcept;
    void destroy_resources() noexcept;
    void flush() noexcept;

    VideoEncoderConfig config_;
    ID3D11Device* device_ = nullptr;

    D3dFrameConverter converter_;
    ArenaStorage arena_storage_;
    RegionOfInterestMap region_of_interest_;

    void* session_ = nullptr;
    NV_ENC_INITIALIZE_PARAMS init_params_{};
    NV_ENC_CONFIG encode_config_{};

    NV_ENC_REGISTERED_PTR registered_[kSurfaceCount] = {};
    NV_ENC_OUTPUT_PTR outputs_[kOutputCount] = {};

    std::uint32_t pending_[kOutputCount] = {};
    Nanoseconds pending_capture_ns_[kOutputCount] = {};
    std::uint64_t pending_frame_index_[kOutputCount] = {};

    std::uint32_t pending_head_ = 0;
    std::uint32_t pending_count_ = 0;
    std::uint32_t next_output_ = 0;

    std::uint32_t encode_width_ = 0;
    std::uint32_t encode_height_ = 0;

    std::uint64_t frame_index_ = 0;
    bool started_ = false;
    bool keyframe_pending_ = true;
    bool locked_ = false;
    bool region_of_interest_active_ = false;
};

Outcome NvencEncoder::initialize() noexcept
{
    if (device_ == nullptr) {
        return fail(Status::InvalidArgument, "nvenc: a D3D11 device is required");
    }
    if (config_.width == 0 || config_.height == 0) {
        return fail(Status::InvalidArgument, "nvenc: the encoded size must be known up front");
    }

    encode_width_ = rounded_up_even(config_.width);
    encode_height_ = rounded_up_even(config_.height);

    if (!arena_storage_.reserve(kEncoderArenaBytes)) {
        return fail(Status::OutOfMemory, "nvenc: arena reservation failed");
    }
    if (!region_of_interest_.initialize(arena_storage_.arena(), encode_width_, encode_height_,
                                        config_.coding_unit_size)) {
        return fail(Status::OutOfMemory, "nvenc: region of interest allocation failed");
    }

    TL_TRY(converter_.initialize(device_, encode_width_, encode_height_, kSurfaceCount));
    TL_TRY(open_session());
    TL_TRY(configure());
    TL_TRY(register_surfaces());
    TL_TRY(create_output_buffers());

    return ok();
}

Outcome NvencEncoder::open_session() noexcept
{
    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS params{};
    params.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
    params.deviceType = NV_ENC_DEVICE_TYPE_DIRECTX;
    params.device = device_;
    params.apiVersion = NVENCAPI_VERSION;

    void* session = nullptr;
    const NVENCSTATUS status = api().nvEncOpenEncodeSessionEx(&params, &session);
    if (status != NV_ENC_SUCCESS) {
        return from_nvenc_status(status, "nvenc: nvEncOpenEncodeSessionEx failed");
    }
    session_ = session;
    return ok();
}

Outcome NvencEncoder::configure() noexcept
{
    const GUID codec = codec_guid(config_.codec);
    const GUID preset = NV_ENC_PRESET_P4_GUID;
    const NV_ENC_TUNING_INFO tuning = config_.low_latency ? NV_ENC_TUNING_INFO_ULTRA_LOW_LATENCY
                                                          : NV_ENC_TUNING_INFO_HIGH_QUALITY;

    NV_ENC_PRESET_CONFIG preset_config{};
    preset_config.version = NV_ENC_PRESET_CONFIG_VER;
    preset_config.presetCfg.version = NV_ENC_CONFIG_VER;

    NVENCSTATUS status =
        api().nvEncGetEncodePresetConfigEx(session_, codec, preset, tuning, &preset_config);
    if (status != NV_ENC_SUCCESS) {
        return from_nvenc_status(status, "nvenc: nvEncGetEncodePresetConfigEx failed");
    }

    encode_config_ = preset_config.presetCfg;
    encode_config_.version = NV_ENC_CONFIG_VER;
    encode_config_.frameIntervalP = 1;
    encode_config_.gopLength =
        config_.gop_length == 0 ? NVENC_INFINITE_GOPLENGTH : config_.gop_length;

    NV_ENC_RC_PARAMS& rc = encode_config_.rcParams;
    rc.rateControlMode = rate_control_mode(config_.rate_control);
    rc.averageBitRate = config_.target_bitrate_bps;
    rc.maxBitRate = config_.max_bitrate_bps;
    rc.enableAQ = 1;
    rc.zeroReorderDelay = 1;

    if (rc.rateControlMode == NV_ENC_PARAMS_RC_CONSTQP) {
        rc.constQP.qpIntra = config_.constant_quality;
        rc.constQP.qpInterP = config_.constant_quality;
        rc.constQP.qpInterB = config_.constant_quality;
    } else if (config_.framerate_millihertz != 0) {
        const std::uint64_t bits_per_frame =
            (static_cast<std::uint64_t>(config_.target_bitrate_bps) * 1000ull) /
            config_.framerate_millihertz;
        rc.vbvBufferSize = static_cast<std::uint32_t>(bits_per_frame);
        rc.vbvInitialDelay = rc.vbvBufferSize;
    }

    region_of_interest_active_ =
        config_.region_of_interest && rc.rateControlMode != NV_ENC_PARAMS_RC_CONSTQP;
    if (region_of_interest_active_) {
        rc.qpMapMode = NV_ENC_QP_MAP_DELTA;
    }

    if (config_.codec == VideoCodec::H264) {
        encode_config_.profileGUID = NV_ENC_H264_PROFILE_HIGH_GUID;
        NV_ENC_CONFIG_H264& h264 = encode_config_.encodeCodecConfig.h264Config;
        h264.idrPeriod = encode_config_.gopLength;
        h264.repeatSPSPPS = config_.repeat_parameter_sets ? 1u : 0u;
        h264.maxNumRefFrames = config_.max_reference_frames;
        h264.sliceMode = 3;
        h264.sliceModeData = 1;
        if (config_.intra_refresh && encode_config_.gopLength == NVENC_INFINITE_GOPLENGTH) {
            h264.enableIntraRefresh = 1;
            h264.intraRefreshPeriod = config_.intra_refresh_period_frames;
            h264.intraRefreshCnt = config_.intra_refresh_length_frames;
        }
    }

    std::memset(&init_params_, 0, sizeof(init_params_));
    init_params_.version = NV_ENC_INITIALIZE_PARAMS_VER;
    init_params_.encodeGUID = codec;
    init_params_.presetGUID = preset;
    init_params_.tuningInfo = tuning;
    init_params_.encodeWidth = encode_width_;
    init_params_.encodeHeight = encode_height_;
    init_params_.darWidth = encode_width_;
    init_params_.darHeight = encode_height_;
    init_params_.frameRateNum = config_.framerate_millihertz;
    init_params_.frameRateDen = 1000;
    init_params_.enablePTD = 1;
    init_params_.encodeConfig = &encode_config_;

    status = api().nvEncInitializeEncoder(session_, &init_params_);
    if (status != NV_ENC_SUCCESS) {
        return from_nvenc_status(status, "nvenc: nvEncInitializeEncoder failed");
    }

    TL_LOG_INFO("nvenc ready: %ux%u, %s, %u kbps", encode_width_, encode_height_,
                to_string(config_.codec), config_.target_bitrate_bps / 1000u);
    return ok();
}

Outcome NvencEncoder::register_surfaces() noexcept
{
    for (std::uint32_t i = 0; i < kSurfaceCount; ++i) {
        ID3D11Texture2D* texture = converter_.surface(i);
        if (texture == nullptr) {
            return fail(Status::Unavailable, "nvenc: the converter surface pool is incomplete");
        }

        NV_ENC_REGISTER_RESOURCE resource{};
        resource.version = NV_ENC_REGISTER_RESOURCE_VER;
        resource.resourceType = NV_ENC_INPUT_RESOURCE_TYPE_DIRECTX;
        resource.resourceToRegister = texture;
        resource.width = encode_width_;
        resource.height = encode_height_;
        resource.pitch = 0;
        resource.bufferFormat = NV_ENC_BUFFER_FORMAT_NV12;
        resource.bufferUsage = NV_ENC_INPUT_IMAGE;

        const NVENCSTATUS status = api().nvEncRegisterResource(session_, &resource);
        if (status != NV_ENC_SUCCESS) {
            return from_nvenc_status(status, "nvenc: nvEncRegisterResource failed");
        }
        registered_[i] = resource.registeredResource;
    }
    return ok();
}

Outcome NvencEncoder::create_output_buffers() noexcept
{
    for (std::uint32_t i = 0; i < kOutputCount; ++i) {
        NV_ENC_CREATE_BITSTREAM_BUFFER buffer{};
        buffer.version = NV_ENC_CREATE_BITSTREAM_BUFFER_VER;

        const NVENCSTATUS status = api().nvEncCreateBitstreamBuffer(session_, &buffer);
        if (status != NV_ENC_SUCCESS) {
            return from_nvenc_status(status, "nvenc: nvEncCreateBitstreamBuffer failed");
        }
        outputs_[i] = buffer.bitstreamBuffer;
    }
    return ok();
}

Outcome NvencEncoder::start()
{
    if (session_ == nullptr) {
        return fail(Status::Unavailable, "nvenc: not initialized");
    }
    started_ = true;
    keyframe_pending_ = true;
    return ok();
}

void NvencEncoder::flush() noexcept
{
    if (session_ == nullptr || !started_) {
        return;
    }

    NV_ENC_PIC_PARAMS pic{};
    pic.version = NV_ENC_PIC_PARAMS_VER;
    pic.encodePicFlags = NV_ENC_PIC_FLAG_EOS;
    (void)api().nvEncEncodePicture(session_, &pic);
}

void NvencEncoder::destroy_resources() noexcept
{
    if (session_ == nullptr) {
        return;
    }

    for (std::uint32_t i = 0; i < kOutputCount; ++i) {
        if (outputs_[i] != nullptr) {
            (void)api().nvEncDestroyBitstreamBuffer(session_, outputs_[i]);
            outputs_[i] = nullptr;
        }
    }
    for (std::uint32_t i = 0; i < kSurfaceCount; ++i) {
        if (registered_[i] != nullptr) {
            (void)api().nvEncUnregisterResource(session_, registered_[i]);
            registered_[i] = nullptr;
        }
    }

    (void)api().nvEncDestroyEncoder(session_);
    session_ = nullptr;
}

void NvencEncoder::stop() noexcept
{
    release();
    flush();
    destroy_resources();
    converter_.shutdown();

    started_ = false;
    pending_head_ = 0;
    pending_count_ = 0;
    next_output_ = 0;
}

Outcome NvencEncoder::submit(const capture::ClassifiedFrame& frame)
{
    if (!started_ || session_ == nullptr) {
        return fail(Status::Unavailable, "nvenc: not started");
    }
    if (frame.frame == nullptr || frame.tiles == nullptr) {
        return fail(Status::InvalidArgument, "nvenc: the classified frame is empty");
    }

    const capture::FrameSurface& surface = frame.frame->surface;
    if (surface.memory != capture::SurfaceMemory::GpuTexture || surface.gpu_texture == nullptr) {
        return fail(Status::NotSupported, "nvenc: a GPU texture is required");
    }
    if (pending_count_ == kOutputCount) {
        return fail(Status::Full, "nvenc: every output buffer is still in flight");
    }

    std::uint32_t surface_index = 0;
    TL_TRY(converter_.convert(static_cast<ID3D11Texture2D*>(surface.gpu_texture), surface.rotation,
                              surface_index));

    NV_ENC_MAP_INPUT_RESOURCE mapped{};
    mapped.version = NV_ENC_MAP_INPUT_RESOURCE_VER;
    mapped.registeredResource = registered_[surface_index];

    NVENCSTATUS status = api().nvEncMapInputResource(session_, &mapped);
    if (status != NV_ENC_SUCCESS) {
        return from_nvenc_status(status, "nvenc: nvEncMapInputResource failed");
    }

    const std::uint32_t output_index = next_output_;

    NV_ENC_PIC_PARAMS pic{};
    pic.version = NV_ENC_PIC_PARAMS_VER;
    pic.inputBuffer = mapped.mappedResource;
    pic.bufferFmt = mapped.mappedBufferFmt;
    pic.inputWidth = encode_width_;
    pic.inputHeight = encode_height_;
    pic.outputBitstream = outputs_[output_index];
    pic.pictureStruct = NV_ENC_PIC_STRUCT_FRAME;
    pic.inputTimeStamp = frame_index_;

    if (keyframe_pending_) {
        pic.encodePicFlags = NV_ENC_PIC_FLAG_FORCEIDR | NV_ENC_PIC_FLAG_OUTPUT_SPSPPS;
    }

    if (region_of_interest_active_) {
        region_of_interest_.build(*frame.tiles, config_.dirty_region_qp_delta,
                                  config_.static_region_qp_delta);
        pic.qpDeltaMap = const_cast<std::int8_t*>(region_of_interest_.deltas().data());
        pic.qpDeltaMapSize = region_of_interest_.unit_count();
    }

    status = api().nvEncEncodePicture(session_, &pic);
    (void)api().nvEncUnmapInputResource(session_, mapped.mappedResource);

    if (status == NV_ENC_ERR_NEED_MORE_INPUT) {
        return fail(Status::NotSupported,
                    "nvenc: the encoder buffered the frame, which this configuration forbids");
    }
    if (status != NV_ENC_SUCCESS) {
        return from_nvenc_status(status, "nvenc: nvEncEncodePicture failed");
    }

    const std::uint32_t slot = (pending_head_ + pending_count_) % kOutputCount;
    pending_[slot] = output_index;
    pending_capture_ns_[slot] = frame.frame->metadata.present_time_ns;
    pending_frame_index_[slot] = frame_index_;
    ++pending_count_;

    next_output_ = (next_output_ + 1) % kOutputCount;
    ++frame_index_;
    keyframe_pending_ = false;
    return ok();
}

Outcome NvencEncoder::poll(transport::EncodedVideoFrame& out, std::uint32_t timeout_ms)
{
    out = transport::EncodedVideoFrame{};

    if (!started_ || session_ == nullptr) {
        return fail(Status::Unavailable, "nvenc: not started");
    }
    if (locked_) {
        return fail(Status::AlreadyExists, "nvenc: the previous packet was not released");
    }
    if (pending_count_ == 0) {
        return fail(Status::Timeout, "nvenc: no encoded frame is pending");
    }

    NV_ENC_LOCK_BITSTREAM lock{};
    lock.version = NV_ENC_LOCK_BITSTREAM_VER;
    lock.outputBitstream = outputs_[pending_[pending_head_]];
    lock.doNotWait = timeout_ms == 0 ? 1u : 0u;

    const NVENCSTATUS status = api().nvEncLockBitstream(session_, &lock);
    if (status == NV_ENC_ERR_LOCK_BUSY) {
        return fail(Status::Timeout, "nvenc: the encoded frame is not ready");
    }
    if (status != NV_ENC_SUCCESS) {
        return from_nvenc_status(status, "nvenc: nvEncLockBitstream failed");
    }

    locked_ = true;

    const bool key = lock.pictureType == NV_ENC_PIC_TYPE_IDR;

    out.bitstream = Span<const std::byte>(static_cast<const std::byte*>(lock.bitstreamBufferPtr),
                                          lock.bitstreamSizeInBytes);
    out.capture_time_ns = pending_capture_ns_[pending_head_];
    out.encode_end_time_ns = now_ns();
    out.frame_index = pending_frame_index_[pending_head_];
    out.width = encode_width_;
    out.height = encode_height_;
    out.average_qp = static_cast<std::int32_t>(lock.frameAvgQP);
    out.codec = to_wire_codec(config_.codec);
    out.kind = key ? transport::WireFrameKind::Key : transport::WireFrameKind::Delta;
    out.temporal_index = 0;
    out.spatial_index = 0;
    return ok();
}

void NvencEncoder::release() noexcept
{
    if (!locked_ || session_ == nullptr) {
        return;
    }

    (void)api().nvEncUnlockBitstream(session_, outputs_[pending_[pending_head_]]);
    pending_head_ = (pending_head_ + 1) % kOutputCount;
    --pending_count_;
    locked_ = false;
}

Outcome NvencEncoder::set_target_bitrate(std::uint32_t bits_per_second) noexcept
{
    if (session_ == nullptr || bits_per_second == 0) {
        return fail(Status::InvalidArgument, "nvenc: set_target_bitrate");
    }

    encode_config_.rcParams.averageBitRate = bits_per_second;
    if (encode_config_.rcParams.maxBitRate < bits_per_second) {
        encode_config_.rcParams.maxBitRate = bits_per_second;
    }
    if (config_.framerate_millihertz != 0 &&
        encode_config_.rcParams.rateControlMode != NV_ENC_PARAMS_RC_CONSTQP) {
        const std::uint64_t bits_per_frame =
            (static_cast<std::uint64_t>(bits_per_second) * 1000ull) / config_.framerate_millihertz;
        encode_config_.rcParams.vbvBufferSize = static_cast<std::uint32_t>(bits_per_frame);
        encode_config_.rcParams.vbvInitialDelay = encode_config_.rcParams.vbvBufferSize;
    }

    NV_ENC_RECONFIGURE_PARAMS reconfigure{};
    reconfigure.version = NV_ENC_RECONFIGURE_PARAMS_VER;
    reconfigure.reInitEncodeParams = init_params_;
    reconfigure.reInitEncodeParams.encodeConfig = &encode_config_;

    const NVENCSTATUS status = api().nvEncReconfigureEncoder(session_, &reconfigure);
    if (status != NV_ENC_SUCCESS) {
        return from_nvenc_status(status, "nvenc: nvEncReconfigureEncoder failed");
    }

    config_.target_bitrate_bps = bits_per_second;
    return ok();
}

VideoEncoderInfo NvencEncoder::info() const noexcept
{
    VideoEncoderInfo out;
    out.codec = config_.codec;
    out.backend = VideoEncoderBackend::Nvenc;
    out.width = encode_width_;
    out.height = encode_height_;
    out.coding_unit_size = config_.coding_unit_size;
    out.accepts_gpu_surfaces = true;
    out.supports_dirty_regions = region_of_interest_active_;
    out.supports_intra_refresh = true;
    return out;
}

}  // namespace

bool nvenc_available(VideoCodec codec) noexcept
{
    return NvencLibrary::instance().loaded() &&
           (codec == VideoCodec::H264 || codec == VideoCodec::Hevc || codec == VideoCodec::Av1);
}

Result<std::unique_ptr<VideoEncoder>> create_nvenc_encoder(const VideoEncoderConfig& config,
                                                           void* native_device)
{
    if (!NvencLibrary::instance().loaded()) {
        return Error{Status::Unavailable, "nvenc: nvEncodeAPI64.dll is not present"};
    }

    auto encoder =
        std::make_unique<NvencEncoder>(config, static_cast<ID3D11Device*>(native_device));
    const Outcome prepared = encoder->initialize();
    if (!prepared.ok()) {
        return prepared.error();
    }
    return std::unique_ptr<VideoEncoder>(encoder.release());
}

}  // namespace tl::encode

#endif
