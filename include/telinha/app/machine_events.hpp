#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <mutex>

#include "telinha/core/span.hpp"

namespace tl::app {

void enable_machine_events() noexcept;
[[nodiscard]] bool machine_events_enabled() noexcept;

class MachineEvent {
public:
    explicit MachineEvent(const char* name) noexcept;
    ~MachineEvent();

    MachineEvent(const MachineEvent&) = delete;
    MachineEvent& operator=(const MachineEvent&) = delete;

    MachineEvent& text(const char* key, const char* value) noexcept;
    MachineEvent& text(const char* key, const char* value, std::size_t length) noexcept;
    MachineEvent& integer(const char* key, std::uint64_t value) noexcept;
    MachineEvent& number(const char* key, double value) noexcept;
    MachineEvent& flag(const char* key, bool value) noexcept;

    MachineEvent& begin_array(const char* key) noexcept;
    MachineEvent& end_array() noexcept;
    MachineEvent& begin_object() noexcept;
    MachineEvent& end_object() noexcept;

private:
    static constexpr int kMaxDepth = 8;

    void separate() noexcept;
    void write_key(const char* key) noexcept;

    std::unique_lock<std::mutex> lock_;
    int depth_ = 0;
    bool needs_comma_[kMaxDepth] = {};
};

inline constexpr std::size_t kPeerIdCapacity = 65;
inline constexpr std::size_t kDeviceIdCapacity = 128;

enum class MachineCommandKind : std::uint8_t {
    None = 0,
    Stop,
    SwitchTarget,
    SetQuality,
    SetFullscreen,
    SetAudio,
    AddPeer,
    PeerAnswer,
    RemovePeer,
    SetVolume,
    PeerSdpAnswer,
};

enum class PeerFormat : std::uint8_t {
    Token = 0,
    Sdp,
};

struct MachineCommand {
    MachineCommandKind kind = MachineCommandKind::None;
    PeerFormat format = PeerFormat::Token;
    bool window = false;
    bool device = false;
    bool enabled = false;
    std::uint64_t handle = 0;
    std::uint32_t max_fps = 0;
    std::uint32_t max_width = 0;
    std::uint32_t max_height = 0;
    std::uint32_t max_bitrate_kbps = 0;
    std::uint32_t pid = 0;
    std::uint32_t volume = 0;
    char scope[16] = {};
    char device_id[kDeviceIdCapacity] = {};
    char peer[kPeerIdCapacity] = {};
    // The decoded code or SDP. After take_machine_command it stays valid until the next take.
    const char* payload = nullptr;
    std::size_t payload_length = 0;
};

[[nodiscard]] bool parse_machine_command(const char* line, MachineCommand& out,
                                         Span<char> scratch) noexcept;

void start_stop_watcher(std::atomic<bool>& stop);
void start_command_reader(std::atomic<bool>& stop);
[[nodiscard]] bool take_machine_command(MachineCommand& out) noexcept;

}  // namespace tl::app
