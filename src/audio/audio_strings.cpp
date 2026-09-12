#include "telinha/audio/audio_source.hpp"

namespace tl::audio {

const char* to_string(SampleFormat format) noexcept
{
    switch (format) {
        case SampleFormat::Unknown: return "Unknown";
        case SampleFormat::Int16: return "Int16";
        case SampleFormat::Float32: return "Float32";
    }
    return "Unrecognized";
}

const char* to_string(AudioCaptureScope scope) noexcept
{
    switch (scope) {
        case AudioCaptureScope::None: return "None";
        case AudioCaptureScope::SystemLoopback: return "SystemLoopback";
        case AudioCaptureScope::ProcessLoopback: return "ProcessLoopback";
    }
    return "Unrecognized";
}

}  // namespace tl::audio
