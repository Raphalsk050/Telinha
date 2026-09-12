#include "telinha/encode/video_encoder.hpp"

namespace tl::encode {

const char* to_string(VideoCodec codec) noexcept
{
    switch (codec) {
        case VideoCodec::H264: return "h264";
        case VideoCodec::Hevc: return "hevc";
        case VideoCodec::Av1: return "av1";
        case VideoCodec::Unknown: break;
    }
    return "unknown";
}

transport::WireVideoCodec to_wire_codec(VideoCodec codec) noexcept
{
    switch (codec) {
        case VideoCodec::H264: return transport::WireVideoCodec::H264;
        case VideoCodec::Av1: return transport::WireVideoCodec::Av1;
        case VideoCodec::Hevc:
        case VideoCodec::Unknown: break;
    }
    return transport::WireVideoCodec::Unknown;
}

const char* to_string(VideoEncoderBackend backend) noexcept
{
    switch (backend) {
        case VideoEncoderBackend::Automatic: return "automatic";
        case VideoEncoderBackend::Nvenc: return "nvenc";
        case VideoEncoderBackend::AmdAmf: return "amf";
        case VideoEncoderBackend::QuickSync: return "quicksync";
        case VideoEncoderBackend::MediaFoundation: return "media-foundation";
        case VideoEncoderBackend::Software: return "software";
    }
    return "unknown";
}

const char* to_string(RateControlMode mode) noexcept
{
    switch (mode) {
        case RateControlMode::ConstantBitrate: return "constant-bitrate";
        case RateControlMode::VariableBitrate: return "variable-bitrate";
        case RateControlMode::ConstantQuality: return "constant-quality";
    }
    return "unknown";
}

}  // namespace tl::encode
