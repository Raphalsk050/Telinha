#pragma once

#include "telinha/core/config.hpp"

#if TL_PLATFORM_WINDOWS

#include <windows.h>

#include <mfapi.h>
#include <mfidl.h>
#include <mfobjects.h>
#include <objbase.h>
#include <wrl/client.h>

#include <cstdint>

#include "telinha/core/result.hpp"

namespace tl::encode {

[[nodiscard]] Outcome from_hresult(HRESULT hr, const char* context) noexcept;

class ComApartment {
public:
    ComApartment() noexcept;
    ~ComApartment();

    ComApartment(const ComApartment&) = delete;
    ComApartment& operator=(const ComApartment&) = delete;

private:
    bool owned_ = false;
};

class MediaFoundationRuntime {
public:
    static MediaFoundationRuntime& instance() noexcept;

    [[nodiscard]] bool ready() const noexcept { return ready_; }

private:
    MediaFoundationRuntime() noexcept;
    ~MediaFoundationRuntime();

    MediaFoundationRuntime(const MediaFoundationRuntime&) = delete;
    MediaFoundationRuntime& operator=(const MediaFoundationRuntime&) = delete;

    ComApartment apartment_;
    bool ready_ = false;
};

class AsyncMftPump final : public IMFAsyncCallback {
public:
    [[nodiscard]] static AsyncMftPump* create() noexcept;

    AsyncMftPump(const AsyncMftPump&) = delete;
    AsyncMftPump& operator=(const AsyncMftPump&) = delete;

    [[nodiscard]] Outcome start(IMFTransform* transform) noexcept;
    void detach() noexcept;

    [[nodiscard]] Outcome wait_input_credit(std::uint32_t timeout_ms) noexcept;
    [[nodiscard]] Outcome wait_output_credit(std::uint32_t timeout_ms) noexcept;
    [[nodiscard]] Outcome wait_drained(std::uint32_t timeout_ms) noexcept;

    [[nodiscard]] bool try_take_output_credit() noexcept;
    void return_output_credit() noexcept;
    void clear_credits() noexcept;

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID riid, void** out) noexcept override;
    ULONG STDMETHODCALLTYPE AddRef() noexcept override;
    ULONG STDMETHODCALLTYPE Release() noexcept override;
    HRESULT STDMETHODCALLTYPE GetParameters(DWORD* flags, DWORD* queue) noexcept override;
    HRESULT STDMETHODCALLTYPE Invoke(IMFAsyncResult* result) noexcept override;

private:
    AsyncMftPump() noexcept;
    ~AsyncMftPump();

    void arm() noexcept;
    [[nodiscard]] Outcome wait_credit(std::uint32_t& counter, CONDITION_VARIABLE& condition,
                                      std::uint32_t timeout_ms, const char* context) noexcept;

    Microsoft::WRL::ComPtr<IMFMediaEventGenerator> generator_;
    LONG references_ = 1;

    SRWLOCK lock_ = SRWLOCK_INIT;
    CONDITION_VARIABLE input_ready_ = CONDITION_VARIABLE_INIT;
    CONDITION_VARIABLE output_ready_ = CONDITION_VARIABLE_INIT;
    CONDITION_VARIABLE drain_ready_ = CONDITION_VARIABLE_INIT;

    std::uint32_t input_credits_ = 0;
    std::uint32_t output_credits_ = 0;
    bool drained_ = false;
    bool detached_ = false;
    HRESULT error_ = S_OK;
};

}  // namespace tl::encode

#endif
