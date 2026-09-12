#include "texture_ring_policy.hpp"

namespace tl::capture {

bool TextureRingPolicy::initialize(LinearArena& arena, std::uint32_t capacity) noexcept
{
    if (capacity == 0 || capacity >= kInvalidIndex) {
        return false;
    }

    slots_ = arena.allocate_array_aligned<Slot>(capacity, kCacheLineSize);
    if (slots_ == nullptr) {
        return false;
    }

    capacity_ = capacity;
    leased_count_ = 0;
    cursor_ = 0;
    for (std::uint32_t i = 0; i < capacity; ++i) {
        slots_[i] = Slot{};
    }
    return true;
}

void TextureRingPolicy::set_layout(const SurfaceLayout& layout) noexcept
{
    layout_ = layout;
}

TextureLease TextureRingPolicy::lease() noexcept
{
    ++statistics_.leases;

    if (!initialized() || !layout_.valid()) {
        ++statistics_.exhaustions;
        return TextureLease{};
    }

    std::uint32_t chosen = kInvalidIndex;
    for (std::uint32_t probe = 0; probe < capacity_; ++probe) {
        const std::uint32_t candidate = (cursor_ + probe) % capacity_;
        if (!slots_[candidate].leased) {
            chosen = candidate;
            break;
        }
    }

    if (chosen == kInvalidIndex) {
        ++statistics_.exhaustions;
        return TextureLease{};
    }

    cursor_ = (chosen + 1) % capacity_;

    Slot& slot = slots_[chosen];
    TextureLease result;
    result.handle = TextureHandle{chosen, slot.generation};

    if (slot.has_texture && slot.layout == layout_) {
        result.outcome = LeaseOutcome::Reused;
        ++statistics_.reuses;
    } else {
        result.outcome = LeaseOutcome::NeedsCreation;
        result.retire_stale = slot.has_texture;
        result.stale_layout = slot.layout;
        if (slot.has_texture) {
            ++statistics_.retirements;
        }
        slot.has_texture = false;
        slot.layout = layout_;
    }

    slot.leased = true;
    ++leased_count_;
    if (leased_count_ > statistics_.peak_leased) {
        statistics_.peak_leased = leased_count_;
    }
    return result;
}

void TextureRingPolicy::mark_created(TextureHandle handle) noexcept
{
    if (!leased(handle)) {
        return;
    }
    Slot& slot = slots_[handle.index];
    slot.has_texture = true;
    slot.layout = layout_;
    ++statistics_.creations;
}

bool TextureRingPolicy::give_back(TextureHandle handle) noexcept
{
    if (!leased(handle)) {
        return false;
    }

    Slot& slot = slots_[handle.index];
    if (slot.has_texture && slot.layout != layout_) {
        ++statistics_.stale_returns;
    }

    slot.leased = false;
    --leased_count_;
    slot.generation = slot.generation + 1 == 0 ? 1 : slot.generation + 1;
    return true;
}

bool TextureRingPolicy::leased(TextureHandle handle) const noexcept
{
    return initialized() && handle.index < capacity_ &&
           slots_[handle.index].generation == handle.generation && slots_[handle.index].leased;
}

bool TextureRingPolicy::slot_has_texture(std::uint32_t slot) const noexcept
{
    return initialized() && slot < capacity_ && slots_[slot].has_texture;
}

SurfaceLayout TextureRingPolicy::slot_layout(std::uint32_t slot) const noexcept
{
    if (!initialized() || slot >= capacity_) {
        return SurfaceLayout{};
    }
    return slots_[slot].layout;
}

}  // namespace tl::capture
