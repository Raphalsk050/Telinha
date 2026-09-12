#include "telinha/receive/audio_renderer.hpp"
#include "telinha/receive/decoded_media.hpp"
#include "telinha/receive/video_decoder.hpp"
#include "telinha/receive/video_renderer.hpp"

namespace tl::receive {

const char* to_string(FrameMemory memory) noexcept
{
    switch (memory) {
        case FrameMemory::None: return "None";
        case FrameMemory::GpuTexture: return "GpuTexture";
        case FrameMemory::CpuPlanar: return "CpuPlanar";
    }
    return "Unknown";
}

const char* to_string(VideoDecoderBackend backend) noexcept
{
    switch (backend) {
        case VideoDecoderBackend::Automatic: return "Automatic";
        case VideoDecoderBackend::D3D11VideoDevice: return "D3D11VideoDevice";
        case VideoDecoderBackend::MediaFoundation: return "MediaFoundation";
        case VideoDecoderBackend::Software: return "Software";
    }
    return "Unknown";
}

const char* to_string(RendererBackend backend) noexcept
{
    switch (backend) {
        case RendererBackend::Automatic: return "Automatic";
        case RendererBackend::Direct3D11: return "Direct3D11";
        case RendererBackend::Headless: return "Headless";
    }
    return "Unknown";
}

const char* to_string(AudioRendererBackend backend) noexcept
{
    switch (backend) {
        case AudioRendererBackend::Automatic: return "Automatic";
        case AudioRendererBackend::Wasapi: return "Wasapi";
        case AudioRendererBackend::Headless: return "Headless";
    }
    return "Unknown";
}

}  // namespace tl::receive
