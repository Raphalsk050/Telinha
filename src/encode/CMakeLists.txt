set(TELINHA_ENCODE_SOURCES
    encode_strings.cpp
    backends/backend_registry.cpp)

add_library(telinha_encode STATIC ${TELINHA_ENCODE_SOURCES})
add_library(Telinha::Encode ALIAS telinha_encode)

target_include_directories(telinha_encode PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)

target_link_libraries(telinha_encode PUBLIC Telinha::Core Telinha::Capture Telinha::Audio)

if(WIN32)
    target_link_libraries(telinha_encode PRIVATE mfplat mfuuid mfreadwrite)
endif()

telinha_apply_target_defaults(telinha_encode)
