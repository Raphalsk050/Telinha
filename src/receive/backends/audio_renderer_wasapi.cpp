#include <audioclient.h>
#include <avrt.h>
#include <mmdeviceapi.h>
#include <windows.h>
#include <wrl/client.h>

#include <atomic>
#include <cstring>
#include <new>
#include <thread>

#include "telinha/core/log.hpp"
#include "telinha/receive/backends.hpp"

namespace tl::receive {
namespace {

using Microsoft::WRL::ComPtr;

constexpr Nanoseconds kHundredNanoseconds = 100;
constexpr DWORD kRenderWaitMs = 200;

void apply_gain(std::int16_t* samples, std::size_t count, std::uint32_t percent) noexcept
{
    if (percent == kDefaultVolumePercent) {
        return;
    }
    const auto gain = static_cast<std::int32_t>(percent);
    for (std::size_t index = 0; index < count; ++index) {
        const std::int32_t scaled = static_cast<std::int32_t>(samples[index]) * gain / 100;
        samples[index] =
            static_cast<std::int16_t>(scaled > 32767 ? 32767 : (scaled < -32768 ? -32768 : scaled));
    }
}

class SampleRing {
public:
    ~SampleRing() { release(); }

    [[nodiscard]] Outcome reserve(std::size_t samples)
    {
        release();
        std::size_t rounded = 1;
        while (rounded < samples) {
            rounded <<= 1;
        }
        storage_ = new (std::nothrow) std::int16_t[rounded];
        if (storage_ == nullptr) {
            return fail(Status::OutOfMemory, "SampleRing::reserve");
        }
        capacity_ = rounded;
        mask_ = rounded - 1;
        write_.store(0, std::memory_order_relaxed);
        read_.store(0, std::memory_order_relaxed);
        return ok();
    }

    [[nodiscard]] std::size_t write(const std::int16_t* source, std::size_t count) noexcept
    {
        const std::size_t head = write_.load(std::memory_order_relaxed);
        const std::size_t tail = read_.load(std::memory_order_acquire);
        const std::size_t free_space = capacity_ - (head - tail);
        const std::size_t writable = count < free_space ? count : free_space;

        for (std::size_t index = 0; index < writable; ++index) {
            storage_[(head + index) & mask_] = source[index];
        }
        write_.store(head + writable, std::memory_order_release);
        return writable;
    }

    [[nodiscard]] std::size_t read(std::int16_t* destination, std::size_t count) noexcept
    {
        const std::size_t tail = read_.load(std::memory_order_relaxed);
        const std::size_t head = write_.load(std::memory_order_acquire);
        const std::size_t available = head - tail;
        const std::size_t readable = count < available ? count : available;

        for (std::size_t index = 0; index < readable; ++index) {
            destination[index] = storage_[(tail + index) & mask_];
        }
        read_.store(tail + readable, std::memory_order_release);
        return readable;
    }

    [[nodiscard]] std::size_t available() const noexcept
    {
        return write_.load(std::memory_order_acquire) - read_.load(std::memory_order_acquire);
    }

private:
    void release() noexcept
    {
        delete[] storage_;
        storage_ = nullptr;
        capacity_ = 0;
        mask_ = 0;
    }

    std::int16_t* storage_ = nullptr;
    std::size_t capacity_ = 0;
    std::size_t mask_ = 0;
    alignas(kCacheLineSize) std::atomic<std::size_t> write_{0};
    alignas(kCacheLineSize) std::atomic<std::size_t> read_{0};
};

class WasapiAudioRenderer final : public AudioRenderer {
public:
    explicit WasapiAudioRenderer(const AudioRendererConfig& config) noexcept : config_(config) {}

    ~WasapiAudioRenderer() override { stop(); }

    Outcome start() override
    {
        if (!config_.format.valid()) {
            return fail(Status::InvalidArgument, "WasapiAudioRenderer::start");
        }

        const std::size_t ring_samples =
            static_cast<std::size_t>(config_.format.ns_to_frames(config_.ring_capacity_ns)) *
            config_.format.channels;
        TL_TRY(ring_.reserve(ring_samples == 0 ? 4096 : ring_samples));

        TL_TRY(open_device());

        event_ = CreateEventW(nullptr, FALSE, FALSE, nullptr);
        if (event_ == nullptr) {
            return fail(Status::PlatformError, "WasapiAudioRenderer::start: CreateEventW",
                        static_cast<std::int32_t>(GetLastError()));
        }

        HRESULT result = client_->SetEventHandle(event_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "WasapiAudioRenderer::start: SetEventHandle",
                        result);
        }

