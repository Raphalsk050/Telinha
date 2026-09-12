#pragma once

#include "telinha/core/config.hpp"

#if TL_PLATFORM_WINDOWS

#include <audioclient.h>
#include <mmdeviceapi.h>
#include <windows.h>

#include <cstdint>

#include "telinha/audio/audio_format.hpp"
#include "telinha/core/result.hpp"

namespace tl::audio {

template<typename T>
class ComPtr {
public:
    ComPtr() noexcept = default;

    ComPtr(const ComPtr& other) noexcept : pointer_(other.pointer_)
    {
        if (pointer_ != nullptr) {
            pointer_->AddRef();
        }
    }

    ComPtr(ComPtr&& other) noexcept : pointer_(other.pointer_) { other.pointer_ = nullptr; }

    ComPtr& operator=(const ComPtr& other) noexcept
    {
        if (this != &other) {
            if (other.pointer_ != nullptr) {
                other.pointer_->AddRef();
            }
            reset();
            pointer_ = other.pointer_;
        }
        return *this;
    }

    ComPtr& operator=(ComPtr&& other) noexcept
    {
        if (this != &other) {
            reset();
            pointer_ = other.pointer_;
            other.pointer_ = nullptr;
        }
        return *this;
    }

    ~ComPtr() { reset(); }

    void reset() noexcept
    {
        if (pointer_ != nullptr) {
            pointer_->Release();
            pointer_ = nullptr;
        }
    }

    [[nodiscard]] T** put() noexcept
    {
        reset();
        return &pointer_;
    }

    [[nodiscard]] void** put_void() noexcept { return reinterpret_cast<void**>(put()); }

    [[nodiscard]] T* get() const noexcept { return pointer_; }
    T* operator->() const noexcept { return pointer_; }
    explicit operator bool() const noexcept { return pointer_ != nullptr; }

    void attach(T* raw) noexcept
    {
        reset();
        pointer_ = raw;
    }

private:
    T* pointer_ = nullptr;
};

[[nodiscard]] Status status_from_hresult(HRESULT result) noexcept;

[[nodiscard]] Outcome fail_hresult(HRESULT result, const char* context) noexcept;

[[nodiscard]] AudioFormat format_from_waveformat(const WAVEFORMATEX* wave) noexcept;

void fill_waveformat(const AudioFormat& format, WAVEFORMATEXTENSIBLE& wave) noexcept;

[[nodiscard]] std::uint32_t windows_build_number() noexcept;

[[nodiscard]] std::uint32_t resolve_process_tree_root(std::uint32_t process_id) noexcept;

}  // namespace tl::audio

#endif
