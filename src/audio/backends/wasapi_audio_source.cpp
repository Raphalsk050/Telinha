#include "telinha/core/config.hpp"

#if TL_PLATFORM_WINDOWS

#include <audioclientactivationparams.h>
#include <avrt.h>

#include <algorithm>
#include <atomic>
#include <thread>

#include "../audio_capture_stream.hpp"
#include "telinha/audio/audio_source.hpp"
#include "telinha/core/log.hpp"
#include "wasapi_support.hpp"

namespace tl::audio {
namespace {

constexpr std::uint32_t kProcessLoopbackMinimumBuild = 20348;
constexpr Nanoseconds kRingCapacityNs = 400 * kNanosecondsPerMillisecond;
constexpr Nanoseconds kMaxSilenceBurstNs = 200 * kNanosecondsPerMillisecond;
constexpr std::uint32_t kActivationTimeoutMs = 2000;
constexpr std::uint32_t kDefaultPollMs = 5;

class ActivationHandler final : public IActivateAudioInterfaceCompletionHandler,
                                public IAgileObject {
public:
    ActivationHandler() noexcept : completed_(::CreateEventW(nullptr, TRUE, FALSE, nullptr)) {}

    ~ActivationHandler()
    {
        if (completed_ != nullptr) {
            ::CloseHandle(completed_);
        }
    }

    ActivationHandler(const ActivationHandler&) = delete;
    ActivationHandler& operator=(const ActivationHandler&) = delete;

    [[nodiscard]] bool valid() const noexcept { return completed_ != nullptr; }

    [[nodiscard]] bool wait(std::uint32_t timeout_ms) const noexcept
    {
        return ::WaitForSingleObject(completed_, timeout_ms) == WAIT_OBJECT_0;
    }

    [[nodiscard]] HRESULT activation_result() const noexcept { return activation_result_; }
    [[nodiscard]] ComPtr<IAudioClient>& client() noexcept { return client_; }

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** object) override
    {
        if (object == nullptr) {
            return E_POINTER;
        }
        if (riid == __uuidof(IUnknown) ||
            riid == __uuidof(IActivateAudioInterfaceCompletionHandler)) {
            *object = static_cast<IActivateAudioInterfaceCompletionHandler*>(this);
        } else if (riid == __uuidof(IAgileObject)) {
            *object = static_cast<IAgileObject*>(this);
        } else {
            *object = nullptr;
            return E_NOINTERFACE;
        }
        AddRef();
        return S_OK;
    }

    ULONG STDMETHODCALLTYPE AddRef() override
    {
        return static_cast<ULONG>(references_.fetch_add(1, std::memory_order_relaxed) + 1);
    }

    ULONG STDMETHODCALLTYPE Release() override
    {
        const long remaining = references_.fetch_sub(1, std::memory_order_acq_rel) - 1;
        if (remaining == 0) {
            delete this;
        }
        return static_cast<ULONG>(remaining);
    }

    HRESULT STDMETHODCALLTYPE
    ActivateCompleted(IActivateAudioInterfaceAsyncOperation* operation) override
    {
        HRESULT activation = E_FAIL;
        ComPtr<IUnknown> unknown;

        if (operation != nullptr) {
            const HRESULT queried = operation->GetActivateResult(&activation, unknown.put());
            if (FAILED(queried)) {
                activation = queried;
            }
        }

        if (SUCCEEDED(activation) && unknown) {
            const HRESULT cast =
                unknown->QueryInterface(__uuidof(IAudioClient), client_.put_void());
            if (FAILED(cast)) {
                activation = cast;
            }
        }

        activation_result_ = activation;
        ::SetEvent(completed_);
        return S_OK;
    }

private:
    std::atomic<long> references_{1};
    HANDLE completed_ = nullptr;
    HRESULT activation_result_ = E_FAIL;
    ComPtr<IAudioClient> client_;
};

class DeviceChangeListener final : public IMMNotificationClient {
public:
    explicit DeviceChangeListener(std::atomic<bool>& flag) noexcept : changed_(&flag) {}

