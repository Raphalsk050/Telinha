cmake_minimum_required(VERSION 3.24)

project(Telinha
    VERSION 0.1.0
    DESCRIPTION "Peer to peer screen sharing for Windows"
    LANGUAGES CXX)

if(PROJECT_IS_TOP_LEVEL)
    set(CMAKE_CXX_STANDARD 20)
    set(CMAKE_CXX_STANDARD_REQUIRED ON)
    set(CMAKE_CXX_EXTENSIONS OFF)
    set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
    set_property(GLOBAL PROPERTY USE_FOLDERS ON)

    if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
        set(CMAKE_BUILD_TYPE Release CACHE STRING "" FORCE)
    endif()
endif()

option(TELINHA_BUILD_TESTS "Build the test executables" ON)
option(TELINHA_BUILD_BENCHMARKS "Build the benchmark executables" ON)
option(TELINHA_BUILD_TOOLS "Build the command line tools" ON)
option(TELINHA_WARNINGS_AS_ERRORS "Treat compiler warnings as errors" ON)
option(TELINHA_ENABLE_LTO "Enable link time optimization in optimized builds" ON)
option(TELINHA_ENABLE_WGC "Build the Windows Graphics Capture backend" ON)
set(TELINHA_SANITIZER "none" CACHE STRING "One of none, address, undefined, thread")
set_property(CACHE TELINHA_SANITIZER PROPERTY STRINGS none address undefined thread)

include(cmake/TelinhaCompilerOptions.cmake)

add_subdirectory(src/core)
add_subdirectory(src/capture)

if(TELINHA_BUILD_TESTS)
    enable_testing()
    add_subdirectory(tests)
endif()

if(TELINHA_BUILD_BENCHMARKS)
    add_subdirectory(bench)
endif()

if(TELINHA_BUILD_TOOLS)
    add_subdirectory(tools)
endif()
