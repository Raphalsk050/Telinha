set(TELINHA_AUDIO_SOURCES
    audio_strings.cpp
    backends/backend_registry.cpp)

add_library(telinha_audio STATIC ${TELINHA_AUDIO_SOURCES})
add_library(Telinha::Audio ALIAS telinha_audio)

target_include_directories(telinha_audio PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)

target_link_libraries(telinha_audio PUBLIC Telinha::Core)

if(WIN32)
    target_link_libraries(telinha_audio PRIVATE ole32 mmdevapi)
endif()

telinha_apply_target_defaults(telinha_audio)
