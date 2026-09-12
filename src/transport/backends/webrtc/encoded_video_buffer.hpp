#pragma once

#include <cstdint>

#include "api/scoped_refptr.h"
#include "api/video/encoded_image.h"
#include "api/video/video_frame_buffer.h"
#include "telinha/core/clock.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::transport::backend {

class EncodedVideoBuffer : public webrtc::VideoFrameBuffer {
public:
    explicit EncodedVideoBuffer(const EncodedVideoFrame& frame);

    [[nodiscard]] Type type() const override { return Type::kNative; }
    [[nodiscard]] int width() const override { return width_; }
    [[nodiscard]] int height() const override { return height_; }
    webrtc::scoped_refptr<webrtc::I420BufferInterface> ToI420() override { return nullptr; }

    [[nodiscard]] const webrtc::scoped_refptr<webrtc::EncodedImageBufferInterface>& bitstream()
        const noexcept
    {
        return bitstream_;
    }

    [[nodiscard]] WireVideoCodec codec() const noexcept { return codec_; }
    [[nodiscard]] bool keyframe() const noexcept { return kind_ == WireFrameKind::Key; }
    [[nodiscard]] std::uint64_t frame_index() const noexcept { return frame_index_; }
    [[nodiscard]] std::int32_t average_qp() const noexcept { return average_qp_; }
    [[nodiscard]] std::uint8_t temporal_index() const noexcept { return temporal_index_; }
    [[nodiscard]] std::uint8_t spatial_index() const noexcept { return spatial_index_; }
    [[nodiscard]] Nanoseconds capture_time_ns() const noexcept { return capture_time_ns_; }

private:
    webrtc::scoped_refptr<webrtc::EncodedImageBufferInterface> bitstream_;
    Nanoseconds capture_time_ns_;
    std::uint64_t frame_index_;
    int width_;
    int height_;
    std::int32_t average_qp_;
    WireVideoCodec codec_;
    WireFrameKind kind_;
    std::uint8_t temporal_index_;
    std::uint8_t spatial_index_;
};

}  // namespace tl::transport::backend
