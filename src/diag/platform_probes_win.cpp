#include "platform_probes.hpp"

#include <windows.h>

#include <audioclient.h>
#include <dxgi1_6.h>
#include <mmdeviceapi.h>
#include <wrl/client.h>

namespace tl::diag {
namespace {
using Microsoft::WRL::ComPtr;

constexpr std::uint32_t kProcessLoopbackBuild = 20348;
constexpr std::size_t kAdapterNameCapacity = 128;

struct ComScope {
    ComScope() noexcept { initialised = SUCCEEDED(CoInitializeEx(nullptr, COINIT_MULTITHREADED)); }

    ~ComScope()
    {
        if (initialised) {
            CoUninitialize();
        }
    }

    ComScope(const ComScope&) = delete;
    ComScope& operator=(const ComScope&) = delete;

    bool initialised = false;
};

[[nodiscard]] std::uint32_t windows_build_number() noexcept
{
    using RtlGetVersionFn = LONG(WINAPI*)(PRTL_OSVERSIONINFOW);

    const HMODULE module = GetModuleHandleW(L"ntdll.dll");
    if (module == nullptr) {
        return 0;
    }

    const auto get_version = reinterpret_cast<RtlGetVersionFn>(
        reinterpret_cast<void*>(GetProcAddress(module, "RtlGetVersion")));
    if (get_version == nullptr) {
        return 0;
    }

    RTL_OSVERSIONINFOW info{};
    info.dwOSVersionInfoSize = sizeof(info);
    if (get_version(&info) != 0) {
        return 0;
    }

    return info.dwBuildNumber;
}

void narrow_text(char* destination, std::size_t capacity, const wchar_t* source) noexcept
{
    if (destination == nullptr || capacity == 0) {
        return;
    }

    destination[0] = '\0';
    if (source == nullptr) {
        return;
    }

    const int written = WideCharToMultiByte(CP_UTF8, 0, source, -1, destination,
                                            static_cast<int>(capacity), nullptr, nullptr);
    if (written <= 0) {
        destination[0] = '\0';
    }
}
}  // namespace

void probe_system(ReportBuilder& builder) noexcept
{
    const std::uint32_t build = windows_build_number();

    if (build == 0) {
        builder.add_failure(DiagCategory::System, "windows version",
                            Error{Status::PlatformError, "RtlGetVersion did not answer"}, nullptr);
    } else {
        DiagItem* item = builder.add(DiagCategory::System, "windows version");
        if (item != nullptr) {
            item->available = true;
            item->status = Status::Ok;
            format_detail(item->detail, kDiagDetailCapacity, "build ", build);
        }
    }

    const HMODULE audio_core = LoadLibraryW(L"mmdevapi.dll");
    const bool has_activate =
        audio_core != nullptr && reinterpret_cast<void*>(GetProcAddress(
                                     audio_core, "ActivateAudioInterfaceAsync")) != nullptr;
    if (audio_core != nullptr) {
        FreeLibrary(audio_core);
    }

    if (build != 0 && build < kProcessLoopbackBuild) {
        DiagItem* item = builder.add(DiagCategory::System, "per process loopback");
        if (item != nullptr) {
            item->available = false;
            item->status = Status::NotSupported;
            write_text(item->detail, kDiagDetailCapacity, "needs windows build ");
            append_unsigned(item->detail, kDiagDetailCapacity, kProcessLoopbackBuild);
            append_text(item->detail, kDiagDetailCapacity, ", this machine reports ");
            append_unsigned(item->detail, kDiagDetailCapacity, build);
        }
        return;
    }

    if (!has_activate) {
        builder.add_failure(
            DiagCategory::System, "per process loopback",
            Error{Status::Unavailable, "ActivateAudioInterfaceAsync is missing from mmdevapi"},
            nullptr);
        return;
    }

    builder.add_ok(DiagCategory::System, "per process loopback",
                   "supported, audio can be captured per application");
}

void probe_graphics(ReportBuilder& builder) noexcept
{
    ComPtr<IDXGIFactory1> factory;
    const HRESULT created = CreateDXGIFactory1(IID_PPV_ARGS(&factory));
    if (FAILED(created)) {
        builder.add_failure(DiagCategory::Graphics, "adapters",
                            Error{Status::PlatformError, "CreateDXGIFactory1 failed",
                                  static_cast<std::int32_t>(created)},
                            nullptr);
        return;
    }

    UINT adapter_index = 0;
    ComPtr<IDXGIAdapter1> adapter;
    while (factory->EnumAdapters1(adapter_index, adapter.ReleaseAndGetAddressOf()) !=
           DXGI_ERROR_NOT_FOUND) {
        DXGI_ADAPTER_DESC1 description{};
        if (FAILED(adapter->GetDesc1(&description))) {
            ++adapter_index;
            continue;
        }

        char adapter_name[kAdapterNameCapacity] = {};
        narrow_text(adapter_name, kAdapterNameCapacity, description.Description);

        DiagItem* item = builder.add(DiagCategory::Graphics, adapter_name);
        if (item == nullptr) {
            return;
        }

        const bool software = (description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE) != 0;
        item->available = !software;
        item->status = software ? Status::NotSupported : Status::Ok;
        write_text(item->detail, kDiagDetailCapacity, software ? "software adapter, " : "");
        append_text(item->detail, kDiagDetailCapacity, "dedicated video memory ");
        append_unsigned(
            item->detail, kDiagDetailCapacity,
            static_cast<std::uint64_t>(description.DedicatedVideoMemory) / (1024ull * 1024ull));
        append_text(item->detail, kDiagDetailCapacity, " MiB");

        UINT output_index = 0;
        ComPtr<IDXGIOutput> output;
        while (adapter->EnumOutputs(output_index, output.ReleaseAndGetAddressOf()) !=
               DXGI_ERROR_NOT_FOUND) {
            DXGI_OUTPUT_DESC output_description{};
            if (SUCCEEDED(output->GetDesc(&output_description))) {
                char output_name[kDiagNameCapacity] = {};
                narrow_text(output_name, kDiagNameCapacity, output_description.DeviceName);

                DiagItem* entry = builder.add(DiagCategory::Graphics, output_name);
                if (entry == nullptr) {
                    return;
                }

                entry->available = output_description.AttachedToDesktop != FALSE;
                entry->status = entry->available ? Status::Ok : Status::Unavailable;
                write_text(entry->detail, kDiagDetailCapacity, "driven by ");
                append_text(entry->detail, kDiagDetailCapacity, adapter_name);
                append_text(entry->detail, kDiagDetailCapacity, ", ");
                append_unsigned(
                    entry->detail, kDiagDetailCapacity,
                    static_cast<std::uint64_t>(output_description.DesktopCoordinates.right -
                                               output_description.DesktopCoordinates.left));
                append_text(entry->detail, kDiagDetailCapacity, "x");
                append_unsigned(
                    entry->detail, kDiagDetailCapacity,
                    static_cast<std::uint64_t>(output_description.DesktopCoordinates.bottom -
                                               output_description.DesktopCoordinates.top));
            }
            ++output_index;
        }

        ++adapter_index;
    }

    if (adapter_index == 0) {
        builder.add_failure(DiagCategory::Graphics, "adapters",
                            Error{Status::NotFound, "no dxgi adapter was enumerated"}, nullptr);
    }
}

void probe_audio(ReportBuilder& builder) noexcept
{
    ComScope com;

    ComPtr<IMMDeviceEnumerator> enumerator;
    HRESULT created = CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                                       IID_PPV_ARGS(&enumerator));
    if (FAILED(created)) {
        builder.add_failure(DiagCategory::Audio, "default render device",
                            Error{Status::PlatformError, "MMDeviceEnumerator could not be created",
                                  static_cast<std::int32_t>(created)},
                            nullptr);
        return;
    }

