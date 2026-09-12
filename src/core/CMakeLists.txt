add_library(telinha_core STATIC
    clock.cpp
    latency_histogram.cpp
    log.cpp
    result.cpp)

add_library(Telinha::Core ALIAS telinha_core)

target_include_directories(telinha_core PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)

target_link_libraries(telinha_core PUBLIC Telinha::CompilerOptions)

find_package(Threads REQUIRED)
target_link_libraries(telinha_core PUBLIC Threads::Threads)

telinha_apply_target_defaults(telinha_core)
