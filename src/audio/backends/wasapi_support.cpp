#include "wasapi_support.hpp"

#if TL_PLATFORM_WINDOWS

#include <mmreg.h>
#include <tlhelp32.h>

#include <cstring>

namespace tl::audio {
namespace {

constexpr std::uint32_t kMaxAncestorHops = 16;

struct ProcessIdentity {
    std::uint32_t parent_id = 0;
    wchar_t image[MAX_PATH] = {};
    bool found = false;
};

[[nodiscard]] bool creation_time_of(std::uint32_t process_id, ULONGLONG& out) noexcept
{
    const HANDLE process =
        ::OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, static_cast<DWORD>(process_id));
    if (process == nullptr) {
        return false;
    }

    FILETIME creation{};
    FILETIME exit_time{};
    FILETIME kernel{};
    FILETIME user{};
    const BOOL ok = ::GetProcessTimes(process, &creation, &exit_time, &kernel, &user);
    ::CloseHandle(process);
    if (ok == FALSE) {
        return false;
    }

    ULARGE_INTEGER value{};
    value.LowPart = creation.dwLowDateTime;
    value.HighPart = creation.dwHighDateTime;
    out = value.QuadPart;
    return true;
}

[[nodiscard]] ProcessIdentity identity_of(std::uint32_t process_id) noexcept
{
    ProcessIdentity identity;

    const HANDLE snapshot = ::CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
    if (snapshot == INVALID_HANDLE_VALUE) {
        return identity;
    }

    PROCESSENTRY32W entry{};
    entry.dwSize = sizeof(entry);
    if (::Process32FirstW(snapshot, &entry) != FALSE) {
        do {
            if (entry.th32ProcessID == static_cast<DWORD>(process_id)) {
                identity.parent_id = static_cast<std::uint32_t>(entry.th32ParentProcessID);
                std::wcsncpy(identity.image, entry.szExeFile, MAX_PATH - 1);
                identity.found = true;
                break;
            }
        } while (::Process32NextW(snapshot, &entry) != FALSE);
    }

    ::CloseHandle(snapshot);
    return identity;
}

}  // namespace

Status status_from_hresult(HRESULT result) noexcept
{
    switch (result) {
        case S_OK: return Status::Ok;
        case E_INVALIDARG: return Status::InvalidArgument;
        case E_OUTOFMEMORY: return Status::OutOfMemory;
        case E_ACCESSDENIED: return Status::PermissionDenied;
        case E_NOTIMPL: return Status::NotImplemented;
        case AUDCLNT_E_DEVICE_INVALIDATED: return Status::DeviceLost;
        case AUDCLNT_E_SERVICE_NOT_RUNNING: return Status::Unavailable;
        case AUDCLNT_E_DEVICE_IN_USE: return Status::Unavailable;
        case AUDCLNT_E_UNSUPPORTED_FORMAT: return Status::NotSupported;
        case AUDCLNT_E_ENDPOINT_CREATE_FAILED: return Status::Unavailable;
        case AUDCLNT_E_BUFFER_TOO_LARGE: return Status::InvalidArgument;
        case AUDCLNT_E_OUT_OF_ORDER: return Status::Unknown;
        case AUDCLNT_S_BUFFER_EMPTY: return Status::Empty;
        default: break;
    }
    return Status::PlatformError;
}

Outcome fail_hresult(HRESULT result, const char* context) noexcept
{
    return fail(status_from_hresult(result), context, static_cast<std::int32_t>(result));
}

