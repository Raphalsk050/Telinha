#include <new>

#include "telinha/core/clock.hpp"
#include "telinha/receive/backends.hpp"

namespace tl::receive {
namespace {

class HeadlessVideoRenderer final : public VideoRenderer {
public:
    explicit HeadlessVideoRenderer(const VideoRendererConfig& config) noexcept : config_(config) {}

    Outcome start() override
    {
        started_ = true;
        last_present_ns_ = 0;
        stats_.reset();
        return ok();
    }

    void stop() noexcept override { started_ = false; }

    Outcome pump(bool& close_requested) noexcept override
    {
        close_requested = false;
        return started_ ? ok() : fail(Status::Unavailable, "HeadlessVideoRenderer::pump");
    }

    Outcome present(const DecodedVideoFrame& frame) override
    {
        if (!started_) {
            return fail(Status::Unavailable, "HeadlessVideoRenderer::present");
        }
        if (!frame.valid()) {
            return fail(Status::InvalidArgument, "HeadlessVideoRenderer::present");
        }

        const Nanoseconds now = now_ns();
        if (last_present_ns_ != 0) {
            stats_.present_interval_ns.record(now - last_present_ns_);
        }
        last_present_ns_ = now;
        stats_.present_ns.record(0);
        ++stats_.frames_presented;
        width_ = frame.width;
        height_ = frame.height;
        return ok();
    }

    VideoRendererInfo info() const noexcept override
    {
        VideoRendererInfo result;
        result.backend = RendererBackend::Headless;
        result.width = width_ != 0 ? width_ : config_.width;
        result.height = height_ != 0 ? height_ : config_.height;
        result.native_device = nullptr;
        result.accepts_gpu_surfaces = false;
        return result;
    }

    const RendererStats& stats() const noexcept override { return stats_; }

private:
    VideoRendererConfig config_;
    RendererStats stats_;
    Nanoseconds last_present_ns_ = 0;
    std::uint32_t width_ = 0;
    std::uint32_t height_ = 0;
    bool started_ = false;
};

}  // namespace

Result<std::unique_ptr<VideoRenderer>> create_headless_video_renderer(
    const VideoRendererConfig& config)
{
    std::unique_ptr<VideoRenderer> renderer(new (std::nothrow) HeadlessVideoRenderer(config));
    if (!renderer) {
        return Error{Status::OutOfMemory, "create_headless_video_renderer"};
    }
    return renderer;
}

}  // namespace tl::receive
