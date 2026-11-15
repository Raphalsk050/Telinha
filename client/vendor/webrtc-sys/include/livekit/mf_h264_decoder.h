// H264 decoding through the Windows Media Foundation decoder.
//
// The prebuilt libwebrtc ships without an H264 decoder, so streams sent as
// H264 could not be shown. Windows has one built in; this factory hands it to
// libwebrtc like any other decoder.

#pragma once

#ifdef _WIN32

#include <memory>
#include <vector>

#include "api/video_codecs/sdp_video_format.h"
#include "api/video_codecs/video_decoder.h"
#include "api/video_codecs/video_decoder_factory.h"

namespace livekit_ffi {

class MfH264DecoderFactory : public webrtc::VideoDecoderFactory {
 public:
  std::vector<webrtc::SdpVideoFormat> GetSupportedFormats() const override;

  std::unique_ptr<webrtc::VideoDecoder> Create(
      const webrtc::Environment& env,
      const webrtc::SdpVideoFormat& format) override;
};

}  // namespace livekit_ffi

#endif
