
set(TELINHA_TRANSPORT_SOURCES
    transport_strings.cpp
    signaling/session_blob.cpp
    backends/backend_registry.cpp)

if(TELINHA_ENABLE_WEBRTC)
    include(${CMAKE_CURRENT_SOURCE_DIR}/cmake/TelinhaWebRTC.cmake)
    telinha_acquire_webrtc()

    list(APPEND TELINHA_TRANSPORT_SOURCES
        backends/webrtc/encoded_video_buffer.cpp
        backends/webrtc/passthrough_video_decoder.cpp
        backends/webrtc/passthrough_video_encoder.cpp
        backends/webrtc/push_audio_device.cpp
        backends/webrtc/webrtc_media_transport.cpp)
endif()

add_library(telinha_transport STATIC ${TELINHA_TRANSPORT_SOURCES})
add_library(Telinha::Transport ALIAS telinha_transport)

target_include_directories(telinha_transport PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)
target_include_directories(telinha_transport PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(telinha_transport PUBLIC Telinha::Core)

if(TELINHA_ENABLE_WEBRTC)
    if(MSVC AND NOT CMAKE_MSVC_RUNTIME_LIBRARY STREQUAL "MultiThreaded")
        message(FATAL_ERROR
            "The pinned libwebrtc is built against the static release CRT. Set "
            "CMAKE_MSVC_RUNTIME_LIBRARY to MultiThreaded in the top level CMakeLists before the "
            "first add_subdirectory, or turn TELINHA_ENABLE_WEBRTC off.")
    endif()
    target_compile_definitions(telinha_transport PRIVATE TELINHA_HAS_TRANSPORT_BACKENDS=1)
    target_link_libraries(telinha_transport PRIVATE Telinha::WebRTC)
endif()

telinha_apply_target_defaults(telinha_transport)
