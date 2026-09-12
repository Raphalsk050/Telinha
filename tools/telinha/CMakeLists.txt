add_executable(telinha main.cpp)

target_link_libraries(telinha PRIVATE Telinha::App)

set_target_properties(telinha PROPERTIES OUTPUT_NAME telinha)

if(WIN32)
    target_link_libraries(telinha PRIVATE winmm)
endif()

telinha_apply_target_defaults(telinha)

install(TARGETS telinha RUNTIME DESTINATION .)
