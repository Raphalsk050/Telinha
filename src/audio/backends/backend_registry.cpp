#include "telinha/audio/audio_source.hpp"

namespace tl::audio {

#if !defined(TELINHA_HAS_AUDIO_BACKENDS)

bool process_loopback_available() noexcept
{
    return false;
}

Result<std::unique_ptr<AudioSource>> create_audio_source(const AudioCaptureTarget&,
                                                         const AudioCaptureOptions&)
{
    return Error{Status::NotImplemented, "create_audio_source: no backend linked"};
}

#endif

}  // namespace tl::audio
