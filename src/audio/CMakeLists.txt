set(TELINHA_AUDIO_SOURCES
    audio_capture_stream.cpp
    audio_converter.cpp
    audio_strings.cpp
    backends/backend_registry.cpp)

if(WIN32)
    list(APPEND TELINHA_AUDIO_SOURCES
        backends/wasapi_audio_source.cpp
        backends/wasapi_support.cpp)
endif()

add_library(telinha_audio STATIC ${TELINHA_AUDIO_SOURCES})
add_library(Telinha::Audio ALIAS telinha_audio)

target_include_directories(telinha_audio PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)
target_include_directories(telinha_audio PRIVATE
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}>)

target_link_libraries(telinha_audio PUBLIC Telinha::Core)

if(WIN32)
    target_compile_definitions(telinha_audio PRIVATE TELINHA_HAS_AUDIO_BACKENDS=1)
    target_link_libraries(telinha_audio PRIVATE ole32 mmdevapi avrt)
endif()

telinha_apply_target_defaults(telinha_audio)
