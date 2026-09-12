#pragma once

#include <cstdint>
#include <memory>

#include "telinha/audio/audio_format.hpp"
#include "telinha/audio/captured_audio.hpp"
#include "telinha/core/result.hpp"

namespace tl::audio {

enum class AudioCaptureScope : std::uint8_t {
    None = 0,
    SystemLoopback,
    ProcessLoopback,
};

const char* to_string(AudioCaptureScope scope) noexcept;

struct AudioCaptureTarget {
    AudioCaptureScope scope = AudioCaptureScope::None;
    std::uint32_t process_id = 0;
    bool include_process_tree = true;

    [[nodiscard]] constexpr bool valid() const noexcept
    {
        return scope == AudioCaptureScope::SystemLoopback ||
               (scope == AudioCaptureScope::ProcessLoopback && process_id != 0);
    }

    [[nodiscard]] static constexpr AudioCaptureTarget system_loopback() noexcept
    {
        return AudioCaptureTarget{AudioCaptureScope::SystemLoopback, 0, false};
    }

    [[nodiscard]] static constexpr AudioCaptureTarget process_loopback(
        std::uint32_t pid, bool include_tree = true) noexcept
    {
        return AudioCaptureTarget{AudioCaptureScope::ProcessLoopback, pid, include_tree};
    }
};

struct AudioCaptureOptions {
    AudioFormat requested_format{48000, 2, SampleFormat::Float32};
    std::uint32_t buffer_duration_us = 10000;
    bool emit_silence_when_idle = true;
};

struct AudioSourceInfo {
    AudioCaptureTarget target;
    AudioFormat format;
    std::uint32_t buffer_duration_us = 0;
    bool process_loopback_supported = false;
};

class AudioSource {
public:
    virtual ~AudioSource() = default;

    AudioSource(const AudioSource&) = delete;
    AudioSource& operator=(const AudioSource&) = delete;

    virtual Outcome start() = 0;
    virtual void stop() noexcept = 0;

    [[nodiscard]] virtual Outcome acquire(CapturedAudio& out, std::uint32_t timeout_ms) = 0;
    virtual void release() noexcept = 0;

    [[nodiscard]] virtual AudioSourceInfo info() const noexcept = 0;

protected:
    AudioSource() = default;
};

class AudioLease {
public:
    explicit AudioLease(AudioSource& source) noexcept : source_(&source) {}

    AudioLease(const AudioLease&) = delete;
    AudioLease& operator=(const AudioLease&) = delete;

    ~AudioLease()
    {
        if (held_) {
            source_->release();
        }
    }

    [[nodiscard]] Outcome acquire(CapturedAudio& out, std::uint32_t timeout_ms)
    {
        if (held_) {
            source_->release();
            held_ = false;
        }
        Outcome result = source_->acquire(out, timeout_ms);
        held_ = result.ok();
        return result;
    }

    [[nodiscard]] bool held() const noexcept { return held_; }

private:
    AudioSource* source_;
    bool held_ = false;
};

[[nodiscard]] bool process_loopback_available() noexcept;

[[nodiscard]] Result<std::unique_ptr<AudioSource>> create_audio_source(
    const AudioCaptureTarget& target, const AudioCaptureOptions& options);

}  // namespace tl::audio
