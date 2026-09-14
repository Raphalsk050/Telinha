#include "mf_platform.hpp"

#if TL_PLATFORM_WINDOWS

#include <dxgi.h>

#include <mferror.h>
#include <new>

namespace tl::encode {
namespace {

constexpr ULONGLONG kForever = ~static_cast<ULONGLONG>(0);

[[nodiscard]] ULONGLONG deadline_from(std::uint32_t timeout_ms) noexcept
{
    if (timeout_ms == INFINITE) {
        return kForever;
    }
    return GetTickCount64() + timeout_ms;
}

[[nodiscard]] DWORD remaining_from(ULONGLONG deadline) noexcept
{
    if (deadline == kForever) {
        return INFINITE;
    }
    const ULONGLONG now = GetTickCount64();
    return now >= deadline ? 0 : static_cast<DWORD>(deadline - now);
}

}  // namespace

Outcome from_hresult(HRESULT hr, const char* context) noexcept
{
    if (SUCCEEDED(hr)) {
        return ok();
    }

    Status status = Status::PlatformError;
    switch (hr) {
        case E_OUTOFMEMORY: status = Status::OutOfMemory; break;
        case E_INVALIDARG: status = Status::InvalidArgument; break;
        case E_NOTIMPL: status = Status::NotImplemented; break;
        case E_ACCESSDENIED: status = Status::PermissionDenied; break;
        case MF_E_INVALIDMEDIATYPE:
        case MF_E_INVALIDTYPE:
        case MF_E_UNSUPPORTED_D3D_TYPE: status = Status::NotSupported; break;
        case MF_E_TRANSFORM_TYPE_NOT_SET:
        case MF_E_TRANSFORM_NEED_MORE_INPUT: status = Status::WouldBlock; break;
        case MF_E_NOTACCEPTING: status = Status::Full; break;
        case MF_E_TRANSFORM_STREAM_CHANGE:
        case MF_E_TRANSFORM_INPUT_REMAINING: status = Status::ConfigurationChanged; break;
        case MF_E_SHUTDOWN: status = Status::Unavailable; break;
        case DXGI_ERROR_DEVICE_REMOVED:
        case DXGI_ERROR_DEVICE_RESET: status = Status::DeviceLost; break;
        default: break;
    }
    return fail(status, context, static_cast<std::int32_t>(hr));
}

ComApartment::ComApartment() noexcept
{
    const HRESULT hr = CoInitializeEx(nullptr, COINIT_MULTITHREADED | COINIT_DISABLE_OLE1DDE);
    owned_ = SUCCEEDED(hr);
}

ComApartment::~ComApartment()
{
    if (owned_) {
        CoUninitialize();
    }
}

MediaFoundationRuntime& MediaFoundationRuntime::instance() noexcept
{
    static MediaFoundationRuntime runtime;
    return runtime;
}

MediaFoundationRuntime::MediaFoundationRuntime() noexcept
{
    ready_ = SUCCEEDED(MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET));
}

MediaFoundationRuntime::~MediaFoundationRuntime()
{
    if (ready_) {
        (void)MFShutdown();
        ready_ = false;
    }
}

AsyncMftPump* AsyncMftPump::create() noexcept
{
    return new (std::nothrow) AsyncMftPump();
}

AsyncMftPump::AsyncMftPump() noexcept = default;

AsyncMftPump::~AsyncMftPump() = default;

HRESULT STDMETHODCALLTYPE AsyncMftPump::QueryInterface(REFIID riid, void** out) noexcept
{
    if (out == nullptr) {
        return E_POINTER;
    }
    if (riid == __uuidof(IMFAsyncCallback) || riid == __uuidof(IUnknown)) {
        *out = static_cast<IMFAsyncCallback*>(this);
        AddRef();
        return S_OK;
    }
    *out = nullptr;
    return E_NOINTERFACE;
}

ULONG STDMETHODCALLTYPE AsyncMftPump::AddRef() noexcept
{
    return static_cast<ULONG>(InterlockedIncrement(&references_));
}

ULONG STDMETHODCALLTYPE AsyncMftPump::Release() noexcept
{
    const LONG remaining = InterlockedDecrement(&references_);
    if (remaining == 0) {
        delete this;
    }
    return static_cast<ULONG>(remaining);
}

HRESULT STDMETHODCALLTYPE AsyncMftPump::GetParameters(DWORD* flags, DWORD* queue) noexcept
{
    if (flags == nullptr || queue == nullptr) {
        return E_POINTER;
    }
    *flags = 0;
    *queue = MFASYNC_CALLBACK_QUEUE_MULTITHREADED;
    return S_OK;
}

Outcome AsyncMftPump::start(IMFTransform* transform) noexcept
{
    if (transform == nullptr) {
        return fail(Status::InvalidArgument, "media foundation: null transform");
    }

    const HRESULT hr = transform->QueryInterface(IID_PPV_ARGS(generator_.ReleaseAndGetAddressOf()));
    if (FAILED(hr)) {
        return from_hresult(hr, "media foundation: the transform has no event generator");
    }

    arm();
    return ok();
}

void AsyncMftPump::detach() noexcept
{
    AcquireSRWLockExclusive(&lock_);
    detached_ = true;
    ReleaseSRWLockExclusive(&lock_);

    WakeAllConditionVariable(&input_ready_);
    WakeAllConditionVariable(&output_ready_);
    WakeAllConditionVariable(&drain_ready_);
}

