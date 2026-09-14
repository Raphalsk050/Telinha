#pragma once

#include "telinha/core/config.hpp"

#if TL_PLATFORM_WINDOWS

#include <d3d11_1.h>
#include <wrl/client.h>

#include <cstdint>

#include "telinha/capture/capture_target.hpp"
#include "telinha/core/result.hpp"

namespace tl::encode {

class D3dFrameConverter {
public:
    D3dFrameConverter() noexcept = default;
    ~D3dFrameConverter();

    D3dFrameConverter(const D3dFrameConverter&) = delete;
    D3dFrameConverter& operator=(const D3dFrameConverter&) = delete;

    [[nodiscard]] Outcome initialize(ID3D11Device* device, std::uint32_t output_width,
                                     std::uint32_t output_height,
                                     std::uint32_t surface_count) noexcept;

    void shutdown() noexcept;

    [[nodiscard]] Outcome convert(ID3D11Texture2D* source, capture::SurfaceRotation rotation,
                                  std::uint32_t& out_surface_index) noexcept;

    [[nodiscard]] ID3D11Texture2D* surface(std::uint32_t index) const noexcept
    {
        return index < surface_count_ ? outputs_[index].Get() : nullptr;
    }

    [[nodiscard]] std::uint32_t surface_count() const noexcept { return surface_count_; }

    [[nodiscard]] ID3D11Device* device() const noexcept { return device_.Get(); }
    [[nodiscard]] std::uint32_t output_width() const noexcept { return output_width_; }
    [[nodiscard]] std::uint32_t output_height() const noexcept { return output_height_; }
    [[nodiscard]] bool ready() const noexcept { return processor_ != nullptr; }

private:
    static constexpr std::uint32_t kMaxOutputSurfaces = 8;
    static constexpr std::uint32_t kMaxInputViews = 8;

    [[nodiscard]] Outcome ensure_enumerator(std::uint32_t source_width, std::uint32_t source_height,
                                            DXGI_FORMAT source_format) noexcept;

    [[nodiscard]] Outcome ensure_input_view(ID3D11Texture2D* source,
                                            ID3D11VideoProcessorInputView*& out) noexcept;

    void apply_destination(std::uint32_t content_width, std::uint32_t content_height) noexcept;

    Microsoft::WRL::ComPtr<ID3D11Device> device_;
    Microsoft::WRL::ComPtr<ID3D11DeviceContext> context_;
    Microsoft::WRL::ComPtr<ID3D11VideoDevice> video_device_;
    Microsoft::WRL::ComPtr<ID3D11VideoContext> video_context_;
    Microsoft::WRL::ComPtr<ID3D11VideoProcessorEnumerator> enumerator_;
    Microsoft::WRL::ComPtr<ID3D11VideoProcessor> processor_;

    Microsoft::WRL::ComPtr<ID3D11Texture2D> outputs_[kMaxOutputSurfaces];
    Microsoft::WRL::ComPtr<ID3D11VideoProcessorOutputView> output_views_[kMaxOutputSurfaces];
    std::uint32_t output_width_ = 0;
    std::uint32_t output_height_ = 0;
    std::uint32_t surface_count_ = 0;
    std::uint32_t next_surface_ = 0;
    Microsoft::WRL::ComPtr<ID3D11Texture2D> input_sources_[kMaxInputViews];
    Microsoft::WRL::ComPtr<ID3D11VideoProcessorInputView> input_views_[kMaxInputViews];
    std::uint32_t input_view_count_ = 0;

    std::uint32_t enumerated_width_ = 0;
    std::uint32_t enumerated_height_ = 0;
    DXGI_FORMAT enumerated_format_ = DXGI_FORMAT_UNKNOWN;
    std::uint32_t destination_width_ = 0;
    std::uint32_t destination_height_ = 0;
};

}  // namespace tl::encode

#endif
