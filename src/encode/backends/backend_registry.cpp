#include "telinha/encode/video_encoder.hpp"

#if TL_PLATFORM_WINDOWS
#include "mf_video_encoder.hpp"
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
        case VideoEncoderBackend::MediaFoundation: return media_foundation_available(codec, true);
        case VideoEncoderBackend::Software: return media_foundation_available(codec, false);
        case VideoEncoderBackend::AmdAmf:
        case VideoEncoderBackend::QuickSync: return false;
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

#if TL_PLATFORM_WINDOWS

namespace {

[[nodiscard]] Result<std::unique_ptr<VideoEncoder>> create_backend(VideoEncoderBackend backend,
                                                                   const VideoEncoderConfig& config,
                                                                   void* native_device)
{
    VideoEncoderConfig resolved = config;
    resolved.backend = backend;

    switch (backend) {
        case VideoEncoderBackend::Nvenc: return create_nvenc_encoder(resolved, native_device);
        case VideoEncoderBackend::MediaFoundation:
            return create_media_foundation_encoder(resolved, native_device, true);
        case VideoEncoderBackend::Software:
            return create_media_foundation_encoder(resolved, native_device, false);
        case VideoEncoderBackend::AmdAmf:
        case VideoEncoderBackend::QuickSync:
            return Error{Status::NotImplemented,
                         "create_video_encoder: this vendor backend is not built yet"};
        case VideoEncoderBackend::Automatic: break;
    }
    return Error{Status::NotSupported, "create_video_encoder: unknown backend"};
}

}  // namespace

#endif

Result<std::unique_ptr<VideoEncoder>> create_video_encoder(const VideoEncoderConfig& config,
                                                           void* native_device)
{
    if (!config.valid()) {
        return Error{Status::InvalidArgument, "create_video_encoder: the configuration is invalid"};
    }

#if TL_PLATFORM_WINDOWS
    if (config.backend != VideoEncoderBackend::Automatic) {
        return create_backend(config.backend, config, native_device);
    }

    Error last{Status::Unavailable, "create_video_encoder: no encoder backend is available"};
    for (const VideoEncoderBackend candidate : kPreferenceOrder) {
        if (!encoder_backend_available(candidate, config.codec)) {
            continue;
        }
        Result<std::unique_ptr<VideoEncoder>> created =
            create_backend(candidate, config, native_device);
        if (created.ok()) {
            return created;
        }
        last = created.error();
    }
    return last;
#else
    (void)native_device;
    return Error{Status::NotSupported, "create_video_encoder: this platform has no video encoder"};
#endif
}

}  // namespace tl::encode
