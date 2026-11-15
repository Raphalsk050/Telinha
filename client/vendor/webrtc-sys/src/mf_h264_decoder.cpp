// H264 decoding through the Windows Media Foundation decoder.
//
// Each decoder owns one instance of Microsoft's H264 decoder transform. Frames
// go in as Annex B, come out as NV12, and are copied into I420 for libwebrtc.
// The RTP timestamp rides in the sample time so each picture keeps its own.

#ifdef _WIN32

#include "livekit/mf_h264_decoder.h"

#include <windows.h>

#include <codecapi.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mftransform.h>
#include <wmcodecdsp.h>
#include <wrl/client.h>

#include <mutex>

#include "api/video/i420_buffer.h"
#include "api/video/video_frame.h"
#include "api/video_codecs/h264_profile_level_id.h"
#include "common_video/include/video_frame_buffer_pool.h"
#include "modules/video_coding/codecs/h264/include/h264.h"
#include "modules/video_coding/include/video_error_codes.h"
#include "rtc_base/logging.h"
#include "third_party/libyuv/include/libyuv/convert.h"

using Microsoft::WRL::ComPtr;

namespace livekit_ffi {

namespace {

bool StartMediaFoundation() {
  static std::once_flag once;
  static bool started = false;
  std::call_once(once, [] {
    started = SUCCEEDED(MFStartup(MF_VERSION, MFSTARTUP_LITE));
    if (!started) {
      RTC_LOG(LS_ERROR) << "MFStartup failed, no H264 decoding";
    }
  });
  return started;
}

class MfH264Decoder : public webrtc::VideoDecoder {
 public:
  ~MfH264Decoder() override { Release(); }

  bool Configure(const Settings& settings) override { return true; }

  int32_t RegisterDecodeCompleteCallback(
      webrtc::DecodedImageCallback* callback) override {
    callback_ = callback;
    return WEBRTC_VIDEO_CODEC_OK;
  }

  int32_t Release() override {
    transform_.Reset();
    pool_.Release();
    return WEBRTC_VIDEO_CODEC_OK;
  }

  DecoderInfo GetDecoderInfo() const override {
    DecoderInfo info;
    info.implementation_name = "MediaFoundation";
    info.is_hardware_accelerated = false;
    return info;
  }

  const char* ImplementationName() const override { return "MediaFoundation"; }

  int32_t Decode(const webrtc::EncodedImage& image,
                 int64_t render_time_ms) override {
    if (!callback_) {
      return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
    }
    if (!image.data() || image.size() == 0) {
      return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
    }
    if (!transform_ && !Open()) {
      return WEBRTC_VIDEO_CODEC_ERROR;
    }

    ComPtr<IMFSample> sample;
    ComPtr<IMFMediaBuffer> buffer;
    if (FAILED(MFCreateSample(&sample)) ||
        FAILED(MFCreateMemoryBuffer(static_cast<DWORD>(image.size()),
                                    &buffer))) {
      return WEBRTC_VIDEO_CODEC_MEMORY;
    }
    BYTE* data = nullptr;
    if (FAILED(buffer->Lock(&data, nullptr, nullptr))) {
      return WEBRTC_VIDEO_CODEC_MEMORY;
    }
    memcpy(data, image.data(), image.size());
    buffer->Unlock();
    buffer->SetCurrentLength(static_cast<DWORD>(image.size()));
    sample->AddBuffer(buffer.Get());
    sample->SetSampleTime(static_cast<LONGLONG>(image.RtpTimestamp()));
    sample->SetSampleDuration(0);

    HRESULT hr = transform_->ProcessInput(0, sample.Get(), 0);
    if (hr == MF_E_NOTACCEPTING) {
      // The decoder holds pictures it has not handed out yet: take them first.
      if (!Drain()) {
        return WEBRTC_VIDEO_CODEC_ERROR;
      }
      hr = transform_->ProcessInput(0, sample.Get(), 0);
    }
    if (FAILED(hr)) {
      RTC_LOG(LS_WARNING) << "H264 ProcessInput failed: " << hr;
      return WEBRTC_VIDEO_CODEC_ERROR;
    }
    return Drain() ? WEBRTC_VIDEO_CODEC_OK : WEBRTC_VIDEO_CODEC_ERROR;
  }

