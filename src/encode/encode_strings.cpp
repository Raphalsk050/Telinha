#include "telinha/encode/video_encoder.hpp"

namespace tl::encode {

const char* to_string(VideoCodec codec) noexcept
{
    switch (codec) {
        case VideoCodec::Unknown: return "Unknown";
        case VideoCodec::H264: return "H264";
        case VideoCodec::Hevc: return "Hevc";
        case VideoCodec::Av1: return "Av1";
    }
    return "Unrecognized";
}

const char* to_string(AudioCodec codec) noexcept
{
    switch (codec) {
        case AudioCodec::Unknown: return "Unknown";
        case AudioCodec::Opus: return "Opus";
    }
    return "Unrecognized";
}

const char* to_string(VideoEncoderBackend backend) noexcept
{
    switch (backend) {
        case VideoEncoderBackend::Automatic: return "Automatic";
        case VideoEncoderBackend::Nvenc: return "Nvenc";
        case VideoEncoderBackend::AmdAmf: return "AmdAmf";
        case VideoEncoderBackend::QuickSync: return "QuickSync";
        case VideoEncoderBackend::MediaFoundation: return "MediaFoundation";
        case VideoEncoderBackend::Software: return "Software";
    }
    return "Unrecognized";
}

const char* to_string(RateControlMode mode) noexcept
{
    switch (mode) {
        case RateControlMode::ConstantBitrate: return "ConstantBitrate";
        case RateControlMode::VariableBitrate: return "VariableBitrate";
        case RateControlMode::ConstantQuality: return "ConstantQuality";
    }
    return "Unrecognized";
}

}  // namespace tl::encode
