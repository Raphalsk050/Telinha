#include "media_foundation_source.hpp"

#include <atomic>
#include <cstddef>
#include <cstring>
#include <mutex>
#include <new>
#include <utility>

#include "../dirty_region_builder.hpp"
#include "d3d11_capture_device.hpp"
#include "telinha/core/arena.hpp"
#include "telinha/core/clock.hpp"
#include "telinha/core/log.hpp"

#if defined(_MSC_VER)
#pragma warning(push)
#pragma warning(disable : 4201)
#endif

#include <cfgmgr32.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mfreadwrite.h>

#if defined(_MSC_VER)
#pragma warning(pop)
#endif

namespace tl::capture::win {
namespace {

inline constexpr std::size_t kArenaSlackBytes = 64u * 1024u;
inline constexpr std::uint32_t kRingCapacity = 3;
inline constexpr std::uint32_t kFirstFrameWaitMs = 2500;
inline constexpr std::uint32_t kMaxNativeFormats = 160;
inline constexpr std::uint32_t kMaxFormatAttempts = 8;
inline constexpr std::uint64_t kSmoothMillihertz = 49500;
inline constexpr std::uint64_t kPreferredMillihertz = 60000;
inline constexpr std::uint64_t kPreferredArea = 1920ull * 1080ull;
// The desktop app reads handles as JavaScript numbers, exact only up to 53 bits.
inline constexpr std::uint64_t kHandleMask = (1ull << 53) - 1;
inline constexpr DWORD kVideoStream = static_cast<DWORD>(MF_SOURCE_READER_FIRST_VIDEO_STREAM);

const DEVPROPKEY kInstanceIdKey = {
    {0x78c34fc8, 0x104a, 0x4aca, {0x9e, 0xa4, 0x52, 0x4d, 0x52, 0x99, 0x6e, 0x57}}, 256};
const DEVPROPKEY kContainerIdKey = {
    {0x8c7ed206, 0x3f8a, 0x4827, {0xb3, 0xab, 0xae, 0x9e, 0x1f, 0xae, 0xfc, 0x6c}}, 2};

[[nodiscard]] std::uint64_t handle_for_link(const wchar_t* link) noexcept
{
    std::uint64_t hash = 14695981039346656037ull;
    for (const wchar_t* cursor = link; *cursor != L'\0'; ++cursor) {
        wchar_t symbol = *cursor;
        if (symbol >= L'A' && symbol <= L'Z') {
            symbol = static_cast<wchar_t>(symbol - L'A' + L'a');
        }
        hash ^= static_cast<std::uint64_t>(symbol);
        hash *= 1099511628211ull;
    }
    hash &= kHandleMask;
    return hash == 0 ? 1 : hash;
}

void read_container_id(const wchar_t* link, std::uint8_t* out) noexcept
{
    wchar_t instance[MAX_DEVICE_ID_LEN] = {};
    ULONG size = static_cast<ULONG>(sizeof(instance));
    DEVPROPTYPE type = DEVPROP_TYPE_EMPTY;
    if (CM_Get_Device_Interface_PropertyW(link, &kInstanceIdKey, &type,
                                          reinterpret_cast<PBYTE>(instance), &size,
                                          0) != CR_SUCCESS ||
        type != DEVPROP_TYPE_STRING) {
        return;
    }

    DEVINST node = 0;
    if (CM_Locate_DevNodeW(&node, instance, CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS) {
        return;
    }

    GUID container = {};
    size = static_cast<ULONG>(sizeof(container));
    if (CM_Get_DevNode_PropertyW(node, &kContainerIdKey, &type, reinterpret_cast<PBYTE>(&container),
                                 &size, 0) != CR_SUCCESS ||
        type != DEVPROP_TYPE_GUID) {
        return;
    }
    std::memcpy(out, &container, sizeof(container));
}

[[nodiscard]] bool device_busy(HRESULT failure) noexcept
{
    if (failure == HRESULT_FROM_WIN32(ERROR_SHARING_VIOLATION) ||
        failure == HRESULT_FROM_WIN32(ERROR_BUSY)) {
        return true;
    }
#if defined(MF_E_HW_MFT_FAILED_START_STREAMING)
    if (failure == MF_E_HW_MFT_FAILED_START_STREAMING) {
        return true;
    }
#endif
#if defined(MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED)
    if (failure == MF_E_VIDEO_RECORDING_DEVICE_PREEMPTED) {
        return true;
    }
#endif
    return false;
}

[[nodiscard]] Outcome device_failure(HRESULT failure, const char* context) noexcept
{
    if (device_busy(failure)) {
        return fail(Status::Unavailable,
                    "a placa de captura esta em uso por outro programa, feche o OBS ou a camera",
                    static_cast<std::int32_t>(failure));
    }
    return fail(Status::DeviceLost, context, static_cast<std::int32_t>(failure));
}

class MediaFoundationScope {
public:
    MediaFoundationScope() noexcept = default;
    ~MediaFoundationScope() { close(); }

    MediaFoundationScope(const MediaFoundationScope&) = delete;
    MediaFoundationScope& operator=(const MediaFoundationScope&) = delete;

    [[nodiscard]] Outcome open() noexcept
    {
        if (started_) {
            return ok();
        }
        const HRESULT com = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
        if (SUCCEEDED(com)) {
            owns_com_ = true;
            com_thread_ = GetCurrentThreadId();
        } else if (com != RPC_E_CHANGED_MODE) {
            return outcome_from_hresult(com, "CoInitializeEx");
        }

        const HRESULT hr = MFStartup(MF_VERSION, MFSTARTUP_LITE);
        if (FAILED(hr)) {
            close();
            return outcome_from_hresult(hr, "MFStartup");
        }
        started_ = true;
        return ok();
    }

    void close() noexcept
    {
        if (started_) {
            MFShutdown();
            started_ = false;
        }
        if (owns_com_ && com_thread_ == GetCurrentThreadId()) {
            CoUninitialize();
        }
        owns_com_ = false;
    }

private:
    DWORD com_thread_ = 0;
    bool owns_com_ = false;
    bool started_ = false;
};

class CoString {
public:
    CoString() noexcept = default;
    ~CoString() { CoTaskMemFree(text_); }

    CoString(const CoString&) = delete;
    CoString& operator=(const CoString&) = delete;

    [[nodiscard]] LPWSTR* put() noexcept
    {
        CoTaskMemFree(text_);
        text_ = nullptr;
        return &text_;
    }

    [[nodiscard]] const wchar_t* get() const noexcept { return text_; }

private:
    LPWSTR text_ = nullptr;
};

class DeviceList {
public:
    DeviceList() noexcept = default;

    ~DeviceList()
    {
        for (UINT32 index = 0; index < count_; ++index) {
            if (items_[index] != nullptr) {
                items_[index]->Release();
            }
        }
        CoTaskMemFree(items_);
    }

    DeviceList(const DeviceList&) = delete;
    DeviceList& operator=(const DeviceList&) = delete;

    [[nodiscard]] Outcome load() noexcept
    {
        ComPtr<IMFAttributes> attributes;
        HRESULT hr = MFCreateAttributes(&attributes, 1);
        if (SUCCEEDED(hr)) {
            hr = attributes->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                                     MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
        }
        if (SUCCEEDED(hr)) {
            hr = MFEnumDeviceSources(attributes.Get(), &items_, &count_);
        }
        if (FAILED(hr)) {
            return outcome_from_hresult(hr, "MFEnumDeviceSources");
        }
        return ok();
    }

    [[nodiscard]] UINT32 count() const noexcept { return count_; }
    [[nodiscard]] IMFActivate* at(UINT32 index) const noexcept { return items_[index]; }

private:
    IMFActivate** items_ = nullptr;
    UINT32 count_ = 0;
};

[[nodiscard]] bool describe_device(IMFActivate* activate, CaptureTargetInfo& out) noexcept
{
    CoString link;
    UINT32 length = 0;
    if (FAILED(activate->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
                                            link.put(), &length)) ||
        link.get() == nullptr) {
        return false;
    }

    out = CaptureTargetInfo{};
    out.target = CaptureTarget::device(handle_for_link(link.get()));

    CoString name;
    if (SUCCEEDED(activate->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, name.put(),
                                               &length)) &&
        name.get() != nullptr) {
        static_cast<void>(utf16_to_utf8(name.get(), out.name, kTargetNameCapacity));
    }
    read_container_id(link.get(), out.container_id);
    return true;
}

// The reader delivers samples on Media Foundation threads; only the newest one is kept.
class ReaderCallback final : public IMFSourceReaderCallback {
public:
    ReaderCallback() noexcept : ready_(CreateEventW(nullptr, FALSE, FALSE, nullptr)) {}

