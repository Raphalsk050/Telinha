#pragma once

#include "telinha/core/config.hpp"

#if TL_PLATFORM_WINDOWS

#include <memory>

#include "telinha/core/result.hpp"
#include "telinha/encode/video_encoder.hpp"

namespace tl::encode {

[[nodiscard]] bool media_foundation_available(VideoCodec codec, bool require_hardware) noexcept;

[[nodiscard]] Result<std::unique_ptr<VideoEncoder>> create_media_foundation_encoder(
    const VideoEncoderConfig& config, void* native_device, bool require_hardware);

}  // namespace tl::encode

#endif
