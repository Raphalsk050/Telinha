#include <cstring>
#include <thread>
#include <type_traits>
#include <vector>

#include "telinha/capture/capture_pipeline.hpp"
#include "telinha/capture/capture_source.hpp"
#include "telinha/capture/capture_stats.hpp"
#include "telinha/capture/capture_target.hpp"
#include "telinha/capture/captured_frame.hpp"
#include "telinha/capture/pixel_format.hpp"
#include "telinha/core/arena.hpp"
#include "telinha/core/clock.hpp"
#include "telinha/core/handle.hpp"
#include "telinha/core/latency_histogram.hpp"
#include "telinha/core/log.hpp"
#include "telinha/core/mpmc_ring.hpp"
#include "telinha/core/pool.hpp"
#include "telinha/core/rect.hpp"
#include "telinha/core/result.hpp"
#include "telinha/core/span.hpp"
#include "telinha/core/spsc_ring.hpp"
#include "telinha/core/tile_dirty_map.hpp"
#include "test_framework.hpp"

using namespace tl;

static_assert(std::is_trivially_copyable_v<Error>);
static_assert(std::is_trivially_copyable_v<capture::CaptureTarget>);
static_assert(std::is_trivially_copyable_v<capture::CapturedFrame>);
static_assert(std::is_trivially_destructible_v<capture::CapturedFrame>);
static_assert(std::is_trivially_copyable_v<capture::FrameMetadata>);
static_assert(std::is_trivially_copyable_v<capture::CursorState>);
static_assert(std::is_trivially_copyable_v<capture::MoveRect>);
static_assert(sizeof(capture::CaptureTarget) == 16);
static_assert(alignof(std::max_align_t) <= kCacheLineSize);

TEST_CASE("result", "outcome carries status and context")
{
    const Outcome good = ok();
    CHECK(good.ok());
    CHECK(good.status() == Status::Ok);

    const Outcome bad = fail(Status::DeviceLost, "unit test", -7);
    CHECK(!bad.ok());
    CHECK(bad.status() == Status::DeviceLost);
    CHECK(bad.error().platform_code == -7);
    CHECK(std::strcmp(bad.error().context, "unit test") == 0);
    CHECK(std::strcmp(to_string(Status::DeviceLost), "DeviceLost") == 0);
}

TEST_CASE("result", "result holds a value in place")
{
    Result<int> value{42};
    REQUIRE(value.ok());
    CHECK_EQ(value.value(), 42);

    Result<int> failure{Error{Status::Timeout, "no frame"}};
    CHECK(!failure.ok());
    CHECK(failure.status() == Status::Timeout);
    CHECK_EQ(failure.value_or(7), 7);

    Result<std::vector<int>> moved{std::vector<int>{1, 2, 3}};
    REQUIRE(moved.ok());
    CHECK_EQ(moved.value().size(), std::size_t{3});
}

TEST_CASE("arena", "allocation is aligned and rewinds")
{
    ArenaStorage storage(4096);
    REQUIRE(storage.valid());
    LinearArena& arena = storage.arena();

    void* first = arena.allocate(1, 1);
    void* second = arena.allocate(8, 64);
    REQUIRE(first != nullptr);
    REQUIRE(second != nullptr);
    CHECK((reinterpret_cast<std::uintptr_t>(second) % 64) == 0);

    const LinearArena::Marker marker = arena.mark();
    const std::size_t used_before = arena.used();
    std::int32_t* block = arena.allocate_array<std::int32_t>(100);
    REQUIRE(block != nullptr);
    CHECK(arena.used() > used_before);
    arena.release(marker);
    CHECK_EQ(arena.used(), used_before);

    arena.reset();
    CHECK_EQ(arena.used(), std::size_t{0});
}

TEST_CASE("arena", "exhaustion returns null instead of growing")
{
    ArenaStorage storage(128);
    REQUIRE(storage.valid());
    LinearArena& arena = storage.arena();
    CHECK(arena.allocate(64, 1) != nullptr);
    CHECK(arena.allocate(4096, 1) == nullptr);
}

TEST_CASE("pool", "stale handles are rejected")
{
    ArenaStorage storage(64 * 1024);
    REQUIRE(storage.valid());

    Pool<std::uint64_t> pool;
    REQUIRE(pool.initialize(storage.arena(), 4));

    const auto a = pool.acquire(std::uint64_t{1});
    const auto b = pool.acquire(std::uint64_t{2});
    REQUIRE(a.valid());
    REQUIRE(b.valid());
    CHECK_EQ(pool.live_count(), std::uint32_t{2});

    REQUIRE(pool.get(a) != nullptr);
    CHECK_EQ(*pool.get(a), std::uint64_t{1});

    CHECK(pool.release(a));
    CHECK(pool.get(a) == nullptr);
    CHECK(!pool.release(a));

    const auto c = pool.acquire(std::uint64_t{3});
    REQUIRE(c.valid());
    CHECK(c.index == a.index);
    CHECK(c.generation != a.generation);
    CHECK(pool.get(a) == nullptr);
}