    ~ReaderCallback()
    {
        if (ready_ != nullptr) {
            CloseHandle(ready_);
        }
    }

    ReaderCallback(const ReaderCallback&) = delete;
    ReaderCallback& operator=(const ReaderCallback&) = delete;

    [[nodiscard]] bool valid() const noexcept { return ready_ != nullptr; }
    [[nodiscard]] HANDLE ready() const noexcept { return ready_; }

    void attach(IMFSourceReader* reader) noexcept
    {
        const std::lock_guard<std::mutex> guard(mutex_);
        reader_ = reader;
        stopping_ = false;
    }

    void detach() noexcept
    {
        ComPtr<IMFSourceReader> reader;
        ComPtr<IMFSample> sample;
        {
            const std::lock_guard<std::mutex> guard(mutex_);
            stopping_ = true;
            reader = std::move(reader_);
            sample = std::move(latest_);
        }
    }

    [[nodiscard]] HRESULT request() noexcept
    {
        const std::lock_guard<std::mutex> guard(mutex_);
        if (stopping_ || reader_ == nullptr) {
            return MF_E_SHUTDOWN;
        }
        return reader_->ReadSample(kVideoStream, 0, nullptr, nullptr, nullptr, nullptr);
    }

    [[nodiscard]] ComPtr<IMFSample> take(HRESULT& failure, bool& format_changed) noexcept
    {
        const std::lock_guard<std::mutex> guard(mutex_);
        failure = failure_;
        format_changed = format_changed_;
        format_changed_ = false;
        ComPtr<IMFSample> sample = std::move(latest_);
        return sample;
    }

