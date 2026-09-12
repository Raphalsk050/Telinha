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
option(TELINHA_ENABLE_WEBRTC "Build the libwebrtc transport backend" ${WIN32})
set(TELINHA_SANITIZER "none" CACHE STRING "One of none, address, undefined, thread")
set_property(CACHE TELINHA_SANITIZER PROPERTY STRINGS none address undefined thread)

if(MSVC AND TELINHA_ENABLE_WEBRTC)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded" CACHE STRING
        "The pinned libwebrtc links the static release CRT, so the whole tree must match" FORCE)
endif()

include(cmake/TelinhaCompilerOptions.cmake)

set(TELINHA_CORE_MODULES core capture transport)
set(TELINHA_OPTIONAL_MODULES audio encode receive diag app)

file(GLOB TELINHA_MODULE_FILES "${PROJECT_SOURCE_DIR}/src/*/CMakeLists.txt")
foreach(module_file ${TELINHA_MODULE_FILES})
    get_filename_component(module_dir "${module_file}" DIRECTORY)
    get_filename_component(module "${module_dir}" NAME)
    if(NOT module IN_LIST TELINHA_CORE_MODULES AND NOT module IN_LIST TELINHA_OPTIONAL_MODULES)
        message(FATAL_ERROR
            "src/${module} has a CMakeLists.txt but is in neither TELINHA_CORE_MODULES nor "
            "TELINHA_OPTIONAL_MODULES, so nothing would build it and no CI job would cover it. "
            "Add ${module} to TELINHA_OPTIONAL_MODULES in the top level CMakeLists.txt.")
    endif()
endforeach()

foreach(module ${TELINHA_CORE_MODULES})
    add_subdirectory(src/${module})
endforeach()

foreach(module ${TELINHA_OPTIONAL_MODULES})
    if(EXISTS "${PROJECT_SOURCE_DIR}/src/${module}/CMakeLists.txt")
        add_subdirectory(src/${module})
    endif()
endforeach()

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