    DeviceChangeListener(const DeviceChangeListener&) = delete;
    DeviceChangeListener& operator=(const DeviceChangeListener&) = delete;

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** object) override
    {
        if (object == nullptr) {
            return E_POINTER;
        }
        if (riid == __uuidof(IUnknown) || riid == __uuidof(IMMNotificationClient)) {
            *object = static_cast<IMMNotificationClient*>(this);
            AddRef();
            return S_OK;
        }
        *object = nullptr;
        return E_NOINTERFACE;
    }

    ULONG STDMETHODCALLTYPE AddRef() override
    {
        return static_cast<ULONG>(references_.fetch_add(1, std::memory_order_relaxed) + 1);
    }

    ULONG STDMETHODCALLTYPE Release() override
    {
        const long remaining = references_.fetch_sub(1, std::memory_order_acq_rel) - 1;
        if (remaining == 0) {
            delete this;
        }
        return static_cast<ULONG>(remaining);
    }

    HRESULT STDMETHODCALLTYPE OnDefaultDeviceChanged(EDataFlow flow, ERole role, LPCWSTR) override
    {
        if (flow == eRender && role == eConsole) {
            changed_->store(true, std::memory_order_release);
        }
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE OnDeviceStateChanged(LPCWSTR, DWORD) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE OnDeviceAdded(LPCWSTR) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE OnDeviceRemoved(LPCWSTR) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE OnPropertyValueChanged(LPCWSTR, const PROPERTYKEY) override
    {
        return S_OK;
    }

private:
    std::atomic<long> references_{1};
    std::atomic<bool>* changed_;
};

class WasapiAudioSource final : public AudioSource {
public:
    WasapiAudioSource() noexcept = default;

    ~WasapiAudioSource() override
    {
        stop();
        release_endpoint();
        if (enumerator_ && listener_ != nullptr) {
            enumerator_->UnregisterEndpointNotificationCallback(listener_);
        }
        if (listener_ != nullptr) {
            listener_->Release();
            listener_ = nullptr;
        }
        enumerator_.reset();
        if (com_initialized_) {
            ::CoUninitialize();
        }
    }

    [[nodiscard]] Outcome open(const AudioCaptureTarget& target,
                               const AudioCaptureOptions& options) noexcept;

    Outcome start() override;
    void stop() noexcept override;
    [[nodiscard]] Outcome acquire(CapturedAudio& out, std::uint32_t timeout_ms) override;
    void release() noexcept override;
    [[nodiscard]] AudioSourceInfo info() const noexcept override { return info_; }

private:
    [[nodiscard]] Outcome open_endpoint_loopback() noexcept;
    [[nodiscard]] Outcome open_process_loopback(std::uint32_t process_id,
                                                ProcessLoopbackMode mode) noexcept;
    [[nodiscard]] Outcome finish_client_setup(bool event_driven) noexcept;
    void release_endpoint() noexcept;
    void capture_loop() noexcept;
    void drain_packets(Nanoseconds& next_expected_ns, bool& produced) noexcept;
    void handle_device_change() noexcept;

    AudioCaptureStream stream_;
    AudioCaptureOptions options_;
    AudioSourceInfo info_;

    ComPtr<IMMDeviceEnumerator> enumerator_;
    ComPtr<IAudioClient> client_;
    ComPtr<IAudioCaptureClient> capture_;
    DeviceChangeListener* listener_ = nullptr;
    HANDLE ready_event_ = nullptr;

    std::thread worker_;
    std::atomic<bool> running_{false};
    std::atomic<bool> device_changed_{false};
    std::atomic<bool> device_lost_{false};

    AudioFormat device_format_;
    AudioFormat transmit_format_;
    Nanoseconds device_period_ns_ = 0;
    std::uint32_t buffer_frames_ = 0;
    std::uint32_t poll_ms_ = kDefaultPollMs;
    bool com_initialized_ = false;
    bool event_driven_ = false;
    bool held_ = false;
};

Outcome WasapiAudioSource::open(const AudioCaptureTarget& target,
                                const AudioCaptureOptions& options) noexcept
{
    if (!target.valid()) {
        return fail(Status::InvalidArgument, "wasapi: audio target is not valid");
    }

    const HRESULT com = ::CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (com == RPC_E_CHANGED_MODE) {
        com_initialized_ = false;
    } else if (FAILED(com)) {
        return fail_hresult(com, "wasapi: CoInitializeEx");
    } else {
        com_initialized_ = true;
    }

    options_ = options;
    transmit_format_ = options.requested_format.valid()
                           ? options.requested_format
                           : AudioFormat{48000, 2, SampleFormat::Int16};

    info_.target = target;
    info_.process_loopback_supported = windows_build_number() >= kProcessLoopbackMinimumBuild;

    const HRESULT created =
        ::CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                           __uuidof(IMMDeviceEnumerator), enumerator_.put_void());
    if (FAILED(created)) {
        return fail_hresult(created, "wasapi: MMDeviceEnumerator");
    }

