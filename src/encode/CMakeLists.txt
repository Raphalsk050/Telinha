set(TELINHA_ENCODE_SOURCES
    encode_strings.cpp
    region_of_interest.cpp
    backends/backend_registry.cpp)

if(WIN32)
    list(APPEND TELINHA_ENCODE_SOURCES
        backends/d3d_frame_converter.cpp
        backends/nvenc_library.cpp
        backends/nvenc_encoder.cpp
        backends/mf_platform.cpp
        backends/mf_video_encoder.cpp)
endif()

add_library(telinha_encode STATIC ${TELINHA_ENCODE_SOURCES})
add_library(Telinha::Encode ALIAS telinha_encode)

target_include_directories(telinha_encode PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)

target_include_directories(telinha_encode PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/backends)

target_link_libraries(telinha_encode PUBLIC
    Telinha::Core
    Telinha::Capture
    Telinha::Transport)

if(WIN32)
    target_include_directories(telinha_encode PRIVATE
        ${PROJECT_SOURCE_DIR}/third_party/nv-codec-headers)
    target_compile_definitions(telinha_encode PRIVATE TELINHA_HAS_ENCODE_BACKENDS=1)
    target_link_libraries(telinha_encode PRIVATE
        d3d11 dxgi dxguid mfplat mfuuid strmiids ole32 oleaut32)
endif()

telinha_apply_target_defaults(telinha_encode)