void AsyncMftPump::arm() noexcept
{
    AcquireSRWLockExclusive(&lock_);
    const bool stop = detached_ || generator_ == nullptr;
    ReleaseSRWLockExclusive(&lock_);

    if (stop) {
        return;
    }

    const HRESULT hr = generator_->BeginGetEvent(this, nullptr);
    if (FAILED(hr) && hr != MF_E_MULTIPLE_SUBSCRIBERS) {
        AcquireSRWLockExclusive(&lock_);
        if (SUCCEEDED(error_)) {
            error_ = hr;
        }
        ReleaseSRWLockExclusive(&lock_);
        WakeAllConditionVariable(&input_ready_);
        WakeAllConditionVariable(&output_ready_);
        WakeAllConditionVariable(&drain_ready_);
    }
}

HRESULT STDMETHODCALLTYPE AsyncMftPump::Invoke(IMFAsyncResult* result) noexcept
{
    Microsoft::WRL::ComPtr<IMFMediaEvent> event;
    HRESULT hr = generator_ == nullptr ? MF_E_SHUTDOWN : generator_->EndGetEvent(result, &event);

    MediaEventType type = MEUnknown;
    if (SUCCEEDED(hr)) {
        hr = event->GetType(&type);
    }
    if (SUCCEEDED(hr) && type == MEError) {
        HRESULT reported = S_OK;
        if (SUCCEEDED(event->GetStatus(&reported)) && FAILED(reported)) {
            hr = reported;
        } else {
            hr = E_FAIL;
        }
    }

    AcquireSRWLockExclusive(&lock_);
    bool wake_input = false;
    bool wake_output = false;
    bool wake_drain = false;

    if (FAILED(hr)) {
        if (SUCCEEDED(error_)) {
            error_ = hr;
        }
        wake_input = true;
        wake_output = true;
        wake_drain = true;
    } else {
        switch (type) {
            case METransformNeedInput:
                ++input_credits_;
                wake_input = true;
                break;
            case METransformHaveOutput:
                ++output_credits_;
                wake_output = true;
                break;
            case METransformDrainComplete:
                drained_ = true;
                wake_drain = true;
                break;
            default: break;
        }
    }
    ReleaseSRWLockExclusive(&lock_);

    if (wake_input) {
        WakeAllConditionVariable(&input_ready_);
    }
    if (wake_output) {
        WakeAllConditionVariable(&output_ready_);
    }
    if (wake_drain) {
        WakeAllConditionVariable(&drain_ready_);
    }

    if (SUCCEEDED(hr)) {
        arm();
    }
    return S_OK;
}

Outcome AsyncMftPump::wait_credit(std::uint32_t& counter, CONDITION_VARIABLE& condition,
                                  std::uint32_t timeout_ms, const char* context) noexcept
{
    const ULONGLONG deadline = deadline_from(timeout_ms);

    AcquireSRWLockExclusive(&lock_);
    for (;;) {
        if (counter > 0) {
            --counter;
            ReleaseSRWLockExclusive(&lock_);
            return ok();
        }
        if (FAILED(error_)) {
            const HRESULT reported = error_;
            ReleaseSRWLockExclusive(&lock_);
            return from_hresult(reported, context);
        }
        if (detached_) {
            ReleaseSRWLockExclusive(&lock_);
            return fail(Status::Unavailable, context);
        }

        const DWORD remaining = remaining_from(deadline);
        if (remaining == 0) {
            ReleaseSRWLockExclusive(&lock_);
            return fail(Status::Timeout, context);
        }
        (void)SleepConditionVariableSRW(&condition, &lock_, remaining, 0);
    }
}

Outcome AsyncMftPump::wait_input_credit(std::uint32_t timeout_ms) noexcept
{
    return wait_credit(input_credits_, input_ready_, timeout_ms,
                       "media foundation: the encoder accepted no input");
}

Outcome AsyncMftPump::wait_output_credit(std::uint32_t timeout_ms) noexcept
{
    return wait_credit(output_credits_, output_ready_, timeout_ms,
                       "media foundation: no encoded frame is pending");
}

Outcome AsyncMftPump::wait_drained(std::uint32_t timeout_ms) noexcept
{
    const ULONGLONG deadline = deadline_from(timeout_ms);

    AcquireSRWLockExclusive(&lock_);
    for (;;) {
        if (drained_) {
            ReleaseSRWLockExclusive(&lock_);
            return ok();
        }
        if (FAILED(error_) || detached_) {
            ReleaseSRWLockExclusive(&lock_);
            return fail(Status::Unavailable, "media foundation: the drain did not complete");
        }
        const DWORD remaining = remaining_from(deadline);
        if (remaining == 0) {
            ReleaseSRWLockExclusive(&lock_);
            return fail(Status::Timeout, "media foundation: the drain did not complete");
        }
        (void)SleepConditionVariableSRW(&drain_ready_, &lock_, remaining, 0);
    }
}

bool AsyncMftPump::try_take_output_credit() noexcept
{
    AcquireSRWLockExclusive(&lock_);
    const bool taken = output_credits_ > 0;
    if (taken) {
        --output_credits_;
    }
    ReleaseSRWLockExclusive(&lock_);
    return taken;
}

void AsyncMftPump::return_output_credit() noexcept
{
    AcquireSRWLockExclusive(&lock_);
    ++output_credits_;
    ReleaseSRWLockExclusive(&lock_);
    WakeAllConditionVariable(&output_ready_);
}

void AsyncMftPump::clear_credits() noexcept
{
    AcquireSRWLockExclusive(&lock_);
    input_credits_ = 0;
    output_credits_ = 0;
    drained_ = false;
    ReleaseSRWLockExclusive(&lock_);
}

}  // namespace tl::encode

#endif
