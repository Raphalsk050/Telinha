#include "telinha/encode/video_encoder.hpp"

#if TL_PLATFORM_WINDOWS
#include "nvenc_encoder.hpp"
#endif

namespace tl::encode {
namespace {

constexpr VideoEncoderBackend kPreferenceOrder[] = {
    VideoEncoderBackend::Nvenc,     VideoEncoderBackend::AmdAmf,
    VideoEncoderBackend::QuickSync, VideoEncoderBackend::MediaFoundation,
    VideoEncoderBackend::Software,
};

}  // namespace

bool encoder_backend_available(VideoEncoderBackend backend, VideoCodec codec) noexcept
{
    if (codec == VideoCodec::Unknown) {
        return false;
    }

#if TL_PLATFORM_WINDOWS
    switch (backend) {
        case VideoEncoderBackend::Nvenc: return nvenc_available(codec);
        case VideoEncoderBackend::AmdAmf:
        case VideoEncoderBackend::QuickSync:
        case VideoEncoderBackend::MediaFoundation:
        case VideoEncoderBackend::Software: return false;
        case VideoEncoderBackend::Automatic: break;
    }

    for (const VideoEncoderBackend candidate : kPreferenceOrder) {
        if (encoder_backend_available(candidate, codec)) {
            return true;
        }
    }
    return false;
#else
    (void)backend;
    return false;
#endif
}

VideoEncoderBackend select_video_encoder_backend(VideoCodec codec) noexcept
{
    for (const VideoEncoderBackend candidate : kPreferenceOrder) {
        if (encoder_backend_available(candidate, codec)) {
            return candidate;
        }
    }
    return VideoEncoderBackend::Automatic;
}

Result<std::unique_ptr<VideoEncoder>> create_video_encoder(const VideoEncoderConfig& config,
                                                           void* native_device)
{
    if (!config.valid()) {
        return Error{Status::InvalidArgument, "create_video_encoder: the configuration is invalid"};
    }

    VideoEncoderConfig resolved = config;
    if (resolved.backend == VideoEncoderBackend::Automatic) {
        resolved.backend = select_video_encoder_backend(resolved.codec);
    }

#if TL_PLATFORM_WINDOWS
    switch (resolved.backend) {
        case VideoEncoderBackend::Nvenc: return create_nvenc_encoder(resolved, native_device);
        case VideoEncoderBackend::AmdAmf:
        case VideoEncoderBackend::QuickSync:
        case VideoEncoderBackend::MediaFoundation:
        case VideoEncoderBackend::Software:
        case VideoEncoderBackend::Automatic: break;
    }
    return Error{Status::Unavailable,
                 "create_video_encoder: no encoder backend accepted this configuration"};
#else
    (void)native_device;
    return Error{Status::NotSupported, "create_video_encoder: this platform has no video encoder"};
#endif
}

}  // namespace tl::encode