    const bool excluding = target.scope == AudioCaptureScope::ProcessLoopback &&
                           target.process_loopback_mode == ProcessLoopbackMode::ExcludeProcessTree;

    if (target.scope == AudioCaptureScope::ProcessLoopback && info_.process_loopback_supported) {
        const std::uint32_t root = resolve_process_tree_root(target.process_id);
        const Outcome activated = open_process_loopback(root, target.process_loopback_mode);
        if (activated.ok()) {
            info_.target = AudioCaptureTarget::process_loopback(root, target.process_loopback_mode);
        } else if (excluding) {
            return activated;
        } else {
            TL_LOG_WARN(
                "wasapi: process loopback for pid %u failed with %s, falling back to the "
                "system mix",
                target.process_id, to_string(activated.error().status));
            release_endpoint();
            info_.process_loopback_supported = false;
            info_.target = AudioCaptureTarget::system_loopback();
            TL_TRY(open_endpoint_loopback());
        }
    } else {
        if (excluding) {
            return fail(Status::NotSupported,
                        "wasapi: excluding a process tree needs process loopback",
                        static_cast<std::int32_t>(windows_build_number()));
        }
        if (target.scope == AudioCaptureScope::ProcessLoopback) {
            TL_LOG_WARN("wasapi: build %u has no process loopback, falling back to the system mix",
                        windows_build_number());
            info_.target = AudioCaptureTarget::system_loopback();
        }
        TL_TRY(open_endpoint_loopback());
    }

    listener_ = new (std::nothrow) DeviceChangeListener(device_changed_);
    if (listener_ != nullptr) {
        enumerator_->RegisterEndpointNotificationCallback(listener_);
    }

    info_.format = transmit_format_;
    info_.buffer_duration_us =
        static_cast<std::uint32_t>(device_period_ns_ / kNanosecondsPerMicrosecond);

    TL_TRY(stream_.configure(device_format_, transmit_format_, buffer_frames_, kRingCapacityNs));
    return ok();
}

Outcome WasapiAudioSource::open_endpoint_loopback() noexcept
{
    ComPtr<IMMDevice> device;
    const HRESULT endpoint = enumerator_->GetDefaultAudioEndpoint(eRender, eConsole, device.put());
    if (FAILED(endpoint)) {
        return fail_hresult(endpoint, "wasapi: no default render endpoint");
    }

    const HRESULT activated =
        device->Activate(__uuidof(IAudioClient), CLSCTX_ALL, nullptr, client_.put_void());
    if (FAILED(activated)) {
        return fail_hresult(activated, "wasapi: IAudioClient activation");
    }

    WAVEFORMATEX* mix = nullptr;
    const HRESULT mixed = client_->GetMixFormat(&mix);
    if (FAILED(mixed) || mix == nullptr) {
        return fail_hresult(mixed, "wasapi: GetMixFormat");
    }

    device_format_ = format_from_waveformat(mix);

    REFERENCE_TIME requested = static_cast<REFERENCE_TIME>(options_.buffer_duration_us) * 10;
    if (requested <= 0) {
        requested = 100000;
    }

    const HRESULT initialized = client_->Initialize(
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, requested, 0, mix, nullptr);
    ::CoTaskMemFree(mix);

    if (FAILED(initialized)) {
        return fail_hresult(initialized, "wasapi: IAudioClient::Initialize for endpoint loopback");
    }
    if (!device_format_.valid()) {
        return fail(Status::NotSupported, "wasapi: endpoint mix format is not supported");
    }

    return finish_client_setup(false);
}

