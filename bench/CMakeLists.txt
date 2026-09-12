add_library(telinha_bench_framework INTERFACE)
add_library(Telinha::BenchFramework ALIAS telinha_bench_framework)
target_include_directories(telinha_bench_framework INTERFACE
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/framework>)

function(telinha_add_benchmark target)
    add_executable(${target} ${ARGN})
    target_link_libraries(${target} PRIVATE
        Telinha::BenchFramework Telinha::Core Telinha::Capture)
    telinha_apply_target_defaults(${target})
endfunction()

foreach(suite core capture)
    if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/${suite}/CMakeLists.txt")
        add_subdirectory(${suite})
    endif()
endforeach()
