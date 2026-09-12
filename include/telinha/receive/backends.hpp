#pragma once

#include <memory>

#include "telinha/core/result.hpp"
#include "telinha/receive/audio_renderer.hpp"
#include "telinha/receive/video_decoder.hpp"
#include "telinha/receive/video_renderer.hpp"

namespace tl::receive {

[[nodiscard]] Result<std::unique_ptr<VideoRenderer>> create_headless_video_renderer(
    const VideoRendererConfig& config);

[[nodiscard]] Result<std::unique_ptr<AudioRenderer>> create_headless_audio_renderer(
    const AudioRendererConfig& config);

#if TL_PLATFORM_WINDOWS

[[nodiscard]] Result<std::unique_ptr<VideoRenderer>> create_d3d11_video_renderer(
    const VideoRendererConfig& config);

[[nodiscard]] Result<std::unique_ptr<AudioRenderer>> create_wasapi_audio_renderer(
    const AudioRendererConfig& config);

[[nodiscard]] Result<std::unique_ptr<VideoDecoder>> create_media_foundation_decoder(
    const VideoDecoderConfig& config, void* native_device);

#endif

}  // namespace tl::receive
