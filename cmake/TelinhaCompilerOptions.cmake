include(CheckIPOSupported)

add_library(telinha_compiler_options INTERFACE)
add_library(Telinha::CompilerOptions ALIAS telinha_compiler_options)

target_compile_features(telinha_compiler_options INTERFACE cxx_std_20)

if(MSVC)
    target_compile_options(telinha_compiler_options INTERFACE
        /W4
        /permissive-
        /utf-8
        /Zc:__cplusplus
        /Zc:preprocessor
        /Zc:inline
        /GR-
        /EHsc
        /volatile:iso
        $<$<CONFIG:Debug>:/Od>
        $<$<CONFIG:Debug>:/Zi>
        $<$<NOT:$<CONFIG:Debug>>:/O2>
        $<$<NOT:$<CONFIG:Debug>>:/Oi>
        $<$<NOT:$<CONFIG:Debug>>:/Zi>)
    target_link_options(telinha_compiler_options INTERFACE
        $<$<NOT:$<CONFIG:Debug>>:/DEBUG:FULL>
        $<$<NOT:$<CONFIG:Debug>>:/OPT:REF>
        $<$<NOT:$<CONFIG:Debug>>:/OPT:ICF>)
    target_compile_definitions(telinha_compiler_options INTERFACE
        NOMINMAX
        WIN32_LEAN_AND_MEAN
        UNICODE
        _UNICODE)
    if(TELINHA_WARNINGS_AS_ERRORS)
        target_compile_options(telinha_compiler_options INTERFACE /WX)
        target_link_options(telinha_compiler_options INTERFACE /WX)
    endif()
else()
    target_compile_options(telinha_compiler_options INTERFACE
        -Wall
        -Wextra
        -Wpedantic
        -Wshadow
        -Wcast-align
        -Wnon-virtual-dtor
        -Woverloaded-virtual
        -Wnull-dereference
        -Wno-unknown-pragmas
        -fno-rtti
        $<$<CONFIG:Debug>:-Og>
        $<$<CONFIG:Debug>:-g3>
        $<$<CONFIG:Debug>:-fno-omit-frame-pointer>)
    if(TELINHA_WARNINGS_AS_ERRORS)
        target_compile_options(telinha_compiler_options INTERFACE -Werror)
    endif()
endif()

target_compile_definitions(telinha_compiler_options INTERFACE
    $<$<CONFIG:Debug>:TELINHA_ENABLE_ASSERTS>)

if(NOT TELINHA_SANITIZER STREQUAL "none")
    if(MSVC)
        if(NOT TELINHA_SANITIZER STREQUAL "address")
            message(FATAL_ERROR "MSVC only supports TELINHA_SANITIZER=address")
        endif()
        target_compile_options(telinha_compiler_options INTERFACE /fsanitize=address)
    else()
        target_compile_options(telinha_compiler_options INTERFACE
            -fsanitize=${TELINHA_SANITIZER} -fno-omit-frame-pointer -g)
        target_link_options(telinha_compiler_options INTERFACE
            -fsanitize=${TELINHA_SANITIZER})
    endif()
endif()

set(TELINHA_IPO_AVAILABLE OFF)
if(TELINHA_ENABLE_LTO AND TELINHA_SANITIZER STREQUAL "none")
    check_ipo_supported(RESULT TELINHA_IPO_AVAILABLE OUTPUT TELINHA_IPO_MESSAGE)
    if(NOT TELINHA_IPO_AVAILABLE)
        message(STATUS "Link time optimization unavailable: ${TELINHA_IPO_MESSAGE}")
    endif()
endif()

function(telinha_apply_target_defaults target)
    target_link_libraries(${target} PRIVATE Telinha::CompilerOptions)
    set_target_properties(${target} PROPERTIES
        CXX_STANDARD 20
        CXX_STANDARD_REQUIRED ON
        CXX_EXTENSIONS OFF
        POSITION_INDEPENDENT_CODE ON)
    if(TELINHA_IPO_AVAILABLE)
        set_target_properties(${target} PROPERTIES
            INTERPROCEDURAL_OPTIMIZATION_RELEASE ON
            INTERPROCEDURAL_OPTIMIZATION_RELWITHDEBINFO ON)
    endif()
endfunction()
