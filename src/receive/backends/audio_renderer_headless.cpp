#include <new>

#include "telinha/core/clock.hpp"
#include "telinha/receive/backends.hpp"

namespace tl::receive {
namespace {

class HeadlessAudioRenderer final : public AudioRenderer {
public:
    explicit HeadlessAudioRenderer(const AudioRendererConfig& config) noexcept : config_(config) {}

    Outcome start() override
    {
        if (!config_.format.valid()) {
            return fail(Status::InvalidArgument, "HeadlessAudioRenderer::start");
        }
        started_ = true;
        start_ns_ = now_ns();
        submitted_frames_ = 0;
        stats_.reset();
        return ok();
    }

    void stop() noexcept override { started_ = false; }

    Outcome submit(const PcmFrame& frame) override
    {
        if (!started_) {
            return fail(Status::Unavailable, "HeadlessAudioRenderer::submit");
        }
        if (!frame.valid()) {
            return fail(Status::InvalidArgument, "HeadlessAudioRenderer::submit");
        }
        if (frame.format != config_.format) {
            ++stats_.format_rejections;
            return fail(Status::NotSupported, "HeadlessAudioRenderer::submit: format");
        }

        const Nanoseconds capacity = config_.ring_capacity_ns;
        if (buffered_ns() + config_.format.frames_to_ns(frame.frames_per_channel) > capacity) {
            ++stats_.frames_dropped;
            return fail(Status::Full, "HeadlessAudioRenderer::submit");
        }

        submitted_frames_ += frame.frames_per_channel;
        stats_.frames_submitted += frame.frames_per_channel;
        return ok();
    }

    Nanoseconds buffered_ns() const noexcept override
    {
        if (!started_) {
            return 0;
        }
        const Nanoseconds elapsed = now_ns() - start_ns_;
        const std::uint64_t consumed = config_.format.ns_to_frames(elapsed);
        return consumed >= submitted_frames_
                   ? 0
                   : config_.format.frames_to_ns(submitted_frames_ - consumed);
    }

    AudioRendererInfo info() const noexcept override
    {
        AudioRendererInfo result;
        result.backend = AudioRendererBackend::Headless;
        result.format = config_.format;
        result.device_period_ns = 10ull * kNanosecondsPerMillisecond;
        return result;
    }

    const AudioRendererStats& stats() const noexcept override { return stats_; }

private:
    AudioRendererConfig config_;
    AudioRendererStats stats_;
    Nanoseconds start_ns_ = 0;
    std::uint64_t submitted_frames_ = 0;
    bool started_ = false;
};

}  // namespace

Result<std::unique_ptr<AudioRenderer>> create_headless_audio_renderer(
    const AudioRendererConfig& config)
{
    std::unique_ptr<AudioRenderer> renderer(new (std::nothrow) HeadlessAudioRenderer(config));
    if (!renderer) {
        return Error{Status::OutOfMemory, "create_headless_audio_renderer"};
    }
    return renderer;
}

}  // namespace tl::receive
