#include "nvenc_library.hpp"

#if TL_PLATFORM_WINDOWS

#include <cstring>

namespace tl::encode {
namespace {

using CreateInstanceProc = NVENCSTATUS(NVENCAPI*)(NV_ENCODE_API_FUNCTION_LIST*);
using MaxVersionProc = NVENCSTATUS(NVENCAPI*)(uint32_t*);

}  // namespace

NvencLibrary& NvencLibrary::instance() noexcept
{
    static NvencLibrary library;
    return library;
}

NvencLibrary::NvencLibrary() noexcept
{
    module_ = ::LoadLibraryW(L"nvEncodeAPI64.dll");
    if (module_ == nullptr) {
        return;
    }

    const auto max_version = reinterpret_cast<MaxVersionProc>(
        reinterpret_cast<void*>(::GetProcAddress(module_, "NvEncodeAPIGetMaxSupportedVersion")));
    if (max_version != nullptr) {
        uint32_t driver_version = 0;
        if (max_version(&driver_version) != NV_ENC_SUCCESS) {
            return;
        }
        const uint32_t required = (NVENCAPI_MAJOR_VERSION << 4) | NVENCAPI_MINOR_VERSION;
        if (driver_version < required) {
            return;
        }
    }

    const auto create_instance = reinterpret_cast<CreateInstanceProc>(
        reinterpret_cast<void*>(::GetProcAddress(module_, "NvEncodeAPICreateInstance")));
    if (create_instance == nullptr) {
        return;
    }

    std::memset(&functions_, 0, sizeof(functions_));
    functions_.version = NV_ENCODE_API_FUNCTION_LIST_VER;
    if (create_instance(&functions_) != NV_ENC_SUCCESS) {
        std::memset(&functions_, 0, sizeof(functions_));
        return;
    }

    loaded_ = functions_.nvEncOpenEncodeSessionEx != nullptr;
}

NvencLibrary::~NvencLibrary()
{
    if (module_ != nullptr) {
        ::FreeLibrary(module_);
        module_ = nullptr;
    }
    loaded_ = false;
}

Outcome from_nvenc_status(NVENCSTATUS status, const char* context) noexcept
{
    if (status == NV_ENC_SUCCESS) {
        return ok();
    }
    switch (status) {
        case NV_ENC_ERR_UNSUPPORTED_DEVICE:
        case NV_ENC_ERR_UNSUPPORTED_PARAM:
            return fail(Status::NotSupported, context, static_cast<std::int32_t>(status));
        case NV_ENC_ERR_OUT_OF_MEMORY:
            return fail(Status::OutOfMemory, context, static_cast<std::int32_t>(status));
        case NV_ENC_ERR_INVALID_PARAM:
        case NV_ENC_ERR_INVALID_PTR:
        case NV_ENC_ERR_INVALID_VERSION:
            return fail(Status::InvalidArgument, context, static_cast<std::int32_t>(status));
        case NV_ENC_ERR_ENCODER_BUSY:
            return fail(Status::WouldBlock, context, static_cast<std::int32_t>(status));
        case NV_ENC_ERR_NEED_MORE_INPUT:
            return fail(Status::Empty, context, static_cast<std::int32_t>(status));
        case NV_ENC_ERR_DEVICE_NOT_EXIST:
            return fail(Status::DeviceLost, context, static_cast<std::int32_t>(status));
        default: break;
    }
    return fail(Status::PlatformError, context, static_cast<std::int32_t>(status));
}

}  // namespace tl::encode

#endif
