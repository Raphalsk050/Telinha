#pragma once

#include <cstdint>
#include <memory>

#include "telinha/capture/capture_pipeline.hpp"
#include "telinha/core/result.hpp"
#include "telinha/encode/encoded_packet.hpp"

namespace tl::encode {

enum class VideoEncoderBackend : std::uint8_t {
    Automatic = 0,
    Nvenc,
    AmdAmf,
    QuickSync,
    MediaFoundation,
    Software,
};

const char* to_string(VideoEncoderBackend backend) noexcept;

enum class RateControlMode : std::uint8_t {
    ConstantBitrate = 0,
    VariableBitrate,
    ConstantQuality,
};

const char* to_string(RateControlMode mode) noexcept;

struct VideoEncoderConfig {
    VideoCodec codec = VideoCodec::H264;
    VideoEncoderBackend backend = VideoEncoderBackend::Automatic;
    RateControlMode rate_control = RateControlMode::ConstantBitrate;

    std::uint32_t width = 0;
    std::uint32_t height = 0;
    std::uint32_t framerate_millihertz = 60000;

    std::uint32_t target_bitrate_bps = 8u * 1000u * 1000u;
    std::uint32_t max_bitrate_bps = 20u * 1000u * 1000u;
    std::uint32_t constant_quality = 23;

    std::uint32_t gop_length = 0;
    std::uint32_t max_reference_frames = 1;

    std::uint32_t coding_unit_size = 16;

    bool low_latency = true;
    bool repeat_parameter_sets = true;
    bool screen_content_tools = true;
    bool intra_refresh = false;
};

struct VideoEncoderInfo {
    VideoCodec codec = VideoCodec::Unknown;
    VideoEncoderBackend backend = VideoEncoderBackend::Automatic;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    std::uint32_t coding_unit_size = 0;
    bool accepts_gpu_surfaces = false;
    bool supports_dirty_regions = false;
    bool supports_intra_refresh = false;
};

class VideoEncoder {
public:
    virtual ~VideoEncoder() = default;

    VideoEncoder(const VideoEncoder&) = delete;
    VideoEncoder& operator=(const VideoEncoder&) = delete;

    virtual Outcome start() = 0;
    virtual void stop() noexcept = 0;

    [[nodiscard]] virtual Outcome submit(const capture::ClassifiedFrame& frame) = 0;

    [[nodiscard]] virtual Outcome poll(EncodedVideoPacket& out, std::uint32_t timeout_ms) = 0;
    virtual void release() noexcept = 0;

    virtual void request_keyframe() noexcept = 0;

    [[nodiscard]] virtual Outcome set_target_bitrate(std::uint32_t bits_per_second) noexcept = 0;

    [[nodiscard]] virtual VideoEncoderInfo info() const noexcept = 0;

protected:
    VideoEncoder() = default;
};

[[nodiscard]] bool encoder_backend_available(VideoEncoderBackend backend,
                                             VideoCodec codec) noexcept;

[[nodiscard]] Result<std::unique_ptr<VideoEncoder>> create_video_encoder(
    const VideoEncoderConfig& config, void* native_device);

}  // namespace tl::encode
