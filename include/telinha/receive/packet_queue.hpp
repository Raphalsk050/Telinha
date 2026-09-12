#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <new>
#include <type_traits>

#include "telinha/core/arena.hpp"
#include "telinha/core/config.hpp"
#include "telinha/core/result.hpp"
#include "telinha/core/spsc_ring.hpp"
#include "telinha/transport/media_transport.hpp"

namespace tl::receive {

struct VideoPacketHeader {
    std::uint64_t frame_index = 0;
    Nanoseconds remote_time_ns = 0;
    Nanoseconds arrival_time_ns = 0;
    std::uint32_t slot = 0;
    std::uint32_t byte_size = 0;
    std::uint32_t width = 0;
    std::uint32_t height = 0;
    transport::WireVideoCodec codec = transport::WireVideoCodec::Unknown;
    std::uint8_t temporal_index = 0;
    bool keyframe = false;
};

struct AudioPacketHeader {
    Nanoseconds remote_time_ns = 0;
    Nanoseconds arrival_time_ns = 0;
    std::uint32_t slot = 0;
    std::uint32_t byte_size = 0;
    std::uint32_t sample_rate_hz = 0;
    std::uint32_t frames_per_channel = 0;
    std::uint16_t channels = 0;
};

template<typename Header, std::size_t SlotCount>
class PacketQueue {
    static_assert(SlotCount >= 2, "a queue of one has no steady state");
    static_assert((SlotCount & (SlotCount - 1)) == 0, "SlotCount must be a power of two");
    static_assert(std::is_trivially_copyable_v<Header>);

public:
    static constexpr std::size_t slot_count = SlotCount;
    static constexpr std::uint32_t slot_mask = static_cast<std::uint32_t>(SlotCount - 1);

    PacketQueue() noexcept = default;

    PacketQueue(const PacketQueue&) = delete;
    PacketQueue& operator=(const PacketQueue&) = delete;

    ~PacketQueue() { release_storage(); }

    [[nodiscard]] Outcome reserve(std::size_t slot_bytes)
    {
        if (storage_ != nullptr) {
            return fail(Status::AlreadyExists, "PacketQueue::reserve");
        }
        if (slot_bytes == 0) {
            return fail(Status::InvalidArgument, "PacketQueue::reserve");
        }

        const std::size_t stride = LinearArena::align_up(slot_bytes, kCacheLineSize);
        void* memory =
            ::operator new(stride * SlotCount, std::align_val_t{kCacheLineSize}, std::nothrow);
        if (memory == nullptr) {
            return fail(Status::OutOfMemory, "PacketQueue::reserve");
        }

        headers_ = new (std::nothrow) Header[SlotCount];
        if (headers_ == nullptr) {
            ::operator delete(memory, std::align_val_t{kCacheLineSize});
            return fail(Status::OutOfMemory, "PacketQueue::reserve");
        }

        storage_ = static_cast<std::byte*>(memory);
        stride_ = stride;

        for (std::uint32_t index = 0; index < SlotCount; ++index) {
            const bool pushed = free_.push(index);
            TL_ASSERT(pushed);
            (void)pushed;
        }
        return ok();
    }

    [[nodiscard]] std::byte* begin_write(std::size_t bytes, std::uint32_t& slot) noexcept
    {
        if (bytes == 0 || bytes > stride_) {
            oversized_.fetch_add(1, std::memory_order_relaxed);
            return nullptr;
        }
        std::uint32_t index = 0;
        if (!free_.pop(index)) {
            starved_.fetch_add(1, std::memory_order_relaxed);
            return nullptr;
        }
        slot = index;
        return storage_ + static_cast<std::size_t>(index & slot_mask) * stride_;
    }

    void commit(const Header& header) noexcept
    {
        headers_[header.slot & slot_mask] = header;
        const bool pushed = ready_.push(header.slot);
        TL_ASSERT(pushed);
        (void)pushed;
    }

    [[nodiscard]] bool pop(Header& out) noexcept
    {
        std::uint32_t slot = 0;
        if (!ready_.pop(slot)) {
            return false;
        }
        out = headers_[slot & slot_mask];
        return true;
    }

    void recycle(std::uint32_t slot) noexcept
    {
        const bool pushed = free_.push(slot);
        TL_ASSERT(pushed);
        (void)pushed;
    }

    [[nodiscard]] const std::byte* slot_data(std::uint32_t slot) const noexcept
    {
        return storage_ + static_cast<std::size_t>(slot & slot_mask) * stride_;
    }

    [[nodiscard]] std::size_t slot_stride() const noexcept { return stride_; }
    [[nodiscard]] std::size_t byte_capacity() const noexcept { return stride_ * SlotCount; }

    [[nodiscard]] std::uint64_t oversized() const noexcept
    {
        return oversized_.load(std::memory_order_relaxed);
    }

    [[nodiscard]] std::uint64_t starved() const noexcept
    {
        return starved_.load(std::memory_order_relaxed);
    }

private:
    void release_storage() noexcept
    {
        if (storage_ != nullptr) {
            ::operator delete(storage_, std::align_val_t{kCacheLineSize});
            storage_ = nullptr;
        }
        delete[] headers_;
        headers_ = nullptr;
        stride_ = 0;
    }

    std::byte* storage_ = nullptr;
    std::size_t stride_ = 0;
    Header* headers_ = nullptr;
    SpscRing<std::uint32_t, SlotCount> ready_;
    SpscRing<std::uint32_t, SlotCount> free_;
    std::atomic<std::uint64_t> oversized_{0};
    std::atomic<std::uint64_t> starved_{0};
};

using VideoPacketQueue = PacketQueue<VideoPacketHeader, 16>;
using AudioPacketQueue = PacketQueue<AudioPacketHeader, 128>;

}  // namespace tl::receive
