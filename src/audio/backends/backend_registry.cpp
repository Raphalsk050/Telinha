#include "telinha/audio/audio_source.hpp"

namespace tl::audio {

#if !defined(TELINHA_HAS_AUDIO_BACKENDS)

bool process_loopback_available() noexcept
{
    return false;
}

Outcome enumerate_capture_endpoints(Span<AudioEndpointInfo>, std::uint32_t& written,
                                    std::uint32_t& available) noexcept
{
    written = 0;
    available = 0;
    return fail(Status::NotImplemented, "enumerate_capture_endpoints: no backend linked");
}

Result<std::unique_ptr<AudioSource>> create_audio_source(const AudioCaptureTarget&,
                                                         const AudioCaptureOptions&)
{
    return Error{Status::NotImplemented, "create_audio_source: no backend linked"};
}

#endif

}  // namespace tl::audio
