#include "module_probes.hpp"

#include <memory>

#include "telinha/capture/capture_source.hpp"
#include "telinha/encode/video_encoder.hpp"

namespace tl::diag {
namespace {
constexpr std::size_t kTargetProbeCapacity = 16;

constexpr capture::CaptureBackend kCaptureBackends[] = {
    capture::CaptureBackend::DesktopDuplication,
    capture::CaptureBackend::GraphicsCapture,
    capture::CaptureBackend::Synthetic,
};

constexpr encode::VideoEncoderBackend kEncoderBackends[] = {
    encode::VideoEncoderBackend::Nvenc,     encode::VideoEncoderBackend::AmdAmf,
    encode::VideoEncoderBackend::QuickSync, encode::VideoEncoderBackend::MediaFoundation,
    encode::VideoEncoderBackend::Software,
};

constexpr encode::VideoCodec kCodecs[] = {
    encode::VideoCodec::H264,
    encode::VideoCodec::Hevc,
    encode::VideoCodec::Av1,
};

void probe_targets(ReportBuilder& builder, capture::CaptureTargetKind kind,
                   const char* name) noexcept
{
    capture::CaptureTargetInfo targets[kTargetProbeCapacity] = {};
    std::uint32_t written = 0;
    std::uint32_t available = 0;

    const Outcome outcome = capture::enumerate_targets(
        kind, Span<capture::CaptureTargetInfo>{targets, kTargetProbeCapacity}, written, available);

    if (!outcome.ok()) {
        builder.add_failure(DiagCategory::Capture, name, outcome.error(), nullptr);
        return;
    }

    DiagItem* item = builder.add(DiagCategory::Capture, name);
    if (item == nullptr) {
        return;
    }

    item->available = available != 0;
    item->status = available != 0 ? Status::Ok : Status::NotFound;
    format_detail(item->detail, kDiagDetailCapacity, "", available);
    append_text(item->detail, kDiagDetailCapacity, " found, ");
    append_unsigned(item->detail, kDiagDetailCapacity, written);
    append_text(item->detail, kDiagDetailCapacity, " described");

    for (std::uint32_t i = 0; i < written; ++i) {
        const capture::CaptureTargetInfo& target = targets[i];
        DiagItem* entry = builder.add(DiagCategory::Capture, target.name);
        if (entry == nullptr) {
            return;
        }

        entry->available = true;
        entry->status = Status::Ok;
        write_text(entry->detail, kDiagDetailCapacity, "");
        append_unsigned(entry->detail, kDiagDetailCapacity, target.width);
        append_text(entry->detail, kDiagDetailCapacity, "x");
        append_unsigned(entry->detail, kDiagDetailCapacity, target.height);
        append_text(entry->detail, kDiagDetailCapacity, " at ");
        append_unsigned(entry->detail, kDiagDetailCapacity, target.refresh_millihertz / 1000u);
        append_text(entry->detail, kDiagDetailCapacity, " Hz, rotation ");
        append_text(entry->detail, kDiagDetailCapacity, capture::to_string(target.rotation));
        if (target.primary) {
            append_text(entry->detail, kDiagDetailCapacity, ", primary");
        }
        if (target.process_id != 0) {
            append_text(entry->detail, kDiagDetailCapacity, ", pid ");
            append_unsigned(entry->detail, kDiagDetailCapacity, target.process_id);
        }
    }
}

[[nodiscard]] capture::CaptureTarget first_target_for(capture::CaptureBackend backend) noexcept
{
    if (backend == capture::CaptureBackend::Synthetic) {
        return capture::CaptureTarget::monitor(1);
    }

    const capture::CaptureTargetKind kind = backend == capture::CaptureBackend::GraphicsCapture
                                                ? capture::CaptureTargetKind::Window
                                                : capture::CaptureTargetKind::Monitor;

    capture::CaptureTargetInfo targets[kTargetProbeCapacity] = {};
    std::uint32_t written = 0;
    std::uint32_t available = 0;
    const Outcome outcome = capture::enumerate_targets(
        kind, Span<capture::CaptureTargetInfo>{targets, kTargetProbeCapacity}, written, available);

    if (!outcome.ok() || written == 0) {
        return capture::CaptureTarget{};
    }

    return targets[0].target;
}

void probe_capture_backend(ReportBuilder& builder, capture::CaptureBackend backend) noexcept
{
    const char* name = capture::to_string(backend);

    if (!capture::backend_available(backend)) {
        builder.add_failure(DiagCategory::Capture, name,
                            Error{Status::Unavailable, "backend not compiled in or not supported"},
                            nullptr);
        return;
    }

    const capture::CaptureTarget target = first_target_for(backend);
    if (!target.valid()) {
        builder.add_failure(DiagCategory::Capture, name,
                            Error{Status::NotFound, "no target available to instantiate against"},
                            nullptr);
        return;
    }

    capture::CaptureOptions options{};
    options.backend = backend;

    Result<std::unique_ptr<capture::CaptureSource>> source =
        capture::create_capture_source(target, options);
    if (!source.ok()) {
        builder.add_failure(DiagCategory::Capture, name, source.error(), nullptr);
        return;
    }

    const capture::CaptureSourceInfo info = source.value()->info();

    DiagItem* item = builder.add(DiagCategory::Capture, name);
    if (item == nullptr) {
        return;
    }

    item->available = true;
    item->status = Status::Ok;
    write_text(item->detail, kDiagDetailCapacity, "instantiated at ");
    append_unsigned(item->detail, kDiagDetailCapacity, info.width);
    append_text(item->detail, kDiagDetailCapacity, "x");
    append_unsigned(item->detail, kDiagDetailCapacity, info.height);
    append_text(item->detail, kDiagDetailCapacity, ", ");
    append_text(item->detail, kDiagDetailCapacity, capture::to_string(info.format));
}

void probe_encoder(ReportBuilder& builder, encode::VideoEncoderBackend backend,
                   encode::VideoCodec codec) noexcept
{
    char name[kDiagNameCapacity] = {};
    write_text(name, kDiagNameCapacity, encode::to_string(backend));
    append_text(name, kDiagNameCapacity, " ");
    append_text(name, kDiagNameCapacity, encode::to_string(codec));

    encode::VideoEncoderConfig config{};
    config.codec = codec;
    config.backend = backend;
    config.width = 1920;
    config.height = 1080;
    config.coding_unit_size = encode::coding_unit_size_for(codec);

    Result<std::unique_ptr<encode::VideoEncoder>> encoder =
        encode::create_video_encoder(config, nullptr);

    if (!encoder.ok()) {
        builder.add_failure(DiagCategory::Encode, name, encoder.error(), nullptr);
        return;
    }

    const encode::VideoEncoderInfo info = encoder.value()->info();

    DiagItem* item = builder.add(DiagCategory::Encode, name);
    if (item == nullptr) {
        return;
    }

    item->available = true;
    item->status = Status::Ok;
    write_text(item->detail, kDiagDetailCapacity, "coding unit ");
    append_unsigned(item->detail, kDiagDetailCapacity, info.coding_unit_size);
    if (info.accepts_gpu_surfaces) {
        append_text(item->detail, kDiagDetailCapacity, ", gpu surfaces");
    }
    if (info.supports_dirty_regions) {
        append_text(item->detail, kDiagDetailCapacity, ", dirty regions");
    }
    if (info.supports_intra_refresh) {
        append_text(item->detail, kDiagDetailCapacity, ", intra refresh");
    }
}
}  // namespace

void probe_capture(ReportBuilder& builder) noexcept
{
    probe_targets(builder, capture::CaptureTargetKind::Monitor, "monitors");
    probe_targets(builder, capture::CaptureTargetKind::Window, "windows");

    for (const capture::CaptureBackend backend : kCaptureBackends) {
        probe_capture_backend(builder, backend);
    }
}

void probe_encode(ReportBuilder& builder) noexcept
{
    for (const encode::VideoEncoderBackend backend : kEncoderBackends) {
        for (const encode::VideoCodec codec : kCodecs) {
            probe_encoder(builder, backend, codec);
        }
    }
}
}  // namespace tl::diag
