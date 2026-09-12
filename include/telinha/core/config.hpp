#pragma once

#include <cstddef>
#include <cstdint>

#if defined(_MSC_VER)
#define TL_COMPILER_MSVC 1
#else
#define TL_COMPILER_MSVC 0
#endif

#if defined(__clang__)
#define TL_COMPILER_CLANG 1
#else
#define TL_COMPILER_CLANG 0
#endif

#if defined(__GNUC__) && !defined(__clang__)
#define TL_COMPILER_GCC 1
#else
#define TL_COMPILER_GCC 0
#endif

#if defined(_WIN32)
#define TL_PLATFORM_WINDOWS 1
#else
#define TL_PLATFORM_WINDOWS 0
#endif

#if TL_COMPILER_MSVC
#define TL_FORCEINLINE __forceinline
#define TL_NOINLINE __declspec(noinline)
#define TL_RESTRICT __restrict
#else
#define TL_FORCEINLINE inline __attribute__((always_inline))
#define TL_NOINLINE __attribute__((noinline))
#define TL_RESTRICT __restrict__
#endif

#if TL_COMPILER_MSVC
#define TL_LIKELY(x) (x)
#define TL_UNLIKELY(x) (x)
#else
#define TL_LIKELY(x) __builtin_expect(!!(x), 1)
#define TL_UNLIKELY(x) __builtin_expect(!!(x), 0)
#endif

namespace tl {
inline constexpr std::size_t kCacheLineSize = 64;

inline constexpr std::size_t kDefaultFrameQueueCapacity = 8;
}  // namespace tl

#if defined(TELINHA_ENABLE_ASSERTS)
#include <cstdio>
#include <cstdlib>

namespace tl::detail {
[[noreturn]] inline void assert_failed(const char* expr, const char* file, int line) noexcept
{
    std::fprintf(stderr, "telinha: assertion failed: %s at %s:%d\n", expr, file, line);
    std::abort();
}
}  // namespace tl::detail

#define TL_ASSERT(expr) ((expr) ? (void)0 : ::tl::detail::assert_failed(#expr, __FILE__, __LINE__))
#else
#define TL_ASSERT(expr) ((void)0)
#endif