 private:
  bool Open() {
    if (!StartMediaFoundation()) {
      return false;
    }
    // libwebrtc decodes on its own thread; the transform needs COM there.
    HRESULT co = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    (void)co;

    ComPtr<IMFTransform> transform;
    if (FAILED(CoCreateInstance(CLSID_CMSH264DecoderMFT, nullptr,
                                CLSCTX_INPROC_SERVER,
                                IID_PPV_ARGS(&transform)))) {
      RTC_LOG(LS_ERROR) << "Could not create the Windows H264 decoder";
      return false;
    }

    ComPtr<IMFAttributes> attributes;
    if (SUCCEEDED(transform->GetAttributes(&attributes))) {
      attributes->SetUINT32(MF_LOW_LATENCY, TRUE);
    }
    ComPtr<ICodecAPI> codec;
    if (SUCCEEDED(transform.As(&codec))) {
      VARIANT on;
      VariantInit(&on);
      on.vt = VT_UI4;
      on.ulVal = TRUE;
      codec->SetValue(&CODECAPI_AVLowLatencyMode, &on);
    }

    ComPtr<IMFMediaType> input;
    if (FAILED(MFCreateMediaType(&input))) {
      return false;
    }
    input->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
    input->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_H264);
    input->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive);
    if (FAILED(transform->SetInputType(0, input.Get(), 0))) {
      RTC_LOG(LS_ERROR) << "The Windows H264 decoder refused H264 input";
      return false;
    }
    transform_ = transform;
    if (!PickOutput()) {
      transform_.Reset();
      return false;
    }
    transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0);
    transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);
    return true;
  }

  // Chooses NV12 among the outputs and reads the picture size from it.
  bool PickOutput() {
    for (DWORD i = 0;; ++i) {
      ComPtr<IMFMediaType> type;
      if (FAILED(transform_->GetOutputAvailableType(0, i, &type))) {
        return false;
      }
      GUID subtype{};
      if (FAILED(type->GetGUID(MF_MT_SUBTYPE, &subtype)) ||
          subtype != MFVideoFormat_NV12) {
        continue;
      }
      if (FAILED(transform_->SetOutputType(0, type.Get(), 0))) {
        return false;
      }
      UINT32 width = 0, height = 0;
      MFGetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, &width, &height);
      frame_width_ = static_cast<int>(width);
      frame_height_ = static_cast<int>(height);
      UINT32 stride = 0;
      if (SUCCEEDED(type->GetUINT32(MF_MT_DEFAULT_STRIDE, &stride)) &&
          static_cast<INT32>(stride) > 0) {
        stride_ = static_cast<int>(stride);
      } else {
        stride_ = frame_width_;
      }
      // The coded size is padded to whole macroblocks; the aperture is the
      // picture people see.
      picture_width_ = frame_width_;
      picture_height_ = frame_height_;
      MFVideoArea area{};
      if (SUCCEEDED(type->GetBlob(MF_MT_MINIMUM_DISPLAY_APERTURE,
                                  reinterpret_cast<UINT8*>(&area),
                                  sizeof(area), nullptr)) &&
          area.Area.cx > 0 && area.Area.cy > 0) {
        picture_width_ = area.Area.cx;
        picture_height_ = area.Area.cy;
      }
      return true;
    }
  }

  // Takes every picture the decoder has ready and hands it to libwebrtc.
  bool Drain() {
    for (;;) {
      MFT_OUTPUT_STREAM_INFO info{};
      if (FAILED(transform_->GetOutputStreamInfo(0, &info))) {
        return false;
      }
      ComPtr<IMFSample> sample;
      bool ours = !(info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES);
      if (ours) {
        ComPtr<IMFMediaBuffer> buffer;
        DWORD size = info.cbSize > 0
                         ? info.cbSize
                         : static_cast<DWORD>(stride_ * frame_height_ * 3 / 2);
        if (size == 0 || FAILED(MFCreateSample(&sample)) ||
            FAILED(MFCreateMemoryBuffer(size, &buffer))) {
          return false;
        }
        sample->AddBuffer(buffer.Get());
      }

      MFT_OUTPUT_DATA_BUFFER output{};
      output.dwStreamID = 0;
      output.pSample = sample.Get();
      DWORD status = 0;
      HRESULT hr = transform_->ProcessOutput(0, 1, &output, &status);
      if (output.pEvents) {
        output.pEvents->Release();
      }
      if (!ours && output.pSample) {
        sample.Attach(output.pSample);
      }
      if (hr == MF_E_TRANSFORM_NEED_MORE_INPUT) {
        return true;
      }
      if (hr == MF_E_TRANSFORM_STREAM_CHANGE) {
        if (!PickOutput()) {
          return false;
        }
        continue;
      }
      if (FAILED(hr)) {
        RTC_LOG(LS_WARNING) << "H264 ProcessOutput failed: " << hr;
        return false;
      }
      Deliver(sample.Get());
    }
  }

  void Deliver(IMFSample* sample) {
    if (!sample || frame_width_ <= 0 || frame_height_ <= 0) {
      return;
    }
    LONGLONG time = 0;
    sample->GetSampleTime(&time);

    ComPtr<IMFMediaBuffer> buffer;
    if (FAILED(sample->ConvertToContiguousBuffer(&buffer))) {
      return;
    }
    BYTE* data = nullptr;
    DWORD length = 0;
    int stride = stride_;
    ComPtr<IMF2DBuffer> planar;
    LONG pitch = 0;
    bool locked2d = false;
    if (SUCCEEDED(buffer.As(&planar)) &&
        SUCCEEDED(planar->Lock2D(&data, &pitch)) && pitch > 0) {
      stride = static_cast<int>(pitch);
      locked2d = true;
    } else if (FAILED(buffer->Lock(&data, nullptr, &length))) {
      return;
    }

    const int width = picture_width_ & ~1;
    const int height = picture_height_ & ~1;
    webrtc::scoped_refptr<webrtc::I420Buffer> i420 =
        pool_.CreateI420Buffer(width, height);
    if (i420) {
      const uint8_t* y = data;
      const uint8_t* uv = data + static_cast<size_t>(stride) * frame_height_;
      libyuv::NV12ToI420(y, stride, uv, stride, i420->MutableDataY(),
                         i420->StrideY(), i420->MutableDataU(),
                         i420->StrideU(), i420->MutableDataV(),
                         i420->StrideV(), width, height);
    }
    if (locked2d) {
      planar->Unlock2D();
    } else {
      buffer->Unlock();
    }
    if (!i420) {
      return;
    }

    webrtc::VideoFrame frame =
        webrtc::VideoFrame::Builder()
            .set_video_frame_buffer(i420)
            .set_timestamp_rtp(static_cast<uint32_t>(time))
            .build();
    callback_->Decoded(frame, std::nullopt, std::nullopt);
  }

  webrtc::DecodedImageCallback* callback_ = nullptr;
  ComPtr<IMFTransform> transform_;
  webrtc::VideoFrameBufferPool pool_{false, 8};
  int frame_width_ = 0;
  int frame_height_ = 0;
  int picture_width_ = 0;
  int picture_height_ = 0;
  int stride_ = 0;
};

}  // namespace

std::vector<webrtc::SdpVideoFormat> MfH264DecoderFactory::GetSupportedFormats()
    const {
  std::vector<webrtc::SdpVideoFormat> formats;
  const webrtc::H264Profile profiles[] = {
      webrtc::H264Profile::kProfileConstrainedBaseline,
      webrtc::H264Profile::kProfileBaseline,
      webrtc::H264Profile::kProfileMain,
      webrtc::H264Profile::kProfileConstrainedHigh,
      webrtc::H264Profile::kProfileHigh,
  };
  for (const char* mode : {"1", "0"}) {
    for (auto profile : profiles) {
      formats.push_back(webrtc::CreateH264Format(
          profile, webrtc::H264Level::kLevel5_2, mode));
    }
  }
  return formats;
}

std::unique_ptr<webrtc::VideoDecoder> MfH264DecoderFactory::Create(
    const webrtc::Environment& env,
    const webrtc::SdpVideoFormat& format) {
  return std::make_unique<MfH264Decoder>();
}

}  // namespace livekit_ffi

#endif
