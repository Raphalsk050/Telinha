#pragma once

#include "api/video/video_broadcaster.h"
#include "api/video/video_frame.h"
#include "api/video/video_source_interface.h"
#include "pc/video_track_source.h"

namespace tl::transport::backend {

class PushVideoSource : public webrtc::VideoTrackSource {
public:
    PushVideoSource() : webrtc::VideoTrackSource(false) {}

    [[nodiscard]] bool is_screencast() const override { return true; }

    void push(const webrtc::VideoFrame& frame) { broadcaster_.OnFrame(frame); }

    [[nodiscard]] bool has_sinks() const { return broadcaster_.frame_wanted(); }

protected:
    webrtc::VideoSourceInterface<webrtc::VideoFrame>* source() override { return &broadcaster_; }

private:
    webrtc::VideoBroadcaster broadcaster_;
};

}  // namespace tl::transport::backend