        result = client_->GetService(__uuidof(IAudioRenderClient), &render_client_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "WasapiAudioRenderer::start: IAudioRenderClient",
                        result);
        }

        result = client_->Start();
        if (FAILED(result)) {
            return fail(Status::PlatformError, "WasapiAudioRenderer::start: Start", result);
        }

        stats_.reset();
        running_.store(true, std::memory_order_relaxed);
        thread_ = std::thread(&WasapiAudioRenderer::render_thread_main, this);
        return ok();
    }

    void stop() noexcept override
    {
        running_.store(false, std::memory_order_relaxed);
        if (event_ != nullptr) {
            SetEvent(event_);
        }
        if (thread_.joinable()) {
            thread_.join();
        }
        if (client_) {
            client_->Stop();
        }
        render_client_.Reset();
        client_.Reset();
        device_.Reset();
        if (event_ != nullptr) {
            CloseHandle(event_);
            event_ = nullptr;
        }
        if (com_initialized_) {
            CoUninitialize();
            com_initialized_ = false;
        }
    }

    Outcome submit(const PcmFrame& frame) override
    {
        if (!running_.load(std::memory_order_relaxed)) {
            return fail(Status::Unavailable, "WasapiAudioRenderer::submit");
        }
        if (!frame.valid()) {
            return fail(Status::InvalidArgument, "WasapiAudioRenderer::submit");
        }
        if (frame.format != config_.format) {
            ++stats_.format_rejections;
            return fail(Status::NotSupported, "WasapiAudioRenderer::submit: formato");
        }

        const std::size_t samples =
            static_cast<std::size_t>(frame.frames_per_channel) * frame.format.channels;
        const std::size_t written = ring_.write(frame.interleaved.data(), samples);

        stats_.frames_submitted += written / frame.format.channels;
        if (written < samples) {
            stats_.frames_dropped += (samples - written) / frame.format.channels;
            return fail(Status::Full, "WasapiAudioRenderer::submit");
        }
        return ok();
    }

    Nanoseconds buffered_ns() const noexcept override
    {
        const std::size_t queued = ring_.available() / config_.format.channels;
        std::uint32_t padding = 0;
        if (client_) {
            UINT32 device_padding = 0;
            if (SUCCEEDED(client_->GetCurrentPadding(&device_padding))) {
                padding = device_padding;
            }
        }
        return config_.format.frames_to_ns(queued + padding);
    }

    AudioRendererInfo info() const noexcept override
    {
        AudioRendererInfo result;
        result.backend = AudioRendererBackend::Wasapi;
        result.format = config_.format;
        result.device_period_ns = device_period_ns_;
        return result;
    }

    const AudioRendererStats& stats() const noexcept override { return stats_; }

    void set_volume(std::uint32_t percent) noexcept override
    {
        volume_percent_.store(percent > kMaxVolumePercent ? kMaxVolumePercent : percent,
                              std::memory_order_relaxed);
    }

private:
    Outcome open_device()
    {
        const HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
        if (SUCCEEDED(initialized)) {
            com_initialized_ = true;
        } else if (initialized != RPC_E_CHANGED_MODE) {
            return fail(Status::PlatformError, "open_device: CoInitializeEx", initialized);
        }

        ComPtr<IMMDeviceEnumerator> enumerator;
        HRESULT result = CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                                          IID_PPV_ARGS(&enumerator));
        if (FAILED(result)) {
            return fail(Status::PlatformError, "open_device: MMDeviceEnumerator", result);
        }

        result = enumerator->GetDefaultAudioEndpoint(eRender, eConsole, &device_);
        if (FAILED(result)) {
            return fail(Status::Unavailable, "open_device: GetDefaultAudioEndpoint", result);
        }

