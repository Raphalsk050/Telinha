#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>
#include <new>
#include <type_traits>
#include <utility>

#include "telinha/core/config.hpp"
#include "telinha/core/span.hpp"

namespace tl {
class LinearArena {
public:
    struct Marker {
        std::size_t offset = 0;
    };

    constexpr LinearArena() noexcept = default;

    LinearArena(std::byte* base, std::size_t capacity) noexcept : base_(base), capacity_(capacity)
    {}

    explicit LinearArena(Span<std::byte> storage) noexcept
        : base_(storage.data()), capacity_(storage.size())
    {}

    LinearArena(const LinearArena&) = delete;
    LinearArena& operator=(const LinearArena&) = delete;
    LinearArena(LinearArena&&) noexcept = default;
    LinearArena& operator=(LinearArena&&) noexcept = default;

    [[nodiscard]] void* allocate(std::size_t bytes, std::size_t alignment) noexcept
    {
        TL_ASSERT(alignment != 0 && (alignment & (alignment - 1)) == 0);

        const std::size_t aligned = align_up(offset_, alignment);
        if (TL_UNLIKELY(aligned > capacity_ || bytes > capacity_ - aligned)) {
            return nullptr;
        }
        offset_ = aligned + bytes;
        return base_ + aligned;
    }

    template<typename T>
    [[nodiscard]] T* allocate_array(std::size_t count) noexcept
    {
        static_assert(std::is_trivially_destructible_v<T>,
                      "arena memory is released by reset, so T must not need a destructor");
        if (count == 0) {
            return nullptr;
        }
        if (count > (static_cast<std::size_t>(-1) / sizeof(T))) {
            return nullptr;
        }
        void* memory = allocate(sizeof(T) * count, alignof(T));
        return static_cast<T*>(memory);
    }

    template<typename T>
    [[nodiscard]] T* allocate_array_aligned(std::size_t count, std::size_t alignment) noexcept
    {
        static_assert(std::is_trivially_destructible_v<T>,
                      "arena memory is released by reset, so T must not need a destructor");
        if (count == 0) {
            return nullptr;
        }
        if (count > (static_cast<std::size_t>(-1) / sizeof(T))) {
            return nullptr;
        }
        return static_cast<T*>(allocate(sizeof(T) * count, alignment));
    }

    template<typename T, typename... Args>
    [[nodiscard]] T* construct(Args&&... args) noexcept
    {
        static_assert(std::is_trivially_destructible_v<T>,
                      "arena memory is released by reset, so T must not need a destructor");
        void* memory = allocate(sizeof(T), alignof(T));
        if (memory == nullptr) {
            return nullptr;
        }
        return ::new (memory) T(static_cast<Args&&>(args)...);
    }

    [[nodiscard]] Marker mark() const noexcept { return Marker{offset_}; }

    void release(Marker marker) noexcept
    {
        TL_ASSERT(marker.offset <= offset_);
        offset_ = marker.offset;
    }

    void reset() noexcept { offset_ = 0; }

    [[nodiscard]] std::size_t used() const noexcept { return offset_; }
    [[nodiscard]] std::size_t capacity() const noexcept { return capacity_; }
    [[nodiscard]] std::size_t remaining() const noexcept { return capacity_ - offset_; }

    [[nodiscard]] std::size_t high_water_mark() const noexcept { return high_water_; }

    void update_high_water() noexcept
    {
        if (offset_ > high_water_) {
            high_water_ = offset_;
        }
    }

    [[nodiscard]] static constexpr std::size_t align_up(std::size_t value,
                                                        std::size_t alignment) noexcept
    {
        return (value + (alignment - 1)) & ~(alignment - 1);
    }

private:
    std::byte* base_ = nullptr;
    std::size_t capacity_ = 0;
    std::size_t offset_ = 0;
    std::size_t high_water_ = 0;
};

class ArenaStorage {
public:
    ArenaStorage() noexcept = default;

    explicit ArenaStorage(std::size_t capacity, std::size_t alignment = kCacheLineSize)
    {
        (void)reserve(capacity, alignment);
    }

    ArenaStorage(const ArenaStorage&) = delete;
    ArenaStorage& operator=(const ArenaStorage&) = delete;

    ArenaStorage(ArenaStorage&& other) noexcept
        : block_(other.block_),
          capacity_(other.capacity_),
          alignment_(other.alignment_),
          arena_(std::move(other.arena_))
    {
        other.block_ = nullptr;
        other.capacity_ = 0;
        other.alignment_ = kCacheLineSize;
        other.arena_ = LinearArena();
    }

    ArenaStorage& operator=(ArenaStorage&& other) noexcept
    {
        if (this != &other) {
            release();
            block_ = other.block_;
            capacity_ = other.capacity_;
            alignment_ = other.alignment_;
            arena_ = std::move(other.arena_);
            other.block_ = nullptr;
            other.capacity_ = 0;
            other.alignment_ = kCacheLineSize;
            other.arena_ = LinearArena();
        }
        return *this;
    }

    ~ArenaStorage() { release(); }

    [[nodiscard]] bool reserve(std::size_t capacity, std::size_t alignment = kCacheLineSize)
    {
        release();
        if (capacity == 0) {
            return true;
        }
        const std::size_t rounded = LinearArena::align_up(capacity, alignment);
        void* memory = ::operator new(rounded, std::align_val_t{alignment}, std::nothrow);
        if (memory == nullptr) {
            return false;
        }
        block_ = static_cast<std::byte*>(memory);
        capacity_ = rounded;
        alignment_ = alignment;
        arena_ = LinearArena(block_, capacity_);
        return true;
    }

    [[nodiscard]] LinearArena& arena() noexcept { return arena_; }
    [[nodiscard]] const LinearArena& arena() const noexcept { return arena_; }
    [[nodiscard]] bool valid() const noexcept { return block_ != nullptr; }
    [[nodiscard]] std::size_t capacity() const noexcept { return capacity_; }

private:
    void release() noexcept
    {
        if (block_ != nullptr) {
            ::operator delete(block_, std::align_val_t{alignment_});
            block_ = nullptr;
        }
        capacity_ = 0;
        alignment_ = kCacheLineSize;
        arena_ = LinearArena();
    }

    std::byte* block_ = nullptr;
    std::size_t capacity_ = 0;
    std::size_t alignment_ = kCacheLineSize;
    LinearArena arena_;
};

class ArenaScope {
public:
    explicit ArenaScope(LinearArena& arena) noexcept : arena_(&arena), marker_(arena.mark()) {}

    ArenaScope(const ArenaScope&) = delete;
    ArenaScope& operator=(const ArenaScope&) = delete;

    ~ArenaScope()
    {
        arena_->update_high_water();
        arena_->release(marker_);
    }

private:
    LinearArena* arena_;
    LinearArena::Marker marker_;
};
}  // namespace tl