    STDMETHODIMP QueryInterface(REFIID riid, void** object) override
    {
        if (object == nullptr) {
            return E_POINTER;
        }
        if (riid == __uuidof(IUnknown) || riid == __uuidof(IMFSourceReaderCallback)) {
            *object = static_cast<IMFSourceReaderCallback*>(this);
            AddRef();
            return S_OK;
        }
        *object = nullptr;
        return E_NOINTERFACE;
    }

    STDMETHODIMP_(ULONG) AddRef() override
    {
        return static_cast<ULONG>(references_.fetch_add(1, std::memory_order_relaxed) + 1);
    }

    STDMETHODIMP_(ULONG) Release() override
    {
        const long remaining = references_.fetch_sub(1, std::memory_order_acq_rel) - 1;
        if (remaining == 0) {
            delete this;
        }
        return static_cast<ULONG>(remaining);
    }

    STDMETHODIMP OnReadSample(HRESULT status, DWORD, DWORD flags, LONGLONG,
                              IMFSample* sample) override
    {
        {
            const std::lock_guard<std::mutex> guard(mutex_);
            if (stopping_) {
                return S_OK;
            }
            if (FAILED(status)) {
                failure_ = status;
            } else if ((flags & MF_SOURCE_READERF_ERROR) != 0) {
                failure_ = E_FAIL;
            } else if ((flags & MF_SOURCE_READERF_ENDOFSTREAM) != 0) {
                failure_ = MF_E_END_OF_STREAM;
            }
            if ((flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED) != 0) {
                format_changed_ = true;
            }
            if (sample != nullptr) {
                latest_ = sample;
            }
            if (SUCCEEDED(failure_) && reader_ != nullptr) {
                const HRESULT requested =
                    reader_->ReadSample(kVideoStream, 0, nullptr, nullptr, nullptr, nullptr);
                if (FAILED(requested)) {
                    failure_ = requested;
                }
            }
        }
        SetEvent(ready_);
        return S_OK;
    }

