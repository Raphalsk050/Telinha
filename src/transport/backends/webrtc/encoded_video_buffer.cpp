#include "backends/webrtc/encoded_video_buffer.hpp"

#include <cstddef>

namespace tl::transport::backend {

EncodedVideoBuffer::EncodedVideoBuffer(const EncodedVideoFrame& frame)
    : bitstream_(webrtc::EncodedImageBuffer::Create(
          reinterpret_cast<const std::uint8_t*>(frame.bitstream.data()), frame.bitstream.size())),
      capture_time_ns_(frame.capture_time_ns),
      frame_index_(frame.frame_index),
      width_(static_cast<int>(frame.width)),
      height_(static_cast<int>(frame.height)),
      average_qp_(frame.average_qp),
      codec_(frame.codec),
      kind_(frame.kind),
      temporal_index_(frame.temporal_index),
      spatial_index_(frame.spatial_index)
{}

}  // namespace tl::transport::backend