Outcome WasapiAudioSource::open_process_loopback(std::uint32_t process_id,
                                                 ProcessLoopbackMode mode) noexcept
{
    AUDIOCLIENT_ACTIVATION_PARAMS parameters{};
    parameters.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
    parameters.ProcessLoopbackParams.TargetProcessId = static_cast<DWORD>(process_id);
    parameters.ProcessLoopbackParams.ProcessLoopbackMode =
        mode == ProcessLoopbackMode::ExcludeProcessTree
            ? PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE
            : PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE;

    PROPVARIANT variant{};
    variant.vt = VT_BLOB;
    variant.blob.cbSize = sizeof(parameters);
    variant.blob.pBlobData = reinterpret_cast<BYTE*>(&parameters);

    ActivationHandler* handler = new (std::nothrow) ActivationHandler();
    if (handler == nullptr) {
        return fail(Status::OutOfMemory, "wasapi: activation handler");
    }
    if (!handler->valid()) {
        handler->Release();
        return fail(Status::Unavailable, "wasapi: activation event");
    }

    ComPtr<IActivateAudioInterfaceAsyncOperation> operation;
    const HRESULT requested =
        ::ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, __uuidof(IAudioClient),
                                      &variant, handler, operation.put());

    if (FAILED(requested)) {
        handler->Release();
        return fail_hresult(requested, "wasapi: ActivateAudioInterfaceAsync");
    }

    if (!handler->wait(kActivationTimeoutMs)) {
        handler->Release();
        return fail(Status::Timeout, "wasapi: process loopback activation timed out");
    }

    const HRESULT activation = handler->activation_result();
    if (FAILED(activation)) {
        handler->Release();
        return fail_hresult(activation, "wasapi: process loopback activation");
    }

    client_ = handler->client();
    handler->Release();

    if (!client_) {
        return fail(Status::Unavailable, "wasapi: process loopback returned no client");
    }

    device_format_ =
        AudioFormat{transmit_format_.sample_rate, transmit_format_.channels, SampleFormat::Float32};

    WAVEFORMATEXTENSIBLE wave{};
    fill_waveformat(device_format_, wave);

    REFERENCE_TIME requested_duration =
        static_cast<REFERENCE_TIME>(options_.buffer_duration_us) * 10;
    if (requested_duration <= 0) {
        requested_duration = 100000;
    }

    const HRESULT initialized = client_->Initialize(
        AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        requested_duration, 0, reinterpret_cast<const WAVEFORMATEX*>(&wave), nullptr);
    if (FAILED(initialized)) {
        return fail_hresult(initialized, "wasapi: IAudioClient::Initialize for process loopback");
    }

    return finish_client_setup(true);
}

Outcome WasapiAudioSource::finish_client_setup(bool event_driven) noexcept
{
    UINT32 frames = 0;
    const HRESULT sized = client_->GetBufferSize(&frames);
    if (FAILED(sized) || frames == 0) {
        return fail_hresult(sized, "wasapi: GetBufferSize");
    }
    buffer_frames_ = static_cast<std::uint32_t>(frames);

    REFERENCE_TIME default_period = 0;
    REFERENCE_TIME minimum_period = 0;
    if (SUCCEEDED(client_->GetDevicePeriod(&default_period, &minimum_period))) {
        device_period_ns_ = static_cast<Nanoseconds>(default_period) * 100ull;
    }
    if (device_period_ns_ == 0) {
        device_period_ns_ = 10 * kNanosecondsPerMillisecond;
    }

    poll_ms_ = std::max<std::uint32_t>(
        1, static_cast<std::uint32_t>(device_period_ns_ / (2 * kNanosecondsPerMillisecond)));

    if (event_driven) {
        ready_event_ = ::CreateEventW(nullptr, FALSE, FALSE, nullptr);
        if (ready_event_ == nullptr) {
            return fail(Status::Unavailable, "wasapi: capture event");
        }
        const HRESULT bound = client_->SetEventHandle(ready_event_);
        if (FAILED(bound)) {
            return fail_hresult(bound, "wasapi: SetEventHandle");
        }
    }
    event_driven_ = event_driven;

    const HRESULT service = client_->GetService(__uuidof(IAudioCaptureClient), capture_.put_void());
    if (FAILED(service)) {
        return fail_hresult(service, "wasapi: IAudioCaptureClient");
    }

    return ok();
}

void WasapiAudioSource::release_endpoint() noexcept
{
    capture_.reset();
    client_.reset();
    if (ready_event_ != nullptr) {
        ::CloseHandle(ready_event_);
        ready_event_ = nullptr;
    }
}

