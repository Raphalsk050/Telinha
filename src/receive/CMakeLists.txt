set(TELINHA_RECEIVE_SOURCES
    clock_offset.cpp
    jitter_buffer.cpp
    receive_strings.cpp
    backends/backend_registry.cpp
    backends/audio_renderer_headless.cpp
    backends/video_renderer_headless.cpp)

if(WIN32)
    list(APPEND TELINHA_RECEIVE_SOURCES
        backends/audio_renderer_wasapi.cpp
        backends/video_decoder_media_foundation.cpp
        backends/video_renderer_d3d11.cpp)
endif()

add_library(telinha_receive STATIC ${TELINHA_RECEIVE_SOURCES})
add_library(Telinha::Receive ALIAS telinha_receive)

target_include_directories(telinha_receive PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)

target_link_libraries(telinha_receive PUBLIC Telinha::Core Telinha::Capture Telinha::Transport)

if(WIN32)
    target_link_libraries(telinha_receive PRIVATE
        d3d11 dxgi dxguid d3dcompiler mf mfplat mfuuid mfreadwrite ole32 user32 avrt)
endif()

telinha_apply_target_defaults(telinha_receive)
