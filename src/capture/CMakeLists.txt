set(TELINHA_CAPTURE_SOURCES
    capture_pipeline.cpp
    capture_stats.cpp
    capture_strings.cpp
    backends/backend_registry.cpp
    backends/cursor_shape.cpp
    backends/dirty_region_builder.cpp
    backends/synthetic_source.cpp
    backends/texture_ring_policy.cpp)

if(WIN32)
    list(APPEND TELINHA_CAPTURE_SOURCES
        backends/win/d3d11_capture_device.cpp
        backends/win/desktop_duplication_source.cpp
        backends/win/graphics_capture_source.cpp
        backends/win/target_enumeration.cpp
        backends/win/win_capture_common.cpp)
endif()

add_library(telinha_capture STATIC ${TELINHA_CAPTURE_SOURCES})
add_library(Telinha::Capture ALIAS telinha_capture)

target_include_directories(telinha_capture PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)

target_link_libraries(telinha_capture PUBLIC Telinha::Core)

target_compile_definitions(telinha_capture PRIVATE TELINHA_HAS_PLATFORM_BACKENDS=1)

if(WIN32)
    target_link_libraries(telinha_capture PRIVATE d3d11 dxgi dxguid user32 shcore dwmapi)
    if(TELINHA_ENABLE_WGC)
        target_compile_definitions(telinha_capture PRIVATE TELINHA_ENABLE_WGC=1)
        target_link_libraries(telinha_capture PRIVATE windowsapp)
    endif()
endif()

telinha_apply_target_defaults(telinha_capture)
