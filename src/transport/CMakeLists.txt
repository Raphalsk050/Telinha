set(TELINHA_TRANSPORT_SOURCES
    transport_strings.cpp
    backends/backend_registry.cpp)

add_library(telinha_transport STATIC ${TELINHA_TRANSPORT_SOURCES})
add_library(Telinha::Transport ALIAS telinha_transport)

target_include_directories(telinha_transport PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)

target_link_libraries(telinha_transport PUBLIC Telinha::Core)

telinha_apply_target_defaults(telinha_transport)