TEST_CASE("pool", "capacity is a hard limit")
{
    ArenaStorage storage(64 * 1024);
    REQUIRE(storage.valid());
    Pool<std::uint32_t> pool;
    REQUIRE(pool.initialize(storage.arena(), 2));

    CHECK(pool.acquire(std::uint32_t{0}).valid());
    CHECK(pool.acquire(std::uint32_t{1}).valid());
    CHECK(pool.full());
    CHECK(!pool.acquire(std::uint32_t{2}).valid());
}

TEST_CASE("spsc_ring", "wraps and preserves order across threads")
{
    SpscRing<std::uint32_t, 8> ring;
    std::uint32_t value = 0;
    CHECK(!ring.pop(value));

    for (std::uint32_t i = 0; i < 8; ++i) {
        CHECK(ring.push(i));
    }
    CHECK(!ring.push(99));

    for (std::uint32_t i = 0; i < 8; ++i) {
        REQUIRE(ring.pop(value));
        CHECK_EQ(value, i);
    }
    CHECK(!ring.pop(value));

    constexpr std::uint32_t kCount = 20000;
    SpscRing<std::uint32_t, 64> shared;
    std::uint32_t observed = 0;
    std::thread producer([&shared] {
        for (std::uint32_t i = 0; i < kCount; ++i) {
            while (!shared.push(i)) {
                std::this_thread::yield();
            }
        }
    });

    for (std::uint32_t expected = 0; expected < kCount; ++expected) {
        std::uint32_t got = 0;
        while (!shared.pop(got)) {
            std::this_thread::yield();
        }
        if (got != expected) {
            break;
        }
        ++observed;
    }
    producer.join();
    CHECK_EQ(observed, kCount);
}

TEST_CASE("mpmc_ring", "every item survives concurrent producers")
{
    constexpr std::uint32_t kProducers = 4;
    constexpr std::uint32_t kPerProducer = 4000;

    MpmcRing<std::uint32_t, 256> ring;
    std::vector<std::thread> producers;
    producers.reserve(kProducers);

    for (std::uint32_t p = 0; p < kProducers; ++p) {
        producers.emplace_back([&ring, p] {
            for (std::uint32_t i = 0; i < kPerProducer; ++i) {
                while (!ring.push(p * kPerProducer + i)) {
                    std::this_thread::yield();
                }
            }
        });
    }

    std::vector<bool> seen(kProducers * kPerProducer, false);
    std::uint32_t received = 0;
    while (received < kProducers * kPerProducer) {
        std::uint32_t value = 0;
        if (ring.pop(value)) {
            REQUIRE(value < seen.size());
            CHECK(!seen[value]);
            seen[value] = true;
            ++received;
        } else {
            std::this_thread::yield();
        }
    }

    for (std::thread& producer : producers) {
        producer.join();
    }
    CHECK_EQ(received, kProducers * kPerProducer);
}

TEST_CASE("latency_histogram", "bucket bounds contain the recorded value")
{
    const std::uint64_t samples[] = {0,   1,    255,     256,       257,        511,
                                     512, 1000, 999'999, 1'000'000, 16'666'666, 1'000'000'000ull};
    for (std::uint64_t sample : samples) {
        const std::uint32_t index = LatencyHistogram::counter_index(sample);
        CHECK(index < LatencyHistogram::kCounterCount);
        CHECK(LatencyHistogram::lowest_equivalent(index) <= sample);
        CHECK(LatencyHistogram::highest_equivalent(index) >= sample);
    }

    for (std::uint32_t index = 1; index < LatencyHistogram::kCounterCount; ++index) {
        CHECK(LatencyHistogram::lowest_equivalent(index) ==
              LatencyHistogram::highest_equivalent(index - 1) + 1);
    }
}

