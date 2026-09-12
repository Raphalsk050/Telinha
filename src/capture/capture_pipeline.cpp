#include "telinha/capture/capture_pipeline.hpp"

#include "telinha/core/log.hpp"

namespace tl::capture {
namespace {
constexpr std::size_t kPipelineArenaBytes = 1u << 20;
}

CapturePipeline::~CapturePipeline()
{
    stop();
}

Outcome CapturePipeline::initialize(std::unique_ptr<CaptureSource> source,
                                    const CaptureOptions& options)
{
    if (source == nullptr) {
        return fail(Status::InvalidArgument, "CapturePipeline::initialize: null source");
    }
    if (options.tile_size == 0 || (options.tile_size & (options.tile_size - 1)) != 0) {
        return fail(Status::InvalidArgument,
                    "CapturePipeline::initialize: tile size must be a power of two");
    }

    source_ = std::move(source);
    options_ = options;

    if (!storage_.reserve(kPipelineArenaBytes)) {
        source_.reset();
        return fail(Status::OutOfMemory, "CapturePipeline::initialize: arena reservation failed");
    }

    const CaptureSourceInfo source_info = source_->info();
    if (source_info.width == 0 || source_info.height == 0) {
        source_.reset();
        return fail(Status::Unavailable,
                    "CapturePipeline::initialize: source reported an empty surface");
    }

    if (!tiles_.initialize(storage_.arena(), source_info.width, source_info.height,
                           options_.tile_size)) {
        source_.reset();
        return fail(Status::OutOfMemory, "CapturePipeline::initialize: tile map allocation failed");
    }

    stats_.reset();
    last_present_ns_ = 0;

    TL_LOG_INFO("capture pipeline ready: %ux%u, %u tiles, backend %s", source_info.width,
                source_info.height, tiles_.tile_count(), to_string(source_info.backend));
    return ok();
}

Outcome CapturePipeline::start()
{
    if (source_ == nullptr) {
        return fail(Status::Unavailable, "CapturePipeline::start: not initialized");
    }
    if (started_) {
        return ok();
    }
    TL_TRY(source_->start());
    started_ = true;
    return ok();
}

void CapturePipeline::stop() noexcept
{
    if (source_ == nullptr) {
        return;
    }
    release();
    if (started_) {
        source_->stop();
        started_ = false;
    }
}

Outcome CapturePipeline::capture_next(ClassifiedFrame& out, std::uint32_t timeout_ms)
{
    out = ClassifiedFrame{};

    if (source_ == nullptr || !started_) {
        return fail(Status::Unavailable, "CapturePipeline::capture_next: not started");
    }

    release();

    current_frame_ = CapturedFrame{};
    CapturedFrame& frame = current_frame_;

    const Nanoseconds acquire_begin = now_ns();
    const Outcome acquired = source_->acquire(frame, timeout_ms);
    const Nanoseconds acquire_end = now_ns();

    if (!acquired.ok()) {
        if (acquired.status() == Status::Timeout) {
            ++stats_.frames_timed_out;
        }
        return acquired;
    }

    frame_held_ = true;
    stats_.acquire_ns.record(acquire_end - acquire_begin);
    ++stats_.frames_acquired;

    if (frame.metadata.accumulated_frames > 1) {
        stats_.frames_coalesced += frame.metadata.accumulated_frames - 1;
    }
    if (last_present_ns_ != 0 && frame.metadata.present_time_ns > last_present_ns_) {
        stats_.present_interval_ns.record(frame.metadata.present_time_ns - last_present_ns_);
    }
    if (frame.metadata.present_time_ns != 0) {
        last_present_ns_ = frame.metadata.present_time_ns;
    }

    if (!frame.metadata.content_changed) {
        ++stats_.frames_cursor_only;
    }

    const Nanoseconds classify_begin = now_ns();
    tiles_.clear();
    if (frame.metadata.full_surface_dirty) {
        ++stats_.frames_full_dirty;
        tiles_.mark_all();
    } else {
        tiles_.mark(frame.dirty_rects);
        for (const MoveRect& move : frame.move_rects) {
            tiles_.mark(move.destination);
        }
    }
    const std::uint32_t dirty_tiles = tiles_.dirty_tile_count();
    const Nanoseconds classify_end = now_ns();

    stats_.classify_ns.record(classify_end - classify_begin);
    stats_.dirty_rects_total += frame.dirty_rects.count();
    stats_.dirty_tiles_total += dirty_tiles;
    stats_.tiles_total += tiles_.tile_count();

    out.frame = &frame;
    out.tiles = &tiles_;
    out.dirty_tile_count = dirty_tiles;
    return ok();
}

void CapturePipeline::release() noexcept
{
    if (frame_held_ && source_ != nullptr) {
        source_->release();
        frame_held_ = false;
    }
}

CaptureSourceInfo CapturePipeline::info() const noexcept
{
    return source_ == nullptr ? CaptureSourceInfo{} : source_->info();
}
}  // namespace tl::capture