    STDMETHODIMP OnFlush(DWORD) override { return S_OK; }
    STDMETHODIMP OnEvent(DWORD, IMFMediaEvent*) override { return S_OK; }

private:
    std::atomic<long> references_{1};
    std::mutex mutex_;
    HANDLE ready_ = nullptr;
    ComPtr<IMFSourceReader> reader_;
    ComPtr<IMFSample> latest_;
    HRESULT failure_ = S_OK;
    bool format_changed_ = false;
    bool stopping_ = false;
};

struct NativeFormat {
    DWORD index = 0;
    UINT32 width = 0;
    UINT32 height = 0;
    UINT32 rate_numerator = 0;
    UINT32 rate_denominator = 0;
    std::uint64_t millihertz = 0;
    bool compressed = false;
    bool tried = false;
};

// Smooth motion first, then the largest picture up to 1080p, then 60 Hz, then no decoding.
[[nodiscard]] bool preferred(const NativeFormat& candidate, const NativeFormat& best) noexcept
{
    const bool candidate_smooth = candidate.millihertz >= kSmoothMillihertz;
    const bool best_smooth = best.millihertz >= kSmoothMillihertz;
    if (candidate_smooth != best_smooth) {
        return candidate_smooth;
    }

    const std::uint64_t candidate_area = std::uint64_t{candidate.width} * candidate.height;
    const std::uint64_t best_area = std::uint64_t{best.width} * best.height;
    const bool candidate_fits = candidate_area <= kPreferredArea;
    const bool best_fits = best_area <= kPreferredArea;
    if (candidate_fits != best_fits) {
        return candidate_fits;
    }
    if (candidate_area != best_area) {
        return candidate_fits ? candidate_area > best_area : candidate_area < best_area;
    }

    const std::uint64_t candidate_rate =
        candidate.millihertz > kPreferredMillihertz ? kPreferredMillihertz : candidate.millihertz;
    const std::uint64_t best_rate =
        best.millihertz > kPreferredMillihertz ? kPreferredMillihertz : best.millihertz;
    if (candidate_rate != best_rate) {
        return candidate_rate > best_rate;
    }
    if (candidate.millihertz != best.millihertz) {
        return candidate.millihertz < best.millihertz;
    }
    return !candidate.compressed && best.compressed;
}

class MediaFoundationSource final : public CaptureSource {
public:
    MediaFoundationSource(const CaptureTarget& target, const CaptureOptions& options) noexcept
        : target_(target), options_(options)
    {}

    ~MediaFoundationSource() override { stop(); }

    Outcome start() noexcept override
    {
        if (started_) {
            return ok();
        }
        if (target_.kind != CaptureTargetKind::Device || target_.handle == 0) {
            return fail(Status::InvalidArgument, "media foundation capture needs a device target");
        }

        const Outcome prepared = prepare();
        if (!prepared.ok()) {
            stop();
            return prepared;
        }
        started_ = true;
        return ok();
    }

    void stop() noexcept override
    {
        release();
        if (callback_ != nullptr) {
            callback_->detach();
        }
        reader_.Reset();
        if (media_source_ != nullptr) {
            media_source_->Shutdown();
            media_source_.Reset();
        }
        if (callback_ != nullptr) {
            callback_->Release();
            callback_ = nullptr;
        }

        ring_.destroy();
        ring_ready_ = false;
        device_.destroy();
        scratch_.reset();
        scratch_bytes_ = 0;
        media_.close();
        started_ = false;
    }

    [[nodiscard]] Outcome acquire(CapturedFrame& out, std::uint32_t timeout_ms) noexcept override
    {
        if (!started_) {
            return fail(Status::Unavailable, "media foundation capture not started");
        }
        if (leased_) {
            return fail(Status::AlreadyExists, "frame already leased");
        }

        HRESULT failure = S_OK;
        bool format_changed = false;
        ComPtr<IMFSample> sample = callback_->take(failure, format_changed);
        if (sample == nullptr && SUCCEEDED(failure) && !format_changed) {
            const DWORD waited = WaitForSingleObject(callback_->ready(), timeout_ms);
            if (waited == WAIT_TIMEOUT) {
                return fail(Status::Timeout, "no frame from the capture device");
            }
            if (waited != WAIT_OBJECT_0) {
                return fail(Status::Unavailable, "capture device wait failed");
            }
            sample = callback_->take(failure, format_changed);
        }

        if (FAILED(failure)) {
            // Nothing else arrives after a failure; wait out the timeout so the caller does not
            // spin.
            WaitForSingleObject(callback_->ready(), timeout_ms);
            return device_failure(failure, "a placa de captura parou de mandar imagem");
        }
        if (format_changed) {
            TL_TRY(adopt_current_format());
            return fail(Status::ConfigurationChanged, "capture device format changed");
        }
        if (sample == nullptr) {
            return fail(Status::Timeout, "no frame from the capture device");
        }
        return upload(sample.Get(), out);
    }

