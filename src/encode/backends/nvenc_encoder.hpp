#pragma once

#include "telinha/core/config.hpp"

#if TL_PLATFORM_WINDOWS

#include <memory>

#include "telinha/core/result.hpp"
#include "telinha/encode/video_encoder.hpp"

namespace tl::encode {

[[nodiscard]] bool nvenc_available(VideoCodec codec) noexcept;

[[nodiscard]] Result<std::unique_ptr<VideoEncoder>> create_nvenc_encoder(
    const VideoEncoderConfig& config, void* native_device);

}  // namespace tl::encode

#endif
