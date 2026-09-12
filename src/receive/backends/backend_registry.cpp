#include "telinha/receive/backends.hpp"

namespace tl::receive {

bool renderer_backend_available(RendererBackend backend) noexcept
{
    switch (backend) {
        case RendererBackend::Automatic:
        case RendererBackend::Headless: return true;
        case RendererBackend::Direct3D11: return TL_PLATFORM_WINDOWS != 0;
    }
    return false;
}

Result<std::unique_ptr<VideoRenderer>> create_video_renderer(const VideoRendererConfig& config)
{
    RendererBackend backend = config.backend;
    if (backend == RendererBackend::Automatic) {
        backend =
            TL_PLATFORM_WINDOWS != 0 ? RendererBackend::Direct3D11 : RendererBackend::Headless;
    }

    switch (backend) {
        case RendererBackend::Headless: return create_headless_video_renderer(config);
        case RendererBackend::Direct3D11:
#if TL_PLATFORM_WINDOWS
            return create_d3d11_video_renderer(config);
#else
            return Error{Status::NotSupported, "create_video_renderer: Direct3D11"};
#endif
        case RendererBackend::Automatic: break;
    }
    return Error{Status::InvalidArgument, "create_video_renderer"};
}

bool audio_renderer_backend_available(AudioRendererBackend backend) noexcept
{
    switch (backend) {
        case AudioRendererBackend::Automatic:
        case AudioRendererBackend::Headless: return true;
        case AudioRendererBackend::Wasapi: return TL_PLATFORM_WINDOWS != 0;
    }
    return false;
}

Result<std::unique_ptr<AudioRenderer>> create_audio_renderer(const AudioRendererConfig& config)
{
    AudioRendererBackend backend = config.backend;
    if (backend == AudioRendererBackend::Automatic) {
        backend = TL_PLATFORM_WINDOWS != 0 ? AudioRendererBackend::Wasapi
                                           : AudioRendererBackend::Headless;
    }

    switch (backend) {
        case AudioRendererBackend::Headless: return create_headless_audio_renderer(config);
        case AudioRendererBackend::Wasapi:
#if TL_PLATFORM_WINDOWS
            return create_wasapi_audio_renderer(config);
#else
            return Error{Status::NotSupported, "create_audio_renderer: Wasapi"};
#endif
        case AudioRendererBackend::Automatic: break;
    }
    return Error{Status::InvalidArgument, "create_audio_renderer"};
}

bool decoder_backend_available(VideoDecoderBackend backend,
                               transport::WireVideoCodec codec) noexcept
{
    if (codec != transport::WireVideoCodec::H264) {
        return false;
    }
    switch (backend) {
        case VideoDecoderBackend::Automatic:
        case VideoDecoderBackend::D3D11VideoDevice:
        case VideoDecoderBackend::MediaFoundation: return TL_PLATFORM_WINDOWS != 0;
        case VideoDecoderBackend::Software: return false;
    }
    return false;
}

Result<std::unique_ptr<VideoDecoder>> create_video_decoder(const VideoDecoderConfig& config,
                                                           void* native_device)
{
#if TL_PLATFORM_WINDOWS
    VideoDecoderBackend backend = config.backend;
    if (backend == VideoDecoderBackend::Automatic) {
        backend = VideoDecoderBackend::MediaFoundation;
    }
    switch (backend) {
        case VideoDecoderBackend::D3D11VideoDevice:
        case VideoDecoderBackend::MediaFoundation:
            return create_media_foundation_decoder(config, native_device);
        case VideoDecoderBackend::Software:
            return Error{Status::NotImplemented, "create_video_decoder: Software"};
        case VideoDecoderBackend::Automatic: break;
    }
    return Error{Status::InvalidArgument, "create_video_decoder"};
#else
    (void)config;
    (void)native_device;
    return Error{Status::NotSupported, "create_video_decoder: Windows only"};
#endif
}

}  // namespace tl::receive
