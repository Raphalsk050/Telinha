#pragma once

#include <cstdint>
#include <memory>
#include <vector>

#include "api/environment/environment.h"
#include "api/video_codecs/sdp_video_format.h"
#include "api/video_codecs/video_encoder.h"
#include "api/video_codecs/video_encoder_factory.h"
#include "telinha/transport/media_transport.hpp"

namespace tl::transport::backend {

class EncoderFeedback {
public:
    virtual ~EncoderFeedback() = default;

    virtual void on_keyframe_requested() noexcept = 0;
    virtual void on_target_bitrate(std::uint32_t bits_per_second,
                                   std::uint32_t framerate_hz) noexcept = 0;

protected:
    EncoderFeedback() = default;
};

class PassthroughVideoEncoder final : public webrtc::VideoEncoder {
public:
    PassthroughVideoEncoder(WireVideoCodec codec, EncoderFeedback& feedback);

    void SetFecControllerOverride(webrtc::FecControllerOverride* controller) override;
    int InitEncode(const webrtc::VideoCodec* codec_settings, const Settings& settings) override;
    int32_t RegisterEncodeCompleteCallback(webrtc::EncodedImageCallback* callback) override;
    int32_t Release() override;
    int32_t Encode(const webrtc::VideoFrame& frame,
                   const std::vector<webrtc::VideoFrameType>* frame_types) override;
    void SetRates(const RateControlParameters& parameters) override;
    [[nodiscard]] EncoderInfo GetEncoderInfo() const override;

private:
    EncoderFeedback* feedback_;
    webrtc::EncodedImageCallback* callback_ = nullptr;
    WireVideoCodec codec_;
    std::uint32_t target_bitrate_bps_ = 0;
    std::uint32_t framerate_hz_ = 0;
    bool awaiting_keyframe_ = true;
};

class PassthroughVideoEncoderFactory final : public webrtc::VideoEncoderFactory {
public:
    PassthroughVideoEncoderFactory(webrtc::SdpVideoFormat format, WireVideoCodec codec,
                                   EncoderFeedback& feedback);

    [[nodiscard]] std::vector<webrtc::SdpVideoFormat> GetSupportedFormats() const override;
    std::unique_ptr<webrtc::VideoEncoder> Create(const webrtc::Environment& env,
                                                 const webrtc::SdpVideoFormat& format) override;

private:
    webrtc::SdpVideoFormat format_;
    WireVideoCodec codec_;
    EncoderFeedback* feedback_;
};

[[nodiscard]] webrtc::SdpVideoFormat sdp_format_for(WireVideoCodec codec,
                                                    const char* h264_profile_level_id);

}  // namespace tl::transport::backend
