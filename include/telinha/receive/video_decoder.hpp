#pragma once

#include <cstdint>
#include <memory>

#include "telinha/core/result.hpp"
#include "telinha/receive/decoded_media.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::receive {

enum class VideoDecoderBackend : std::uint8_t {
    Automatic = 0,
    D3D11VideoDevice,
    MediaFoundation,
    Software,
};

const char* to_string(VideoDecoderBackend backend) noexcept;

struct VideoDecoderConfig {
    transport::WireVideoCodec codec = transport::WireVideoCodec::H264;
    VideoDecoderBackend backend = VideoDecoderBackend::Automatic;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    std::uint32_t output_frame_count = 4;
    bool low_latency = true;
    bool allow_software_fallback = true;
};

struct VideoDecoderInfo {
    transport::WireVideoCodec codec = transport::WireVideoCodec::Unknown;
    VideoDecoderBackend backend = VideoDecoderBackend::Automatic;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    capture::PixelFormat output_format = capture::PixelFormat::Unknown;
    void* native_device = nullptr;
    bool outputs_gpu_surfaces = false;
};

class VideoDecoder {
public:
    virtual ~VideoDecoder() = default;

    VideoDecoder(const VideoDecoder&) = delete;
    VideoDecoder& operator=(const VideoDecoder&) = delete;

    virtual Outcome start() = 0;
    virtual void stop() noexcept = 0;

    [[nodiscard]] virtual Outcome submit(const transport::EncodedVideoFrame& frame) = 0;

    [[nodiscard]] virtual Outcome poll(DecodedVideoFrame& out, std::uint32_t timeout_ms) = 0;
    virtual void release() noexcept = 0;

    virtual void flush() noexcept = 0;

    [[nodiscard]] virtual VideoDecoderInfo info() const noexcept = 0;

protected:
    VideoDecoder() = default;
};

[[nodiscard]] bool decoder_backend_available(VideoDecoderBackend backend,
                                             transport::WireVideoCodec codec) noexcept;

[[nodiscard]] Result<std::unique_ptr<VideoDecoder>> create_video_decoder(
    const VideoDecoderConfig& config, void* native_device);

}  // namespace tl::receive
