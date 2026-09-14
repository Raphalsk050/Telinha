#pragma once

#include <cstdint>

#include "telinha/audio/audio_source.hpp"
#include "telinha/capture/capture_target.hpp"
#include "telinha/core/log.hpp"
#include "telinha/receive/audio_renderer.hpp"
#include "telinha/receive/jitter_buffer.hpp"
#include "telinha/receive/video_decoder.hpp"
#include "telinha/receive/video_renderer.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::app {

enum class AppMode : std::uint8_t {
    None = 0,
    Usage,
    Wizard,
    List,
    Probe,
    Send,
    Receive,
};

const char* to_string(AppMode mode) noexcept;

enum class AudioScope : std::uint8_t {
    None = 0,
    System,
    Process,
    Device,
};

const char* to_string(AudioScope scope) noexcept;

inline constexpr std::uint32_t kPathCapacity = 512;
inline constexpr std::uint32_t kUrlCapacity = 192;
inline constexpr std::uint32_t kCredentialCapacity = 128;
inline constexpr std::uint32_t kMaxIceServers = 4;
inline constexpr std::uint32_t kErrorCapacity = 320;

struct IceServerConfig {
    char url[kUrlCapacity] = {};
    char username[kCredentialCapacity] = {};
    char credential[kCredentialCapacity] = {};
};

struct NetworkOptions {
    IceServerConfig ice_servers[kMaxIceServers];
    std::uint32_t ice_server_count = 0;
    transport::CandidatePolicy candidate_policy =
        transport::CandidatePolicy::HostAndServerReflexive;
    std::uint16_t local_port_min = 0;
    std::uint16_t local_port_max = 0;
    std::uint32_t start_bitrate_bps = 8u * 1000u * 1000u;
    std::uint32_t min_bitrate_bps = 500u * 1000u;
    std::uint32_t max_bitrate_bps = 40u * 1000u * 1000u;
    Nanoseconds gather_timeout_ns = 8ull * kNanosecondsPerSecond;
    Nanoseconds connect_timeout_ns = 45ull * kNanosecondsPerSecond;
    bool enable_forward_error_correction = true;
    bool enable_retransmission = true;
};

struct SignalingOptions {
    char in_path[kPathCapacity] = {};
    char out_path[kPathCapacity] = {};
    bool use_clipboard = false;
};

struct SenderOptions {
    capture::CaptureTarget target;
    std::int32_t target_index = 0;
    capture::CaptureTargetKind target_kind = capture::CaptureTargetKind::Monitor;

    transport::WireVideoCodec codec = transport::WireVideoCodec::H264;
    std::uint32_t framerate_millihertz = 60000;
    std::uint32_t tile_size = 16;
    bool include_cursor = true;
    bool intra_refresh = true;

    AudioScope audio_scope = AudioScope::System;
    std::uint32_t audio_process_id = 0;
    std::uint32_t audio_exclude_process_id = 0;
    char audio_device_id[audio::kAudioDeviceIdCapacity] = {};

    NetworkOptions network;
    SignalingOptions signaling;
    Nanoseconds stats_interval_ns = 5ull * kNanosecondsPerSecond;
    std::uint32_t capture_timeout_ms = 100;
    bool multi_peer = false;
};

struct ReceiverOptions {
    receive::VideoDecoderConfig video;
    receive::VideoRendererConfig renderer;
    receive::AudioRendererConfig audio_renderer;
    receive::JitterConfig jitter;
    receive::ClockOffsetConfig clock;
    NetworkOptions network;
    SignalingOptions signaling;
    Nanoseconds stats_interval_ns = 5ull * kNanosecondsPerSecond;
    std::uint32_t video_slot_bytes = 2u * 1024u * 1024u;
    std::uint32_t audio_slot_bytes = 16u * 1024u;
    bool render_audio = true;
};

struct AppOptions {
    AppMode mode = AppMode::None;
    LogLevel log_level = LogLevel::Info;
    bool machine_output = false;
    capture::CaptureTargetKind list_kind = capture::CaptureTargetKind::None;
    SenderOptions sender;
    ReceiverOptions receiver;
    std::int32_t title_argument = 0;
};

[[nodiscard]] Outcome parse_command_line(int argc, const char* const* argv, AppOptions& out,
                                         char* error, int error_capacity) noexcept;

void set_window_title(ReceiverOptions& options, const char* text) noexcept;

void print_usage() noexcept;

}  // namespace tl::app
