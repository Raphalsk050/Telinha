#include "telinha/encode/audio_encoder.hpp"
#include "telinha/encode/video_encoder.hpp"

namespace tl::encode {

#if !defined(TELINHA_HAS_ENCODE_BACKENDS)

bool encoder_backend_available(VideoEncoderBackend, VideoCodec) noexcept
{
    return false;
}

Result<std::unique_ptr<VideoEncoder>> create_video_encoder(const VideoEncoderConfig&, void*)
{
    return Error{Status::NotImplemented, "create_video_encoder: no backend linked"};
}

Result<std::unique_ptr<AudioEncoder>> create_audio_encoder(const AudioEncoderConfig&)
{
    return Error{Status::NotImplemented, "create_audio_encoder: no backend linked"};
}

#endif

}  // namespace tl::encode
