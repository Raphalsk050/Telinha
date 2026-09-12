#pragma once

#include "telinha/core/config.hpp"

#if TL_PLATFORM_WINDOWS

#include <windows.h>

#include "nvEncodeAPI.h"
#include "telinha/core/result.hpp"

namespace tl::encode {

class NvencLibrary {
public:
    static NvencLibrary& instance() noexcept;

    [[nodiscard]] bool loaded() const noexcept { return loaded_; }
    [[nodiscard]] const NV_ENCODE_API_FUNCTION_LIST& functions() const noexcept
    {
        return functions_;
    }

private:
    NvencLibrary() noexcept;
    ~NvencLibrary();

    NvencLibrary(const NvencLibrary&) = delete;
    NvencLibrary& operator=(const NvencLibrary&) = delete;

    HMODULE module_ = nullptr;
    NV_ENCODE_API_FUNCTION_LIST functions_{};
    bool loaded_ = false;
};

[[nodiscard]] Outcome from_nvenc_status(NVENCSTATUS status, const char* context) noexcept;

}  // namespace tl::encode

#endif
