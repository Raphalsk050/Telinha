set(TELINHA_DIAG_SOURCES
    diagnostics.cpp
    module_probes.cpp
    report_builder.cpp
    stun_probe.cpp)

if(WIN32)
    list(APPEND TELINHA_DIAG_SOURCES platform_probes_win.cpp)
else()
    list(APPEND TELINHA_DIAG_SOURCES platform_probes_stub.cpp)
endif()

add_library(telinha_diag STATIC ${TELINHA_DIAG_SOURCES})
add_library(Telinha::Diag ALIAS telinha_diag)

target_include_directories(telinha_diag PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)

target_include_directories(telinha_diag PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR})

target_link_libraries(telinha_diag PUBLIC
    Telinha::Core
    Telinha::Capture
    Telinha::Encode)

if(WIN32)
    target_link_libraries(telinha_diag PRIVATE dxgi dxguid ole32 mmdevapi ws2_32)
endif()

telinha_apply_target_defaults(telinha_diag)