    void release() noexcept override
    {
        if (leased_) {
            ring_.give_back(leased_handle_);
        }
        leased_ = false;
        leased_handle_ = TextureHandle{};
    }

    [[nodiscard]] CaptureSourceInfo info() const noexcept override
    {
        CaptureSourceInfo out;
        out.target = target_;
        out.backend = CaptureBackend::MediaFoundation;
        out.width = layout_.width;
        out.height = layout_.height;
        out.format = layout_.format;
        out.rotation = SurfaceRotation::None;
        out.refresh_millihertz = refresh_millihertz_;
        out.process_id = 0;
        out.native_device = device_.device();
        return out;
    }

private:
    Outcome prepare() noexcept
    {
        TL_TRY(media_.open());
        TL_TRY(open_device());

        callback_ = new (std::nothrow) ReaderCallback();
        if (callback_ == nullptr) {
            return fail(Status::OutOfMemory, "capture device callback");
        }
        if (!callback_->valid()) {
            return fail(Status::Unavailable, "capture device event");
        }

        TL_TRY(create_reader());
        TL_TRY(choose_format());
        TL_TRY(device_.create_default());
        TL_TRY(reserve_storage());

        callback_->attach(reader_.Get());
        const HRESULT requested = callback_->request();
        if (FAILED(requested)) {
            return device_failure(requested, "a placa de captura nao comecou a mandar imagem");
        }
        return await_first_frame();
    }

    Outcome open_device() noexcept
    {
        DeviceList devices;
        TL_TRY(devices.load());

        for (UINT32 index = 0; index < devices.count(); ++index) {
            CaptureTargetInfo description;
            if (!describe_device(devices.at(index), description) ||
                description.target.handle != target_.handle) {
                continue;
            }

            std::memcpy(name_, description.name, sizeof(name_));
            const HRESULT hr = devices.at(index)->ActivateObject(IID_PPV_ARGS(&media_source_));
            if (FAILED(hr)) {
                return device_failure(hr, "nao consegui abrir a placa de captura");
            }
            return ok();
        }
        return fail(Status::TargetGone, "a placa de captura nao esta conectada");
    }

    Outcome create_reader() noexcept
    {
        ComPtr<IMFAttributes> attributes;
        HRESULT hr = MFCreateAttributes(&attributes, 3);
        if (SUCCEEDED(hr)) {
            hr = attributes->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK, callback_);
        }
        if (SUCCEEDED(hr)) {
            hr = attributes->SetUINT32(MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, TRUE);
        }
        if (SUCCEEDED(hr)) {
            hr = attributes->SetUINT32(MF_LOW_LATENCY, TRUE);
        }
        if (FAILED(hr)) {
            return outcome_from_hresult(hr, "source reader attributes");
        }

        hr = MFCreateSourceReaderFromMediaSource(media_source_.Get(), attributes.Get(), &reader_);
        if (FAILED(hr)) {
            return device_failure(hr, "nao consegui ler a placa de captura");
        }
        return ok();
    }

