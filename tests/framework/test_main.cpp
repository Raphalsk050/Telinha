#include "test_framework.hpp"

namespace tl::test {

int run_all(int argc, char** argv)
{
    const char* filter = nullptr;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--filter") == 0 && i + 1 < argc) {
            filter = argv[i + 1];
            ++i;
        }
    }

    Registry& registry = Registry::instance();
    int failed_cases = 0;
    int ran_cases = 0;

    for (const TestCase& test_case : registry.cases()) {
        if (filter != nullptr && std::strstr(test_case.suite, filter) == nullptr &&
            std::strstr(test_case.name, filter) == nullptr) {
            continue;
        }

        ++ran_cases;
        std::printf("  %s / %s\n", test_case.suite, test_case.name);
        registry.begin_case();
        try {
            test_case.run();
        } catch (const AbortCase&) {
        }
        if (registry.failures_in_case() != 0) {
            ++failed_cases;
        }
    }

    if (ran_cases == 0) {
        std::fprintf(stderr, "no test cases matched\n");
        return 2;
    }

    std::printf("%d case(s) ran, %d failed\n", ran_cases, failed_cases);
    return failed_cases == 0 ? 0 : 1;
}

}  // namespace tl::test

int main(int argc, char** argv)
{
    return tl::test::run_all(argc, argv);
}
