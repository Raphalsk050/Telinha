add_library(telinha_test_framework STATIC framework/test_main.cpp)
add_library(Telinha::TestFramework ALIAS telinha_test_framework)
target_include_directories(telinha_test_framework PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/framework>)
telinha_apply_target_defaults(telinha_test_framework)

function(telinha_add_test target)
    add_executable(${target} ${ARGN})
    target_link_libraries(${target} PRIVATE
        Telinha::TestFramework
        Telinha::Core
        Telinha::Capture
        Telinha::Audio
        Telinha::Encode
        Telinha::Transport)
    telinha_apply_target_defaults(${target})
    add_test(NAME ${target} COMMAND ${target})
    set_tests_properties(${target} PROPERTIES TIMEOUT 300)
endfunction()

add_subdirectory(foundation)

foreach(suite core capture)
    if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/${suite}/CMakeLists.txt")
        add_subdirectory(${suite})
    endif()
endforeach()
