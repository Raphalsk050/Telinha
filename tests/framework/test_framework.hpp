#pragma once

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

namespace tl::test {

struct TestCase {
    const char* suite;
    const char* name;
    void (*run)();
};

class Registry {
public:
    static Registry& instance()
    {
        static Registry registry;
        return registry;
    }

    void add(const TestCase& test_case) { cases_.push_back(test_case); }

    [[nodiscard]] const std::vector<TestCase>& cases() const { return cases_; }

    void record_failure(const char* file, int line, const char* expression,
                        const std::string& detail)
    {
        ++failures_in_case_;
        std::fprintf(stderr, "    FAIL %s:%d\n      %s\n", file, line, expression);
        if (!detail.empty()) {
            std::fprintf(stderr, "      %s\n", detail.c_str());
        }
    }

    void begin_case() { failures_in_case_ = 0; }
    [[nodiscard]] int failures_in_case() const { return failures_in_case_; }

private:
    std::vector<TestCase> cases_;
    int failures_in_case_ = 0;
};

struct Registrar {
    Registrar(const char* suite, const char* name, void (*run)())
    {
        Registry::instance().add(TestCase{suite, name, run});
    }
};

struct AbortCase {};

int run_all(int argc, char** argv);

}  // namespace tl::test

#define TL_TEST_CONCAT_INNER(a, b) a##b
#define TL_TEST_CONCAT(a, b) TL_TEST_CONCAT_INNER(a, b)

#define TEST_CASE(suite_name, case_name)                                       \
    static void TL_TEST_CONCAT(tl_test_fn_, __LINE__)();                       \
    static const ::tl::test::Registrar TL_TEST_CONCAT(tl_test_reg_, __LINE__){ \
        suite_name, case_name, &TL_TEST_CONCAT(tl_test_fn_, __LINE__)};        \
    static void TL_TEST_CONCAT(tl_test_fn_, __LINE__)()

#define CHECK(expr)                                                                    \
    do {                                                                               \
        if (!(expr)) {                                                                 \
            ::tl::test::Registry::instance().record_failure(__FILE__, __LINE__, #expr, \
                                                            std::string());            \
        }                                                                              \
    } while (false)

#define REQUIRE(expr)                                                                  \
    do {                                                                               \
        if (!(expr)) {                                                                 \
            ::tl::test::Registry::instance().record_failure(__FILE__, __LINE__, #expr, \
                                                            std::string());            \
            throw ::tl::test::AbortCase{};                                             \
        }                                                                              \
    } while (false)

#define CHECK_EQ(lhs, rhs)                                                                 \
    do {                                                                                   \
        const auto tl_lhs_ = (lhs);                                                        \
        const auto tl_rhs_ = (rhs);                                                        \
        if (!(tl_lhs_ == tl_rhs_)) {                                                       \
            ::tl::test::Registry::instance().record_failure(                               \
                __FILE__, __LINE__, #lhs " == " #rhs,                                      \
                "left " + std::to_string(tl_lhs_) + ", right " + std::to_string(tl_rhs_)); \
        }                                                                                  \
    } while (false)

#define REQUIRE_EQ(lhs, rhs)                                                               \
    do {                                                                                   \
        const auto tl_lhs_ = (lhs);                                                        \
        const auto tl_rhs_ = (rhs);                                                        \
        if (!(tl_lhs_ == tl_rhs_)) {                                                       \
            ::tl::test::Registry::instance().record_failure(                               \
                __FILE__, __LINE__, #lhs " == " #rhs,                                      \
                "left " + std::to_string(tl_lhs_) + ", right " + std::to_string(tl_rhs_)); \
            throw ::tl::test::AbortCase{};                                                 \
        }                                                                                  \
    } while (false)