        result = device_->Activate(__uuidof(IAudioClient), CLSCTX_ALL, nullptr, &client_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "open_device: Activate", result);
        }

        WAVEFORMATEXTENSIBLE format = {};
        format.Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
        format.Format.nChannels = config_.format.channels;
        format.Format.nSamplesPerSec = config_.format.sample_rate_hz;
        format.Format.wBitsPerSample = 16;
        format.Format.nBlockAlign =
            static_cast<WORD>(format.Format.nChannels * format.Format.wBitsPerSample / 8);
        format.Format.nAvgBytesPerSec = format.Format.nSamplesPerSec * format.Format.nBlockAlign;
        format.Format.cbSize = sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX);
        format.Samples.wValidBitsPerSample = 16;
        format.dwChannelMask = config_.format.channels == 1
                                   ? SPEAKER_FRONT_CENTER
                                   : (SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT);
        format.SubFormat = KSDATAFORMAT_SUBTYPE_PCM;

        const DWORD flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM |
                            AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;

        const REFERENCE_TIME duration =
            static_cast<REFERENCE_TIME>(config_.target_buffer_ns / kHundredNanoseconds);

        result = client_->Initialize(AUDCLNT_SHAREMODE_SHARED, flags, duration, 0,
                                     reinterpret_cast<const WAVEFORMATEX*>(&format), nullptr);
        if (FAILED(result)) {
            return fail(Status::NotSupported, "open_device: Initialize", result);
        }

        result = client_->GetBufferSize(&buffer_frames_);
        if (FAILED(result)) {
            return fail(Status::PlatformError, "open_device: GetBufferSize", result);
        }

        REFERENCE_TIME default_period = 0;
        REFERENCE_TIME minimum_period = 0;
        if (SUCCEEDED(client_->GetDevicePeriod(&default_period, &minimum_period))) {
            device_period_ns_ = static_cast<Nanoseconds>(default_period) * kHundredNanoseconds;
        }

        return ok();
    }

    void render_thread_main() noexcept
    {
        const HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
        const bool uninitialize = SUCCEEDED(initialized);

        DWORD task_index = 0;
        HANDLE task = AvSetMmThreadCharacteristicsW(L"Pro Audio", &task_index);

        const std::uint16_t channels = config_.format.channels;

        while (running_.load(std::memory_order_relaxed)) {
            if (WaitForSingleObject(event_, kRenderWaitMs) != WAIT_OBJECT_0) {
                continue;
            }
            if (!running_.load(std::memory_order_relaxed)) {
                break;
            }

            UINT32 padding = 0;
            if (FAILED(client_->GetCurrentPadding(&padding))) {
                continue;
            }
            if (padding >= buffer_frames_) {
                continue;
            }

            const UINT32 frames = buffer_frames_ - padding;
            BYTE* buffer = nullptr;
            if (FAILED(render_client_->GetBuffer(frames, &buffer))) {
                continue;
            }

            const std::size_t wanted = static_cast<std::size_t>(frames) * channels;
            const std::size_t budget =
                static_cast<std::size_t>(config_.format.ns_to_frames(config_.target_buffer_ns)) *
                channels;
            const std::size_t queued = ring_.available();
            if (queued > wanted + budget) {
                std::size_t excess = (queued - wanted - budget) / channels * channels;
                std::int16_t scratch[1024];
                while (excess > 0) {
                    const std::size_t chunk =
                        excess < sizeof(scratch) / sizeof(scratch[0])
                            ? excess
                            : sizeof(scratch) / sizeof(scratch[0]) / channels * channels;
                    const std::size_t skipped = ring_.read(scratch, chunk);
                    if (skipped == 0) {
                        break;
                    }
                    excess -= skipped < excess ? skipped : excess;
                    stats_.frames_dropped += skipped / channels;
                }
            }

            auto* const samples = reinterpret_cast<std::int16_t*>(buffer);
            const std::size_t filled = ring_.read(samples, wanted);
            apply_gain(samples, filled, volume_percent_.load(std::memory_order_relaxed));

            DWORD release_flags = 0;
            if (filled < wanted) {
                std::memset(buffer + filled * sizeof(std::int16_t), 0,
                            (wanted - filled) * sizeof(std::int16_t));
                if (filled == 0) {
                    release_flags = AUDCLNT_BUFFERFLAGS_SILENT;
                }
                ++stats_.underruns;
            }

            stats_.frames_rendered += filled / channels;
            render_client_->ReleaseBuffer(frames, release_flags);
        }

        if (task != nullptr) {
            AvRevertMmThreadCharacteristics(task);
        }
        if (uninitialize) {
            CoUninitialize();
        }
    }

    AudioRendererConfig config_;
    AudioRendererStats stats_;
    SampleRing ring_;
    ComPtr<IMMDevice> device_;
    ComPtr<IAudioClient> client_;
    ComPtr<IAudioRenderClient> render_client_;
    std::thread thread_;
    HANDLE event_ = nullptr;
    UINT32 buffer_frames_ = 0;
    Nanoseconds device_period_ns_ = 0;
    std::atomic<bool> running_{false};
    std::atomic<std::uint32_t> volume_percent_{kDefaultVolumePercent};
    bool com_initialized_ = false;
};

}  // namespace

Result<std::unique_ptr<AudioRenderer>> create_wasapi_audio_renderer(
    const AudioRendererConfig& config)
{
    std::unique_ptr<AudioRenderer> renderer(new (std::nothrow) WasapiAudioRenderer(config));
    if (!renderer) {
        return Error{Status::OutOfMemory, "create_wasapi_audio_renderer"};
    }
    return renderer;
}

}  // namespace tl::receive