Outcome WasapiAudioSource::start()
{
    if (running_.load(std::memory_order_acquire)) {
        return ok();
    }
    if (!client_ || !capture_ || !stream_.configured()) {
        return fail(Status::Unavailable, "wasapi: source is not open");
    }

    stream_.reset();
    device_lost_.store(false, std::memory_order_relaxed);
    device_changed_.store(false, std::memory_order_relaxed);

    const HRESULT started = client_->Start();
    if (FAILED(started)) {
        return fail_hresult(started, "wasapi: IAudioClient::Start");
    }

    running_.store(true, std::memory_order_release);
    worker_ = std::thread([this] { capture_loop(); });
    return ok();
}

void WasapiAudioSource::stop() noexcept
{
    if (!running_.exchange(false, std::memory_order_acq_rel)) {
        return;
    }
    if (ready_event_ != nullptr) {
        ::SetEvent(ready_event_);
    }
    if (worker_.joinable()) {
        worker_.join();
    }
    if (client_) {
        client_->Stop();
    }

    const AudioCaptureCounters& counters = stream_.counters();
    TL_LOG_INFO(
        "wasapi: %llu packets, %llu frames, %llu silent, %llu synthesised, %llu dropped, "
        "convert p50 %.0f us p99 %.0f us",
        static_cast<unsigned long long>(counters.packets_captured),
        static_cast<unsigned long long>(counters.frames_captured),
        static_cast<unsigned long long>(counters.packets_silent),
        static_cast<unsigned long long>(counters.packets_synthesised),
        static_cast<unsigned long long>(counters.packets_dropped_ring_full),
        static_cast<double>(counters.convert_ns.percentile_ns(50.0)) / 1000.0,
        static_cast<double>(counters.convert_ns.percentile_ns(99.0)) / 1000.0);
}

void WasapiAudioSource::drain_packets(Nanoseconds& next_expected_ns, bool& produced) noexcept
{
    for (;;) {
        UINT32 packet_frames = 0;
        HRESULT queried = capture_->GetNextPacketSize(&packet_frames);
        if (FAILED(queried)) {
            device_lost_.store(true, std::memory_order_release);
            return;
        }
        if (packet_frames == 0) {
            return;
        }

        BYTE* data = nullptr;
        UINT32 frames = 0;
        DWORD flags = 0;
        UINT64 device_position = 0;
        UINT64 qpc_position = 0;

        queried = capture_->GetBuffer(&data, &frames, &flags, &device_position, &qpc_position);
        if (queried == AUDCLNT_S_BUFFER_EMPTY) {
            return;
        }
        if (FAILED(queried)) {
            device_lost_.store(true, std::memory_order_release);
            return;
        }

        const bool silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT) != 0;
        const bool discontinuity = (flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY) != 0;
        const bool timestamp_valid = (flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR) == 0;
        const Nanoseconds capture_time_ns = now_ns();
        const Nanoseconds device_time_ns =
            timestamp_valid ? qpc_position * 100ull : capture_time_ns;

        if (frames != 0) {
            const std::byte* samples = silent ? nullptr : reinterpret_cast<const std::byte*>(data);
            static_cast<void>(stream_.submit(samples, frames, device_time_ns, capture_time_ns,
                                             timestamp_valid, silent, discontinuity));
            produced = true;
            next_expected_ns = device_time_ns + static_cast<Nanoseconds>(frames) *
                                                    kNanosecondsPerSecond /
                                                    device_format_.sample_rate;
        }

        capture_->ReleaseBuffer(frames);
    }
}

void WasapiAudioSource::handle_device_change() noexcept
{
    if (info_.target.scope != AudioCaptureScope::SystemLoopback) {
        return;
    }

    stream_.note_device_change();

    if (client_) {
        client_->Stop();
    }
    release_endpoint();

    const Outcome reopened = open_endpoint_loopback();
    if (!reopened.ok()) {
        TL_LOG_ERROR("wasapi: could not reopen the default endpoint: %s",
                     to_string(reopened.error().status));
        device_lost_.store(true, std::memory_order_release);
        return;
    }

    const Outcome rebound = stream_.rebind_device_format(device_format_, buffer_frames_);
    if (!rebound.ok()) {
        TL_LOG_ERROR("wasapi: the new endpoint format does not fit the packet budget: %s",
                     to_string(rebound.error().status));
        device_lost_.store(true, std::memory_order_release);
        return;
    }

    const HRESULT started = client_->Start();
    if (FAILED(started)) {
        TL_LOG_ERROR("wasapi: could not restart the endpoint, hresult 0x%08lx",
                     static_cast<unsigned long>(started));
        device_lost_.store(true, std::memory_order_release);
    }
}