    Outcome choose_format() noexcept
    {
        NativeFormat formats[kMaxNativeFormats];
        std::uint32_t count = 0;

        for (DWORD index = 0; count < kMaxNativeFormats; ++index) {
            ComPtr<IMFMediaType> native;
            if (FAILED(reader_->GetNativeMediaType(kVideoStream, index, &native))) {
                break;
            }

            NativeFormat format;
            format.index = index;
            GUID subtype = GUID_NULL;
            if (FAILED(native->GetGUID(MF_MT_SUBTYPE, &subtype)) ||
                FAILED(MFGetAttributeSize(native.Get(), MF_MT_FRAME_SIZE, &format.width,
                                          &format.height)) ||
                format.width == 0 || format.height == 0) {
                continue;
            }
            if (FAILED(MFGetAttributeRatio(native.Get(), MF_MT_FRAME_RATE, &format.rate_numerator,
                                           &format.rate_denominator)) ||
                format.rate_denominator == 0) {
                format.rate_numerator = 0;
                format.rate_denominator = 0;
            } else {
                format.millihertz =
                    std::uint64_t{format.rate_numerator} * 1000u / format.rate_denominator;
            }
            format.compressed = subtype == MFVideoFormat_MJPG || subtype == MFVideoFormat_H264 ||
                                subtype == MFVideoFormat_HEVC;
            formats[count] = format;
            ++count;
        }

        for (std::uint32_t attempt = 0; attempt < kMaxFormatAttempts; ++attempt) {
            NativeFormat* best = nullptr;
            for (std::uint32_t index = 0; index < count; ++index) {
                if (!formats[index].tried &&
                    (best == nullptr || preferred(formats[index], *best))) {
                    best = &formats[index];
                }
            }
            if (best == nullptr) {
                break;
            }
            best->tried = true;
            if (apply_format(*best).ok()) {
                TL_LOG_INFO("captura: %s em %ux%u a %.2f quadros por segundo%s", name_, best->width,
                            best->height, static_cast<double>(best->millihertz) / 1000.0,
                            best->compressed ? ", comprimido pela placa" : "");
                return adopt_current_format();
            }
        }

        TL_TRY(request_rgb32(0, 0, 0, 0));
        return adopt_current_format();
    }

    Outcome apply_format(const NativeFormat& format) noexcept
    {
        ComPtr<IMFMediaType> native;
        HRESULT hr = reader_->GetNativeMediaType(kVideoStream, format.index, &native);
        if (SUCCEEDED(hr)) {
            hr = reader_->SetCurrentMediaType(kVideoStream, nullptr, native.Get());
        }
        if (FAILED(hr)) {
            return outcome_from_hresult(hr, "SetCurrentMediaType native");
        }
        return request_rgb32(format.width, format.height, format.rate_numerator,
                             format.rate_denominator);
    }

