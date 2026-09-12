#pragma once

#include <memory>
#include <vector>

#include "api/environment/environment.h"
#include "api/video_codecs/sdp_video_format.h"
#include "api/video_codecs/video_decoder.h"
#include "api/video_codecs/video_decoder_factory.h"
#include "telinha/transport/media_transport.hpp"

namespace tl::transport::backend {

class RemoteVideoSink {
public:
    virtual ~RemoteVideoSink() = default;

    virtual void on_remote_video(const EncodedVideoFrame& frame) noexcept = 0;

protected:
    RemoteVideoSink() = default;
};

class PassthroughVideoDecoder final : public webrtc::VideoDecoder {
public:
    PassthroughVideoDecoder(WireVideoCodec codec, RemoteVideoSink* sink);

    bool Configure(const Settings& settings) override;
    int32_t Decode(const webrtc::EncodedImage& input_image, int64_t render_time_ms) override;
    int32_t RegisterDecodeCompleteCallback(webrtc::DecodedImageCallback* callback) override;
    int32_t Release() override;
    [[nodiscard]] DecoderInfo GetDecoderInfo() const override;

private:
    WireVideoCodec codec_;
    RemoteVideoSink* sink_;
    webrtc::DecodedImageCallback* callback_ = nullptr;
};

class PassthroughVideoDecoderFactory final : public webrtc::VideoDecoderFactory {
public:
    PassthroughVideoDecoderFactory(webrtc::SdpVideoFormat format, WireVideoCodec codec,
                                   RemoteVideoSink* sink);

    [[nodiscard]] std::vector<webrtc::SdpVideoFormat> GetSupportedFormats() const override;
    std::unique_ptr<webrtc::VideoDecoder> Create(const webrtc::Environment& env,
                                                 const webrtc::SdpVideoFormat& format) override;

private:
    webrtc::SdpVideoFormat format_;
    WireVideoCodec codec_;
    RemoteVideoSink* sink_;
};

}  // namespace tl::transport::backend