    ComPtr<IMMDevice> device;
    created = enumerator->GetDefaultAudioEndpoint(eRender, eConsole, &device);
    if (FAILED(created)) {
        builder.add_failure(DiagCategory::Audio, "default render device",
                            Error{Status::NotFound, "no default render endpoint",
                                  static_cast<std::int32_t>(created)},
                            nullptr);
        return;
    }

    ComPtr<IAudioClient> client;
    created = device->Activate(__uuidof(IAudioClient), CLSCTX_ALL, nullptr, &client);
    if (FAILED(created)) {
        builder.add_failure(DiagCategory::Audio, "default render device",
                            Error{Status::PlatformError, "IAudioClient could not be activated",
                                  static_cast<std::int32_t>(created)},
                            nullptr);
        return;
    }

    WAVEFORMATEX* format = nullptr;
    created = client->GetMixFormat(&format);
    if (FAILED(created) || format == nullptr) {
        builder.add_failure(
            DiagCategory::Audio, "default render device",
            Error{Status::PlatformError, "GetMixFormat failed", static_cast<std::int32_t>(created)},
            nullptr);
        return;
    }

    DiagItem* item = builder.add(DiagCategory::Audio, "default render device");
    if (item != nullptr) {
        item->available = true;
        item->status = Status::Ok;
        write_text(item->detail, kDiagDetailCapacity, "");
        append_unsigned(item->detail, kDiagDetailCapacity, format->nSamplesPerSec);
        append_text(item->detail, kDiagDetailCapacity, " Hz, ");
        append_unsigned(item->detail, kDiagDetailCapacity, format->nChannels);
        append_text(item->detail, kDiagDetailCapacity, " channels, ");
        append_unsigned(item->detail, kDiagDetailCapacity, format->wBitsPerSample);
        append_text(item->detail, kDiagDetailCapacity, " bit");
    }

    CoTaskMemFree(format);
}
}  // namespace tl::diag