    Outcome request_rgb32(UINT32 width, UINT32 height, UINT32 numerator,
                          UINT32 denominator) noexcept
    {
        ComPtr<IMFMediaType> output;
        HRESULT hr = MFCreateMediaType(&output);
        if (SUCCEEDED(hr)) {
            hr = output->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
        }
        if (SUCCEEDED(hr)) {
            hr = output->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_RGB32);
        }
        if (SUCCEEDED(hr) && width != 0 && height != 0) {
            hr = MFSetAttributeSize(output.Get(), MF_MT_FRAME_SIZE, width, height);
            if (SUCCEEDED(hr)) {
                hr = output->SetUINT32(MF_MT_DEFAULT_STRIDE, width * 4u);
            }
        }
        if (SUCCEEDED(hr) && numerator != 0 && denominator != 0) {
            hr = MFSetAttributeRatio(output.Get(), MF_MT_FRAME_RATE, numerator, denominator);
        }
        if (SUCCEEDED(hr)) {
            hr = reader_->SetCurrentMediaType(kVideoStream, nullptr, output.Get());
        }
        if (FAILED(hr)) {
            return outcome_from_hresult(hr, "SetCurrentMediaType RGB32");
        }
        return ok();
    }

    Outcome adopt_current_format() noexcept
    {
        ComPtr<IMFMediaType> current;
        HRESULT hr = reader_->GetCurrentMediaType(kVideoStream, &current);
        if (FAILED(hr)) {
            return outcome_from_hresult(hr, "GetCurrentMediaType");
        }

        UINT32 width = 0;
        UINT32 height = 0;
        hr = MFGetAttributeSize(current.Get(), MF_MT_FRAME_SIZE, &width, &height);
        if (FAILED(hr) || width == 0 || height == 0) {
            return fail(Status::NotSupported,
                        "a placa de captura nao informou o tamanho da imagem");
        }

        UINT32 stride = 0;
        if (SUCCEEDED(current->GetUINT32(MF_MT_DEFAULT_STRIDE, &stride))) {
            stride_ = static_cast<LONG>(static_cast<INT32>(stride));
        } else {
            LONG computed = 0;
            stride_ = SUCCEEDED(MFGetStrideForBitmapInfoHeader(MFVideoFormat_RGB32.Data1, width,
                                                               &computed))
                          ? computed
                          : -static_cast<LONG>(width * 4u);
        }

        UINT32 numerator = 0;
        UINT32 denominator = 0;
        refresh_millihertz_ =
            SUCCEEDED(
                MFGetAttributeRatio(current.Get(), MF_MT_FRAME_RATE, &numerator, &denominator)) &&
                    denominator != 0
                ? static_cast<std::uint32_t>(std::uint64_t{numerator} * 1000u / denominator)
                : 0;

        SurfaceLayout layout;
        layout.width = width;
        layout.height = height;
        layout.format = PixelFormat::B8G8R8A8Unorm;
        if (layout != layout_) {
            layout_ = layout;
            if (ring_ready_) {
                ring_.set_layout(layout_);
                dirty_.resize(layout_.width, layout_.height);
            }
        }
        return ok();
    }

    Outcome reserve_storage() noexcept
    {
        const std::size_t dirty_bytes =
            static_cast<std::size_t>(options_.max_dirty_rects) * (sizeof(Rect) + 4 * sizeof(int)) +
            static_cast<std::size_t>(options_.max_move_rects) * sizeof(MoveRect);

        if (!storage_.reserve(dirty_bytes + kArenaSlackBytes)) {
            return fail(Status::OutOfMemory, "capture device arena");
        }

        LinearArena& arena = storage_.arena();
        DirtyRegionBuilder::Limits limits;
        limits.max_dirty_rects = options_.max_dirty_rects;
        limits.max_move_rects = options_.max_move_rects;
        if (!dirty_.initialize(arena, layout_.width, layout_.height, limits)) {
            return fail(Status::OutOfMemory, "capture device dirty region");
        }

        TL_TRY(ring_.initialize(arena, device_.device(), kRingCapacity));
        ring_.set_layout(layout_);
        arena.update_high_water();
        ring_ready_ = true;
        return ok();
    }

    Outcome await_first_frame() noexcept
    {
        if (WaitForSingleObject(callback_->ready(), kFirstFrameWaitMs) != WAIT_OBJECT_0) {
            TL_LOG_WARN("captura: %s ainda nao mandou imagem, confira se o aparelho esta ligado",
                        name_);
            return ok();
        }

        HRESULT failure = S_OK;
        bool format_changed = false;
        static_cast<void>(callback_->take(failure, format_changed));
        if (FAILED(failure)) {
            return device_failure(failure, "a placa de captura nao mandou imagem");
        }
        if (format_changed) {
            TL_TRY(adopt_current_format());
        }
        return ok();
    }

    Outcome upload(IMFSample* sample, CapturedFrame& out) noexcept
    {
        ComPtr<IMFMediaBuffer> buffer;
        HRESULT hr = sample->ConvertToContiguousBuffer(&buffer);
        if (FAILED(hr)) {
            return outcome_from_hresult(hr, "IMFSample::ConvertToContiguousBuffer");
        }

        ComPtr<IMF2DBuffer> planar;
        const bool two_dimensional = SUCCEEDED(buffer.As(&planar));
        BYTE* first = nullptr;
        LONG stride = stride_;
        DWORD length = 0;
        hr = two_dimensional ? planar->Lock2D(&first, &stride)
                             : buffer->Lock(&first, nullptr, &length);
        if (FAILED(hr)) {
            return outcome_from_hresult(hr, "lock capture sample");
        }

        const std::size_t row_bytes = std::size_t{layout_.width} * 4u;
        const std::size_t pitch = stride < 0
                                      ? static_cast<std::size_t>(-static_cast<std::int64_t>(stride))
                                      : static_cast<std::size_t>(stride);
        Outcome copied = ok();
        if (pitch < row_bytes || layout_.height == 0) {
            copied = fail(Status::OutOfRange, "capture sample stride is too small");
        } else if (!two_dimensional && length < pitch * (layout_.height - 1u) + row_bytes) {
            copied = fail(Status::OutOfRange, "capture sample is too small");
        } else {
            // A locked bottom-up buffer starts at the last row; Lock2D already points at the top.
            const BYTE* top =
                !two_dimensional && stride < 0 ? first + pitch * (layout_.height - 1u) : first;
            copied = copy_to_texture(top, stride);
        }

        if (two_dimensional) {
            planar->Unlock2D();
        } else {
            buffer->Unlock();
        }
        if (!copied.ok()) {
            return copied;
        }

        dirty_.begin_frame();
        dirty_.force_full_surface_without_metadata();
        dirty_.finish();

        const Nanoseconds now = now_ns();
        out = CapturedFrame{};
        out.surface.memory = SurfaceMemory::GpuTexture;
        out.surface.gpu_texture = destination_;
        out.surface.width = layout_.width;
        out.surface.height = layout_.height;
        out.surface.format = layout_.format;
        out.surface.rotation = SurfaceRotation::None;

        out.metadata.frame_index = frame_index_;
        out.metadata.present_time_ns = now;
        out.metadata.acquire_time_ns = now;
        out.metadata.accumulated_frames = 1;
        out.metadata.content_changed = true;
        out.metadata.cursor_changed = false;
        out.metadata.full_surface_dirty = true;
        out.metadata.dirty_metadata_available = false;

        out.dirty_rects = dirty_.dirty_rects();
        out.move_rects = dirty_.move_rects();

        ++frame_index_;
        return ok();
    }

    Outcome copy_to_texture(const BYTE* top, LONG stride) noexcept
    {
        ID3D11Texture2D* destination = nullptr;
        TextureHandle handle;
        TL_TRY(ring_.acquire(destination, handle));

        const UINT row_bytes = layout_.width * 4u;
        if (stride > 0) {
            device_.context()->UpdateSubresource(destination, 0, nullptr, top,
                                                 static_cast<UINT>(stride), 0);
        } else {
            const std::size_t needed = std::size_t{row_bytes} * layout_.height;
            if (scratch_bytes_ < needed) {
                scratch_.reset(new (std::nothrow) std::byte[needed]);
                scratch_bytes_ = scratch_ != nullptr ? needed : 0;
            }
            if (scratch_ == nullptr) {
                ring_.give_back(handle);
                return fail(Status::OutOfMemory, "capture device scratch");
            }

            const std::ptrdiff_t step = stride;
            for (std::uint32_t row = 0; row < layout_.height; ++row) {
                std::memcpy(scratch_.get() + std::size_t{row_bytes} * row,
                            top + step * static_cast<std::ptrdiff_t>(row), row_bytes);
            }
            device_.context()->UpdateSubresource(destination, 0, nullptr, scratch_.get(), row_bytes,
                                                 0);
        }

        destination_ = destination;
        leased_handle_ = handle;
        leased_ = true;
        return ok();
    }

    CaptureTarget target_;
    CaptureOptions options_;
    char name_[kTargetNameCapacity] = {};

    MediaFoundationScope media_;
    D3D11CaptureDevice device_;
    D3D11TextureRing ring_;
    ArenaStorage storage_;
    DirtyRegionBuilder dirty_;

    ReaderCallback* callback_ = nullptr;
    ComPtr<IMFMediaSource> media_source_;
    ComPtr<IMFSourceReader> reader_;

    std::unique_ptr<std::byte[]> scratch_;
    std::size_t scratch_bytes_ = 0;
    SurfaceLayout layout_;
    ID3D11Texture2D* destination_ = nullptr;
    TextureHandle leased_handle_;
    LONG stride_ = 0;
    std::uint64_t frame_index_ = 0;
    std::uint32_t refresh_millihertz_ = 0;
    bool ring_ready_ = false;
    bool leased_ = false;
    bool started_ = false;
};

}  // namespace

Outcome enumerate_capture_devices(Span<CaptureTargetInfo> out, std::uint32_t& written,
                                  std::uint32_t& available) noexcept
{
    written = 0;
    available = 0;

    MediaFoundationScope media;
    TL_TRY(media.open());

    DeviceList devices;
    TL_TRY(devices.load());

    for (UINT32 index = 0; index < devices.count(); ++index) {
        CaptureTargetInfo info;
        if (!describe_device(devices.at(index), info)) {
            continue;
        }
        ++available;
        if (written < out.size()) {
            out[written] = info;
            ++written;
        }
    }

    if (available > written) {
        return fail(Status::OutOfRange, "enumerate_capture_devices: buffer too small");
    }
    return ok();
}

Result<std::unique_ptr<CaptureSource>> create_media_foundation_source(
    const CaptureTarget& target, const CaptureOptions& options) noexcept
{
    auto* source = new (std::nothrow) MediaFoundationSource(target, options);
    if (source == nullptr) {
        return Error{Status::OutOfMemory, "MediaFoundationSource"};
    }
    return std::unique_ptr<CaptureSource>(source);
}

}  // namespace tl::capture::win