AudioFormat format_from_waveformat(const WAVEFORMATEX* wave) noexcept
{
    AudioFormat format;
    if (wave == nullptr) {
        return format;
    }

    format.sample_rate = static_cast<std::uint32_t>(wave->nSamplesPerSec);
    format.channels = static_cast<std::uint16_t>(wave->nChannels);

    GUID subformat = GUID_NULL;
    WORD bits = wave->wBitsPerSample;
    WORD tag = wave->wFormatTag;

    if (tag == WAVE_FORMAT_EXTENSIBLE && wave->cbSize >= 22) {
        const auto* extensible = reinterpret_cast<const WAVEFORMATEXTENSIBLE*>(wave);
        subformat = extensible->SubFormat;
        bits = extensible->Samples.wValidBitsPerSample != 0
                   ? extensible->Samples.wValidBitsPerSample
                   : wave->wBitsPerSample;
        tag =
            subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT ? WAVE_FORMAT_IEEE_FLOAT : WAVE_FORMAT_PCM;
    }

    if (tag == WAVE_FORMAT_IEEE_FLOAT && wave->wBitsPerSample == 32) {
        format.sample_format = SampleFormat::Float32;
    } else if (tag == WAVE_FORMAT_PCM && bits == 16 && wave->wBitsPerSample == 16) {
        format.sample_format = SampleFormat::Int16;
    } else {
        format.sample_format = SampleFormat::Unknown;
    }

    return format;
}

void fill_waveformat(const AudioFormat& format, WAVEFORMATEXTENSIBLE& wave) noexcept
{
    std::memset(&wave, 0, sizeof(wave));

    const WORD bits = static_cast<WORD>(bytes_per_sample(format.sample_format) * 8u);

    wave.Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
    wave.Format.nChannels = static_cast<WORD>(format.channels);
    wave.Format.nSamplesPerSec = static_cast<DWORD>(format.sample_rate);
    wave.Format.wBitsPerSample = bits;
    wave.Format.nBlockAlign = static_cast<WORD>(format.channels * (bits / 8u));
    wave.Format.nAvgBytesPerSec = wave.Format.nSamplesPerSec * wave.Format.nBlockAlign;
    wave.Format.cbSize = sizeof(WAVEFORMATEXTENSIBLE) - sizeof(WAVEFORMATEX);

    wave.Samples.wValidBitsPerSample = bits;
    wave.dwChannelMask =
        format.channels == 1 ? SPEAKER_FRONT_CENTER : (SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT);
    wave.SubFormat = format.sample_format == SampleFormat::Float32 ? KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
                                                                   : KSDATAFORMAT_SUBTYPE_PCM;
}

std::uint32_t windows_build_number() noexcept
{
    using RtlGetVersionFn = LONG(WINAPI*)(PRTL_OSVERSIONINFOW);

    static const std::uint32_t build = [] {
        const HMODULE ntdll = ::GetModuleHandleW(L"ntdll.dll");
        if (ntdll == nullptr) {
            return std::uint32_t{0};
        }
        const auto get_version = reinterpret_cast<RtlGetVersionFn>(
            reinterpret_cast<void*>(::GetProcAddress(ntdll, "RtlGetVersion")));
        if (get_version == nullptr) {
            return std::uint32_t{0};
        }
        RTL_OSVERSIONINFOW info{};
        info.dwOSVersionInfoSize = sizeof(info);
        if (get_version(&info) != 0) {
            return std::uint32_t{0};
        }
        return static_cast<std::uint32_t>(info.dwBuildNumber);
    }();

    return build;
}

std::uint32_t resolve_process_tree_root(std::uint32_t process_id) noexcept
{
    if (process_id == 0) {
        return 0;
    }

    std::uint32_t current = process_id;
    ProcessIdentity identity = identity_of(current);
    if (!identity.found) {
        return process_id;
    }

    for (std::uint32_t hop = 0; hop < kMaxAncestorHops; ++hop) {
        if (identity.parent_id == 0 || identity.parent_id == current) {
            break;
        }

        const ProcessIdentity parent = identity_of(identity.parent_id);
        if (!parent.found) {
            break;
        }
        if (::_wcsicmp(parent.image, identity.image) != 0) {
            break;
        }

        ULONGLONG child_created = 0;
        ULONGLONG parent_created = 0;
        if (!creation_time_of(current, child_created) ||
            !creation_time_of(identity.parent_id, parent_created) ||
            parent_created > child_created) {
            break;
        }

        current = identity.parent_id;
        identity = parent;
    }

    return current;
}

}  // namespace tl::audio

#endif
