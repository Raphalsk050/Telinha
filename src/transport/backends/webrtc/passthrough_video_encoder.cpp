#include "backends/webrtc/passthrough_video_encoder.hpp"

#include <optional>
#include <string>
#include <utility>

#include "api/video/video_frame.h"
#include "api/video/video_frame_type.h"
#include "api/video_codecs/h264_profile_level_id.h"
#include "api/video_codecs/video_codec.h"
#include "backends/webrtc/encoded_video_buffer.hpp"
#include "modules/video_coding/include/video_codec_interface.h"
#include "modules/video_coding/include/video_error_codes.h"

namespace tl::transport::backend {
namespace {

webrtc::VideoCodecType to_webrtc_codec(WireVideoCodec codec) noexcept
{
    switch (codec) {
        case WireVideoCodec::H264: return webrtc::kVideoCodecH264;
        case WireVideoCodec::Vp8: return webrtc::kVideoCodecVP8;
        case WireVideoCodec::Vp9: return webrtc::kVideoCodecVP9;
        case WireVideoCodec::Av1: return webrtc::kVideoCodecAV1;
        case WireVideoCodec::Unknown: break;
    }
    return webrtc::kVideoCodecGeneric;
}

const char* codec_name(WireVideoCodec codec) noexcept
{
    switch (codec) {
        case WireVideoCodec::H264: return "H264";
        case WireVideoCodec::Vp8: return "VP8";
        case WireVideoCodec::Vp9: return "VP9";
        case WireVideoCodec::Av1: return "AV1";
        case WireVideoCodec::Unknown: break;
    }
    return "";
}

}  // namespace

webrtc::SdpVideoFormat sdp_format_for(WireVideoCodec codec, const char* h264_profile_level_id)
{
    webrtc::CodecParameterMap parameters;
    if (codec == WireVideoCodec::H264) {
        parameters["level-asymmetry-allowed"] = "1";
        parameters["packetization-mode"] = "1";
        if (h264_profile_level_id != nullptr && h264_profile_level_id[0] != '\0') {
            parameters["profile-level-id"] = h264_profile_level_id;
        } else {
            const std::optional<std::string> generated =
                webrtc::H264ProfileLevelIdToString(webrtc::H264ProfileLevelId(
                    webrtc::H264Profile::kProfileHigh, webrtc::H264Level::kLevel5_2));
            parameters["profile-level-id"] = generated.value_or(std::string("640c34"));
        }
    }
    return webrtc::SdpVideoFormat(codec_name(codec), std::move(parameters));
}

PassthroughVideoEncoder::PassthroughVideoEncoder(WireVideoCodec codec, EncoderFeedback& feedback)
    : feedback_(&feedback), codec_(codec)
{}

void PassthroughVideoEncoder::SetFecControllerOverride(webrtc::FecControllerOverride* controller)
{
    (void)controller;
}

int PassthroughVideoEncoder::InitEncode(const webrtc::VideoCodec* codec_settings,
                                        const Settings& settings)
{
    (void)settings;
    if (codec_settings != nullptr) {
        target_bitrate_bps_ = codec_settings->startBitrate * 1000u;
        framerate_hz_ = codec_settings->maxFramerate;
    }
    awaiting_keyframe_ = true;
    return WEBRTC_VIDEO_CODEC_OK;
}

int32_t PassthroughVideoEncoder::RegisterEncodeCompleteCallback(
    webrtc::EncodedImageCallback* callback)
{
    callback_ = callback;
    return WEBRTC_VIDEO_CODEC_OK;
}

int32_t PassthroughVideoEncoder::Release()
{
    callback_ = nullptr;
    return WEBRTC_VIDEO_CODEC_OK;
}

int32_t PassthroughVideoEncoder::Encode(const webrtc::VideoFrame& frame,
                                        const std::vector<webrtc::VideoFrameType>* frame_types)
{
    if (callback_ == nullptr) {
        return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
    }

    const webrtc::scoped_refptr<webrtc::VideoFrameBuffer>& buffer = frame.video_frame_buffer();
    if (buffer == nullptr || buffer->type() != webrtc::VideoFrameBuffer::Type::kNative) {
        return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
    }

    const EncodedVideoBuffer& payload = static_cast<const EncodedVideoBuffer&>(*buffer);

    bool keyframe_wanted = awaiting_keyframe_;
    if (frame_types != nullptr) {
        for (webrtc::VideoFrameType type : *frame_types) {
            if (type == webrtc::VideoFrameType::kVideoFrameKey) {
                keyframe_wanted = true;
            }
        }
    }

    if (keyframe_wanted && !payload.keyframe()) {
        feedback_->on_keyframe_requested();
        awaiting_keyframe_ = true;
        callback_->OnFrameDropped(frame.rtp_timestamp(), 0, true);
        return WEBRTC_VIDEO_CODEC_OK;
    }

    awaiting_keyframe_ = false;

    webrtc::EncodedImage image;
    image.SetEncodedData(payload.bitstream());
    image._encodedWidth = static_cast<uint32_t>(payload.width());
    image._encodedHeight = static_cast<uint32_t>(payload.height());
    image.SetRtpTimestamp(frame.rtp_timestamp());
    image.capture_time_ms_ = frame.render_time_ms();
    image.ntp_time_ms_ = frame.ntp_time_ms();
    image._frameType = payload.keyframe() ? webrtc::VideoFrameType::kVideoFrameKey
                                          : webrtc::VideoFrameType::kVideoFrameDelta;
    image.qp_ = static_cast<int>(payload.average_qp());
    image.content_type_ = webrtc::VideoContentType::SCREENSHARE;
    image.rotation_ = webrtc::kVideoRotation_0;
    image.SetSpatialIndex(payload.spatial_index());
    image.SetTemporalIndex(payload.temporal_index());
    image.SetPlayoutDelay(webrtc::VideoPlayoutDelay::Minimal());

    webrtc::CodecSpecificInfo codec_specific;
    codec_specific.codecType = to_webrtc_codec(codec_);
    codec_specific.end_of_picture = true;
    if (codec_specific.codecType == webrtc::kVideoCodecH264) {
        codec_specific.codecSpecific.H264.packetization_mode =
            webrtc::H264PacketizationMode::NonInterleaved;
        codec_specific.codecSpecific.H264.temporal_idx = webrtc::kNoTemporalIdx;
        codec_specific.codecSpecific.H264.base_layer_sync = false;
        codec_specific.codecSpecific.H264.idr_frame = payload.keyframe();
    }

    const webrtc::EncodedImageCallback::Result result =
        callback_->OnEncodedImage(image, &codec_specific);
    if (result.error != webrtc::EncodedImageCallback::Result::OK) {
        return WEBRTC_VIDEO_CODEC_ERROR;
    }

    return WEBRTC_VIDEO_CODEC_OK;
}

void PassthroughVideoEncoder::SetRates(const RateControlParameters& parameters)
{
    const std::uint32_t bitrate_bps = parameters.bitrate.get_sum_bps();
    const std::uint32_t framerate_hz = parameters.framerate_fps > 0.0
                                           ? static_cast<std::uint32_t>(parameters.framerate_fps)
                                           : framerate_hz_;
    if (bitrate_bps == target_bitrate_bps_ && framerate_hz == framerate_hz_) {
        return;
    }
    target_bitrate_bps_ = bitrate_bps;
    framerate_hz_ = framerate_hz;
    feedback_->on_target_bitrate(bitrate_bps, framerate_hz);
}

webrtc::VideoEncoder::EncoderInfo PassthroughVideoEncoder::GetEncoderInfo() const
{
    EncoderInfo info;
    info.implementation_name = "TelinhaPassthrough";
    info.supports_native_handle = true;
    info.has_trusted_rate_controller = true;
    info.is_hardware_accelerated = true;
    info.supports_simulcast = false;
    info.scaling_settings = VideoEncoder::ScalingSettings::kOff;
    return info;
}

PassthroughVideoEncoderFactory::PassthroughVideoEncoderFactory(webrtc::SdpVideoFormat format,
                                                               WireVideoCodec codec,
                                                               EncoderFeedback& feedback)
    : format_(std::move(format)), codec_(codec), feedback_(&feedback)
{}

std::vector<webrtc::SdpVideoFormat> PassthroughVideoEncoderFactory::GetSupportedFormats() const
{
    return {format_};
}

std::unique_ptr<webrtc::VideoEncoder> PassthroughVideoEncoderFactory::Create(
    const webrtc::Environment& env, const webrtc::SdpVideoFormat& format)
{
    (void)env;
    (void)format;
    return std::make_unique<PassthroughVideoEncoder>(codec_, *feedback_);
}

}  // namespace tl::transport::backend