void WasapiAudioSource::capture_loop() noexcept
{
    const HRESULT com = ::CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    const bool owns_com = SUCCEEDED(com);

    DWORD task_index = 0;
    HANDLE task = ::AvSetMmThreadCharacteristicsW(L"Pro Audio", &task_index);

    Nanoseconds next_expected_ns = 0;

    while (running_.load(std::memory_order_acquire)) {
        if (event_driven_ && ready_event_ != nullptr) {
            ::WaitForSingleObject(ready_event_, poll_ms_ * 4);
        } else {
            ::Sleep(poll_ms_);
        }

        if (!running_.load(std::memory_order_acquire)) {
            break;
        }

        if (device_changed_.exchange(false, std::memory_order_acq_rel)) {
            handle_device_change();
            next_expected_ns = 0;
            continue;
        }

        bool produced = false;
        drain_packets(next_expected_ns, produced);

        if (device_lost_.load(std::memory_order_acquire)) {
            break;
        }

        if (!produced && options_.emit_silence_when_idle) {
            const Nanoseconds now = now_ns();
            if (next_expected_ns == 0) {
                next_expected_ns = now;
            } else if (now > next_expected_ns + device_period_ns_) {
                Nanoseconds gap = now - next_expected_ns;
                if (gap > kMaxSilenceBurstNs) {
                    gap = kMaxSilenceBurstNs;
                    next_expected_ns = now - kMaxSilenceBurstNs;
                }
                const std::uint32_t frames = static_cast<std::uint32_t>(
                    gap * device_format_.sample_rate / kNanosecondsPerSecond);
                if (frames != 0) {
                    static_cast<void>(stream_.submit_silence(frames, next_expected_ns, now));
                    next_expected_ns += static_cast<Nanoseconds>(frames) * kNanosecondsPerSecond /
                                        device_format_.sample_rate;
                }
            }
        }
    }

    if (task != nullptr) {
        ::AvRevertMmThreadCharacteristics(task);
    }
    if (owns_com) {
        ::CoUninitialize();
    }
}

Outcome WasapiAudioSource::acquire(CapturedAudio& out, std::uint32_t timeout_ms)
{
    if (held_) {
        release();
    }

    const Nanoseconds deadline =
        now_ns() + static_cast<Nanoseconds>(timeout_ms) * kNanosecondsPerMillisecond;
    for (;;) {
        if (stream_.acquire(out)) {
            held_ = true;
            return ok();
        }
        if (device_lost_.load(std::memory_order_acquire)) {
            return fail(Status::DeviceLost, "wasapi: the capture endpoint went away");
        }
        if (!running_.load(std::memory_order_acquire)) {
            return fail(Status::Unavailable, "wasapi: capture is not running");
        }
        if (now_ns() >= deadline) {
            return fail(Status::Timeout, "wasapi: no audio packet within the timeout");
        }
        ::Sleep(1);
    }
}

void WasapiAudioSource::release() noexcept
{
    if (held_) {
        stream_.release();
        held_ = false;
    }
}

}  // namespace

bool process_loopback_available() noexcept
{
    return windows_build_number() >= kProcessLoopbackMinimumBuild;
}

Result<std::unique_ptr<AudioSource>> create_audio_source(const AudioCaptureTarget& target,
                                                         const AudioCaptureOptions& options)
{
    auto source = std::unique_ptr<WasapiAudioSource>(new (std::nothrow) WasapiAudioSource());
    if (source == nullptr) {
        return Error{Status::OutOfMemory, "create_audio_source: allocation failed"};
    }

    const Outcome opened = source->open(target, options);
    if (!opened.ok()) {
        return opened.error();
    }

    return std::unique_ptr<AudioSource>(source.release());
}

}  // namespace tl::audio

#endif
