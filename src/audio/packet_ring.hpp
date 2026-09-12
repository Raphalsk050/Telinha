#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <new>

#include "telinha/core/clock.hpp"
#include "telinha/core/config.hpp"

namespace tl::audio {

enum class PacketFlag : std::uint32_t {
    None = 0,
    Silent = 0x1,
    Discontinuity = 0x2,
    FormatChanged = 0x4,
};

[[nodiscard]] constexpr std::uint32_t operator|(PacketFlag a, PacketFlag b) noexcept
{
    return static_cast<std::uint32_t>(a) | static_cast<std::uint32_t>(b);
}

[[nodiscard]] constexpr bool has_flag(std::uint32_t flags, PacketFlag flag) noexcept
{
    return (flags & static_cast<std::uint32_t>(flag)) != 0;
}

struct PacketHeader {
    std::uint32_t frame_count = 0;
    std::uint32_t payload_bytes = 0;
    std::uint64_t stream_frame_position = 0;
    Nanoseconds capture_time_ns = 0;
    Nanoseconds device_time_ns = 0;
    std::uint32_t flags = 0;
    std::uint32_t timestamp_valid = 0;
};

class PacketRing {
public:
    PacketRing() noexcept = default;

    PacketRing(const PacketRing&) = delete;
    PacketRing& operator=(const PacketRing&) = delete;

    ~PacketRing() { release(); }

    [[nodiscard]] bool reserve(std::size_t capacity_bytes) noexcept
    {
        release();
        if (capacity_bytes < sizeof(PacketHeader) * 2) {
            return false;
        }

        std::size_t rounded = 1;
        while (rounded < capacity_bytes) {
            rounded <<= 1u;
        }

        void* memory = ::operator new(rounded, std::align_val_t{kCacheLineSize}, std::nothrow);
        if (memory == nullptr) {
            return false;
        }

        storage_ = static_cast<std::byte*>(memory);
        capacity_ = rounded;
        mask_ = rounded - 1;
        write_.value.store(0, std::memory_order_relaxed);
        read_.value.store(0, std::memory_order_relaxed);
        cached_read_ = 0;
        cached_write_ = 0;
        return true;
    }

    void release() noexcept
    {
        if (storage_ != nullptr) {
            ::operator delete(storage_, std::align_val_t{kCacheLineSize});
            storage_ = nullptr;
        }
        capacity_ = 0;
        mask_ = 0;
    }

    void clear() noexcept
    {
        const std::uint64_t write = write_.value.load(std::memory_order_acquire);
        read_.value.store(write, std::memory_order_release);
        cached_write_ = write;
    }

    [[nodiscard]] bool valid() const noexcept { return storage_ != nullptr; }
    [[nodiscard]] std::size_t capacity_bytes() const noexcept { return capacity_; }

    [[nodiscard]] std::size_t readable_bytes() const noexcept
    {
        const std::uint64_t read = read_.value.load(std::memory_order_acquire);
        const std::uint64_t write = write_.value.load(std::memory_order_acquire);
        return static_cast<std::size_t>(write - read);
    }

    [[nodiscard]] bool write(const PacketHeader& header, const std::byte* payload) noexcept
    {
        const std::size_t record = sizeof(PacketHeader) + header.payload_bytes;
        const std::uint64_t write = write_.value.load(std::memory_order_relaxed);

        if (capacity_ - static_cast<std::size_t>(write - cached_read_) < record) {
            cached_read_ = read_.value.load(std::memory_order_acquire);
            if (capacity_ - static_cast<std::size_t>(write - cached_read_) < record) {
                return false;
            }
        }

        std::uint64_t cursor = copy_in(write, &header, sizeof(PacketHeader));
        if (header.payload_bytes != 0 && payload != nullptr) {
            cursor = copy_in(cursor, payload, header.payload_bytes);
        }

        write_.value.store(write + record, std::memory_order_release);
        return true;
    }

    [[nodiscard]] bool read(PacketHeader& header, std::byte* destination,
                            std::uint32_t destination_bytes) noexcept
    {
        const std::uint64_t read = read_.value.load(std::memory_order_relaxed);

        if (static_cast<std::size_t>(cached_write_ - read) < sizeof(PacketHeader)) {
            cached_write_ = write_.value.load(std::memory_order_acquire);
            if (static_cast<std::size_t>(cached_write_ - read) < sizeof(PacketHeader)) {
                return false;
            }
        }

        std::uint64_t cursor = copy_out(read, &header, sizeof(PacketHeader));
        if (header.payload_bytes > destination_bytes) {
            read_.value.store(read + sizeof(PacketHeader) + header.payload_bytes,
                              std::memory_order_release);
            return false;
        }
        if (header.payload_bytes != 0 && destination != nullptr) {
            cursor = copy_out(cursor, destination, header.payload_bytes);
        }

        read_.value.store(read + sizeof(PacketHeader) + header.payload_bytes,
                          std::memory_order_release);
        return true;
    }

    [[nodiscard]] bool drop_oldest() noexcept
    {
        const std::uint64_t read = read_.value.load(std::memory_order_relaxed);
        cached_write_ = write_.value.load(std::memory_order_acquire);
        if (static_cast<std::size_t>(cached_write_ - read) < sizeof(PacketHeader)) {
            return false;
        }

        PacketHeader header;
        copy_out(read, &header, sizeof(PacketHeader));
        read_.value.store(read + sizeof(PacketHeader) + header.payload_bytes,
                          std::memory_order_release);
        return true;
    }

private:
    std::uint64_t copy_in(std::uint64_t cursor, const void* source, std::size_t bytes) noexcept
    {
        const std::size_t offset = static_cast<std::size_t>(cursor) & mask_;
        const std::size_t first = capacity_ - offset < bytes ? capacity_ - offset : bytes;
        std::memcpy(storage_ + offset, source, first);
        if (first < bytes) {
            std::memcpy(storage_, static_cast<const std::byte*>(source) + first, bytes - first);
        }
        return cursor + bytes;
    }

    std::uint64_t copy_out(std::uint64_t cursor, void* destination, std::size_t bytes) noexcept
    {
        const std::size_t offset = static_cast<std::size_t>(cursor) & mask_;
        const std::size_t first = capacity_ - offset < bytes ? capacity_ - offset : bytes;
        std::memcpy(destination, storage_ + offset, first);
        if (first < bytes) {
            std::memcpy(static_cast<std::byte*>(destination) + first, storage_, bytes - first);
        }
        return cursor + bytes;
    }

    struct alignas(kCacheLineSize) PaddedCursor {
        std::atomic<std::uint64_t> value{0};
    };

    std::byte* storage_ = nullptr;
    std::size_t capacity_ = 0;
    std::size_t mask_ = 0;

    PaddedCursor write_;
    alignas(kCacheLineSize) std::uint64_t cached_read_ = 0;
    PaddedCursor read_;
    alignas(kCacheLineSize) std::uint64_t cached_write_ = 0;
};

}  // namespace tl::audio
