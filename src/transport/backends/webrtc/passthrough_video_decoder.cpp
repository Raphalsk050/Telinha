#include "backends/webrtc/passthrough_video_decoder.hpp"

#include <cstddef>
#include <utility>

#include "api/video/encoded_image.h"
#include "modules/video_coding/include/video_error_codes.h"
#include "telinha/core/clock.hpp"
#include "telinha/core/span.hpp"

namespace tl::transport::backend {

PassthroughVideoDecoder::PassthroughVideoDecoder(WireVideoCodec codec, RemoteVideoSink* sink)
    : codec_(codec), sink_(sink)
{}

bool PassthroughVideoDecoder::Configure(const Settings& settings)
{
    (void)settings;
    return true;
}

int32_t PassthroughVideoDecoder::Decode(const webrtc::EncodedImage& input_image,
                                        int64_t render_time_ms)
{
    (void)render_time_ms;
    if (sink_ == nullptr || input_image.size() == 0) {
        return WEBRTC_VIDEO_CODEC_OK;
    }

    EncodedVideoFrame frame;
    frame.bitstream = Span<const std::byte>(reinterpret_cast<const std::byte*>(input_image.data()),
                                            input_image.size());
    frame.codec = codec_;
    frame.capture_time_ns =
        input_image.capture_time_ms_ > 0
            ? static_cast<Nanoseconds>(input_image.capture_time_ms_) * kNanosecondsPerMillisecond
            : 0;
    frame.width = input_image._encodedWidth;
    frame.height = input_image._encodedHeight;
    frame.average_qp = input_image.qp_;
    frame.kind = input_image.IsKey() ? WireFrameKind::Key : WireFrameKind::Delta;
    frame.temporal_index = static_cast<std::uint8_t>(input_image.TemporalIndex().value_or(0));
    frame.spatial_index = static_cast<std::uint8_t>(input_image.SpatialIndex().value_or(0));

    sink_->on_remote_video(frame);
    return WEBRTC_VIDEO_CODEC_OK;
}

int32_t PassthroughVideoDecoder::RegisterDecodeCompleteCallback(
    webrtc::DecodedImageCallback* callback)
{
    callback_ = callback;
    return WEBRTC_VIDEO_CODEC_OK;
}

int32_t PassthroughVideoDecoder::Release()
{
    callback_ = nullptr;
    return WEBRTC_VIDEO_CODEC_OK;
}

webrtc::VideoDecoder::DecoderInfo PassthroughVideoDecoder::GetDecoderInfo() const
{
    DecoderInfo info;
    info.implementation_name = "TelinhaPassthrough";
    info.is_hardware_accelerated = false;
    return info;
}

PassthroughVideoDecoderFactory::PassthroughVideoDecoderFactory(webrtc::SdpVideoFormat format,
                                                               WireVideoCodec codec,
                                                               RemoteVideoSink* sink)
    : format_(std::move(format)), codec_(codec), sink_(sink)
{}

std::vector<webrtc::SdpVideoFormat> PassthroughVideoDecoderFactory::GetSupportedFormats() const
{
    return {format_};
}

std::unique_ptr<webrtc::VideoDecoder> PassthroughVideoDecoderFactory::Create(
    const webrtc::Environment& env, const webrtc::SdpVideoFormat& format)
{
    (void)env;
    (void)format;
    return std::make_unique<PassthroughVideoDecoder>(codec_, sink_);
}

}  // namespace tl::transport::backend
