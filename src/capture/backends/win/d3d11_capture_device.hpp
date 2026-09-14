#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>

#include "../texture_ring_policy.hpp"
#include "telinha/capture/captured_frame.hpp"
#include "telinha/core/arena.hpp"
#include "telinha/core/result.hpp"
#include "win_capture_common.hpp"

namespace tl::capture::win {

[[nodiscard]] Outcome find_output_for_monitor(HMONITOR monitor, ComPtr<IDXGIAdapter1>& adapter,
                                              ComPtr<IDXGIOutput1>& output) noexcept;

class D3D11CaptureDevice {
public:
    D3D11CaptureDevice() noexcept = default;
    ~D3D11CaptureDevice() { destroy(); }

    D3D11CaptureDevice(const D3D11CaptureDevice&) = delete;
    D3D11CaptureDevice& operator=(const D3D11CaptureDevice&) = delete;

    [[nodiscard]] Outcome create_for_monitor(HMONITOR monitor) noexcept;
    [[nodiscard]] Outcome create_default() noexcept;
    void destroy() noexcept;

    [[nodiscard]] ID3D11Device* device() const noexcept { return device_.Get(); }
    [[nodiscard]] ID3D11DeviceContext* context() const noexcept { return context_.Get(); }
    [[nodiscard]] IDXGIAdapter1* adapter() const noexcept { return adapter_.Get(); }
    [[nodiscard]] IDXGIOutput1* output() const noexcept { return output_.Get(); }
    [[nodiscard]] bool valid() const noexcept { return device_ != nullptr; }

private:
    [[nodiscard]] Outcome create_on_adapter(IDXGIAdapter1* adapter) noexcept;

    ComPtr<ID3D11Device> device_;
    ComPtr<ID3D11DeviceContext> context_;
    ComPtr<IDXGIAdapter1> adapter_;
    ComPtr<IDXGIOutput1> output_;
};

class D3D11TextureRing {
public:
    D3D11TextureRing() noexcept = default;
    ~D3D11TextureRing() { destroy(); }

    D3D11TextureRing(const D3D11TextureRing&) = delete;
    D3D11TextureRing& operator=(const D3D11TextureRing&) = delete;

    [[nodiscard]] Outcome initialize(LinearArena& arena, ID3D11Device* device,
                                     std::uint32_t capacity) noexcept;
    void destroy() noexcept;

    void set_layout(const SurfaceLayout& layout) noexcept;
    [[nodiscard]] const SurfaceLayout& layout() const noexcept { return policy_.layout(); }

    [[nodiscard]] Outcome acquire(ID3D11Texture2D*& out, TextureHandle& handle) noexcept;
    void give_back(TextureHandle handle) noexcept;

    [[nodiscard]] const TextureRingPolicy& policy() const noexcept { return policy_; }

private:
    void retire(std::uint32_t slot) noexcept;

    ID3D11Device* device_ = nullptr;
    ID3D11Texture2D** textures_ = nullptr;
    TextureRingPolicy policy_;
};

struct CursorPlacement {
    Rect area;
    Rect visible;
    SurfaceRotation rotation = SurfaceRotation::None;
};

class D3D11CursorCompositor {
public:
    static constexpr std::uint32_t kMaxExtent = 256;

    D3D11CursorCompositor() noexcept = default;
    ~D3D11CursorCompositor() { destroy(); }

    D3D11CursorCompositor(const D3D11CursorCompositor&) = delete;
    D3D11CursorCompositor& operator=(const D3D11CursorCompositor&) = delete;

    [[nodiscard]] Outcome prepare(ID3D11Device* device, PixelFormat format) noexcept;
    void destroy() noexcept;

    [[nodiscard]] Outcome upload_shape(CursorShapeKind kind, std::uint32_t width,
                                       std::uint32_t reported_height, std::uint32_t pitch,
                                       Span<const std::byte> source) noexcept;

    [[nodiscard]] Outcome draw(ID3D11Texture2D* target, ID3D11ShaderResourceView* background,
                               const CursorPlacement& placement) noexcept;

    [[nodiscard]] bool ready() const noexcept
    {
        return pixel_shader_ != nullptr && format_ != PixelFormat::Unknown;
    }
    [[nodiscard]] bool has_shape() const noexcept { return shape_width_ != 0; }

private:
    [[nodiscard]] Outcome create_pipeline(ID3D11Device* device) noexcept;

    ComPtr<ID3D11Device> device_;
    ComPtr<ID3D11DeviceContext> context_;
    ComPtr<ID3D11VertexShader> vertex_shader_;
    ComPtr<ID3D11PixelShader> pixel_shader_;
    ComPtr<ID3D11Buffer> constants_;
    ComPtr<ID3D11Texture2D> shape_;
    ComPtr<ID3D11ShaderResourceView> shape_view_;
    std::unique_ptr<std::byte[]> scratch_;
    PixelFormat format_ = PixelFormat::Unknown;
    std::uint32_t shape_width_ = 0;
    std::uint32_t shape_height_ = 0;
    std::uint32_t blend_mode_ = 0;
};

}  // namespace tl::capture::win
