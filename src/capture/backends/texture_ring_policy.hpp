#pragma once

#include <cstdint>

#include "telinha/capture/pixel_format.hpp"
#include "telinha/core/arena.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/handle.hpp"

namespace tl::capture {

struct SurfaceLayout {
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    PixelFormat format = PixelFormat::Unknown;

    [[nodiscard]] constexpr bool valid() const noexcept
    {
        return width != 0 && height != 0 && format != PixelFormat::Unknown;
    }

    friend constexpr bool operator==(const SurfaceLayout& a, const SurfaceLayout& b) noexcept
    {
        return a.width == b.width && a.height == b.height && a.format == b.format;
    }
    friend constexpr bool operator!=(const SurfaceLayout& a, const SurfaceLayout& b) noexcept
    {
        return !(a == b);
    }
};

struct TextureTag {};
using TextureHandle = Handle<TextureTag>;

enum class LeaseOutcome : std::uint8_t {
    Reused,
    NeedsCreation,
    Exhausted,
};

struct TextureLease {
    TextureHandle handle;
    LeaseOutcome outcome = LeaseOutcome::Exhausted;
    bool retire_stale = false;
    SurfaceLayout stale_layout;
};

class TextureRingPolicy {
public:
    struct Statistics {
        std::uint64_t leases = 0;
        std::uint64_t reuses = 0;
        std::uint64_t creations = 0;
        std::uint64_t retirements = 0;
        std::uint64_t exhaustions = 0;
        std::uint64_t stale_returns = 0;
        std::uint32_t peak_leased = 0;
    };

    TextureRingPolicy() noexcept = default;

    TextureRingPolicy(const TextureRingPolicy&) = delete;
    TextureRingPolicy& operator=(const TextureRingPolicy&) = delete;

    [[nodiscard]] bool initialize(LinearArena& arena, std::uint32_t capacity) noexcept;

    void set_layout(const SurfaceLayout& layout) noexcept;
    [[nodiscard]] const SurfaceLayout& layout() const noexcept { return layout_; }

    [[nodiscard]] TextureLease lease() noexcept;
    void mark_created(TextureHandle handle) noexcept;
    [[nodiscard]] bool give_back(TextureHandle handle) noexcept;

    [[nodiscard]] bool leased(TextureHandle handle) const noexcept;
    [[nodiscard]] std::uint32_t capacity() const noexcept { return capacity_; }
    [[nodiscard]] std::uint32_t leased_count() const noexcept { return leased_count_; }
    [[nodiscard]] bool slot_has_texture(std::uint32_t slot) const noexcept;
    [[nodiscard]] SurfaceLayout slot_layout(std::uint32_t slot) const noexcept;

    [[nodiscard]] const Statistics& statistics() const noexcept { return statistics_; }
    void reset_statistics() noexcept { statistics_ = Statistics{}; }
    [[nodiscard]] bool initialized() const noexcept { return slots_ != nullptr; }

private:
    struct Slot {
        SurfaceLayout layout;
        std::uint32_t generation = 1;
        bool has_texture = false;
        bool leased = false;
    };

    Slot* slots_ = nullptr;
    std::uint32_t capacity_ = 0;
    std::uint32_t leased_count_ = 0;
    std::uint32_t cursor_ = 0;
    SurfaceLayout layout_;
    Statistics statistics_;
};

}  // namespace tl::capture
