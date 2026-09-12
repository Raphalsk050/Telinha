set(TELINHA_APP_SOURCES
    app_options.cpp
    receiver_session.cpp
    signaling.cpp)

set(TELINHA_APP_HAS_SENDER 0)
if(TARGET Telinha::Encode AND TARGET Telinha::Audio)
    set(TELINHA_APP_HAS_SENDER 1)
    list(APPEND TELINHA_APP_SOURCES sender_session.cpp)
endif()

add_library(telinha_app STATIC ${TELINHA_APP_SOURCES})
add_library(Telinha::App ALIAS telinha_app)

target_include_directories(telinha_app PUBLIC
    $<BUILD_INTERFACE:${PROJECT_SOURCE_DIR}/include>)

target_compile_definitions(telinha_app PUBLIC
    TELINHA_APP_HAS_SENDER=${TELINHA_APP_HAS_SENDER})

target_link_libraries(telinha_app PUBLIC
    Telinha::Core
    Telinha::Capture
    Telinha::Receive
    Telinha::Transport)

if(TELINHA_APP_HAS_SENDER)
    target_link_libraries(telinha_app PUBLIC Telinha::Encode Telinha::Audio)
endif()

telinha_apply_target_defaults(telinha_app)