TEST_CASE("latency_histogram", "percentiles track a known distribution")
{
    LatencyHistogram histogram;
    for (std::uint64_t i = 1; i <= 1000; ++i) {
        histogram.record(i * 1000);
    }

    CHECK_EQ(histogram.count(), std::uint64_t{1000});
    CHECK_EQ(histogram.min_ns(), std::uint64_t{1000});
    CHECK_EQ(histogram.max_ns(), std::uint64_t{1'000'000});

    const LatencyHistogram::Report report = histogram.report();
    const double tolerance = 0.01;
    CHECK(static_cast<double>(report.p50_ns) >= 500'000 * (1.0 - tolerance));
    CHECK(static_cast<double>(report.p50_ns) <= 500'000 * (1.0 + tolerance));
    CHECK(static_cast<double>(report.p99_ns) >= 990'000 * (1.0 - tolerance));
    CHECK(static_cast<double>(report.p99_ns) <= 990'000 * (1.0 + tolerance));
    CHECK(report.low_one_percent_ns >= 995'000 * (1.0 - tolerance));
    CHECK(report.low_one_percent_ns <= 1'000'000 * (1.0 + tolerance));
    CHECK(report.mean_ns > 500'000.0 * (1.0 - tolerance));
}

TEST_CASE("latency_histogram", "one percent low reflects a spike the mean hides")
{
    LatencyHistogram histogram;
    for (int i = 0; i < 990; ++i) {
        histogram.record(8 * kNanosecondsPerMillisecond);
    }
    for (int i = 0; i < 10; ++i) {
        histogram.record(40 * kNanosecondsPerMillisecond);
    }

    const LatencyHistogram::Report report = histogram.report();
    CHECK(report.mean_ns < 9.0 * 1'000'000.0);
    CHECK(report.low_one_percent_ns > 39.0 * 1'000'000.0);
    CHECK(report.p50_ns < std::uint64_t{9'000'000});
}

TEST_CASE("rect", "structure of arrays keeps bounds and area")
{
    ArenaStorage storage(64 * 1024);
    REQUIRE(storage.valid());

    RectSoA rects;
    REQUIRE(rects.initialize(storage.arena(), 8));
    CHECK(rects.empty());

    CHECK(rects.push(Rect{0, 0, 10, 10}));
    CHECK(rects.push(Rect{20, 5, 40, 25}));
    CHECK_EQ(rects.count(), std::uint32_t{2});
    CHECK(rects.at(1) == (Rect{20, 5, 40, 25}));
    CHECK(rects.bounds() == (Rect{0, 0, 40, 25}));
    CHECK_EQ(rects.total_area(), std::int64_t{100 + 400});

    rects.clear();
    CHECK(rects.empty());
    CHECK(rects.bounds() == Rect{});
}

TEST_CASE("tile_dirty_map", "rectangles map onto the encoder tile grid")
{
    ArenaStorage storage(256 * 1024);
    REQUIRE(storage.valid());

    TileDirtyMap tiles;
    REQUIRE(tiles.initialize(storage.arena(), 1920, 1080, 16));
    CHECK_EQ(tiles.tiles_x(), std::uint32_t{120});
    CHECK_EQ(tiles.tiles_y(), std::uint32_t{68});
    CHECK_EQ(tiles.dirty_tile_count(), std::uint32_t{0});

    tiles.mark(Rect{0, 0, 16, 16});
    CHECK_EQ(tiles.dirty_tile_count(), std::uint32_t{1});
    CHECK(tiles.is_dirty(0, 0));
    CHECK(!tiles.is_dirty(1, 0));

    tiles.clear();
    tiles.mark(Rect{15, 15, 17, 17});
    CHECK_EQ(tiles.dirty_tile_count(), std::uint32_t{4});

    tiles.clear();
    tiles.mark(Rect{-100, -100, 8, 8});
    CHECK_EQ(tiles.dirty_tile_count(), std::uint32_t{1});

    tiles.clear();
    tiles.mark(Rect{1900, 1060, 5000, 5000});
    CHECK(tiles.is_dirty(tiles.tiles_x() - 1, tiles.tiles_y() - 1));

    tiles.clear();
    tiles.mark(Rect{0, 0, 0, 0});
    CHECK_EQ(tiles.dirty_tile_count(), std::uint32_t{0});

    tiles.mark_all();
    CHECK_EQ(tiles.dirty_tile_count(), tiles.tile_count());
}

TEST_CASE("tile_dirty_map", "marking a rectangle list matches marking each rectangle")
{
    ArenaStorage storage(256 * 1024);
    REQUIRE(storage.valid());
    LinearArena& arena = storage.arena();

    RectSoA rects;
    REQUIRE(rects.initialize(arena, 4));
    CHECK(rects.push(Rect{0, 0, 33, 33}));
    CHECK(rects.push(Rect{200, 200, 264, 264}));

    TileDirtyMap from_list;
    TileDirtyMap from_each;
    REQUIRE(from_list.initialize(arena, 640, 480, 32));
    REQUIRE(from_each.initialize(arena, 640, 480, 32));

    from_list.mark(rects);
    for (std::uint32_t i = 0; i < rects.count(); ++i) {
        from_each.mark(rects.at(i));
    }

    CHECK_EQ(from_list.dirty_tile_count(), from_each.dirty_tile_count());
    for (std::uint32_t y = 0; y < from_list.tiles_y(); ++y) {
        for (std::uint32_t x = 0; x < from_list.tiles_x(); ++x) {
            CHECK(from_list.is_dirty(x, y) == from_each.is_dirty(x, y));
        }
    }
}

TEST_CASE("clock", "monotonic and in nanoseconds")
{
    const Nanoseconds first = now_ns();
    Nanoseconds second = now_ns();
    for (int i = 0; i < 1000 && second == first; ++i) {
        second = now_ns();
    }
    CHECK(second >= first);
    CHECK(ticks_per_second() > 0);
    CHECK(ns_to_ms(16'666'666) > 16.0);
    CHECK(ns_to_ms(16'666'666) < 17.0);
}

TEST_CASE("log", "records survive a round trip and drops are counted")
{
    Logger& logger = Logger::instance();
    const LogLevel previous = logger.min_level();
    logger.set_min_level(LogLevel::Trace);
    logger.reset_dropped_records();
    (void)logger.drain([](const LogRecord&) {});

    logger.write(LogLevel::Warn, "target %s lost after %d frames", "monitor 0", 12);

    std::size_t delivered = 0;
    char captured[kLogMessageCapacity] = {};
    delivered = logger.drain([&captured](const LogRecord& record) {
        std::memcpy(captured, record.message, kLogMessageCapacity);
    });

    CHECK_EQ(delivered, std::size_t{1});
    CHECK(std::strcmp(captured, "target monitor 0 lost after 12 frames") == 0);

    logger.set_min_level(LogLevel::Error);
    logger.write(LogLevel::Debug, "filtered out");
    CHECK_EQ(logger.drain([](const LogRecord&) {}), std::size_t{0});

    logger.set_min_level(previous);
}

TEST_CASE("capture_target", "monitor and window targets stay distinct")
{
    const capture::CaptureTarget monitor = capture::CaptureTarget::monitor(0x1234);
    const capture::CaptureTarget window = capture::CaptureTarget::window(0x1234);

    CHECK(monitor.valid());
    CHECK(window.valid());
    CHECK(monitor != window);
    CHECK(monitor == capture::CaptureTarget::monitor(0x1234));
    CHECK(!capture::CaptureTarget{}.valid());
    CHECK(!capture::CaptureTarget::monitor(0).valid());
    CHECK(std::strcmp(capture::to_string(monitor.kind), "Monitor") == 0);
}

TEST_CASE("pixel_format", "bit depth is reported for every format")
{
    CHECK_EQ(capture::bits_per_pixel(capture::PixelFormat::B8G8R8A8Unorm), std::uint32_t{32});
    CHECK_EQ(capture::bits_per_pixel(capture::PixelFormat::NV12), std::uint32_t{12});
    CHECK_EQ(capture::bits_per_pixel(capture::PixelFormat::Unknown), std::uint32_t{0});
    CHECK(capture::is_high_dynamic_range(capture::PixelFormat::R10G10B10A2Unorm));
    CHECK(!capture::is_high_dynamic_range(capture::PixelFormat::B8G8R8A8Unorm));
}

TEST_CASE("capture_source", "an invalid target is refused before any platform call")
{
    capture::CaptureOptions options;
    Result<std::unique_ptr<capture::CaptureSource>> source =
        capture::create_capture_source(capture::CaptureTarget{}, options);
    CHECK(!source.ok());
}

TEST_CASE("capture_stats", "the report names the budget verdict")
{
    capture::CaptureStats stats;
    stats.frames_acquired = 100;
    stats.dirty_tiles_total = 250;
    stats.tiles_total = 1000;
    for (int i = 0; i < 100; ++i) {
        stats.acquire_ns.record(1 * kNanosecondsPerMillisecond);
        stats.classify_ns.record(50 * kNanosecondsPerMicrosecond);
    }

    char buffer[1024] = {};
    const int written =
        capture::format_capture_report(stats, 2 * kNanosecondsPerMillisecond, buffer, 1024);
    CHECK(written > 0);
    CHECK(std::strstr(buffer, "within budget") != nullptr);
    CHECK(std::strstr(buffer, "OVER BUDGET") == nullptr);
    CHECK(stats.mean_dirty_tile_ratio() > 0.24);
    CHECK(stats.mean_dirty_tile_ratio() < 0.26);

    capture::CaptureStats over;
    for (int i = 0; i < 100; ++i) {
        over.acquire_ns.record(5 * kNanosecondsPerMillisecond);
        over.classify_ns.record(kNanosecondsPerMillisecond);
    }
    CHECK(capture::format_capture_report(over, 2 * kNanosecondsPerMillisecond, buffer, 1024) > 0);
    CHECK(std::strstr(buffer, "OVER BUDGET") != nullptr);

    stats.reset();
    CHECK_EQ(stats.frames_acquired, std::uint64_t{0});
    CHECK_EQ(stats.acquire_ns.count(), std::uint64_t{0});
}
