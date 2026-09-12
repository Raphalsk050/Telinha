include(FetchContent)

set(TELINHA_WEBRTC_VERSION "m154.8037.1.1" CACHE STRING
    "Pinned shiguredo-webrtc-build release tag providing libwebrtc")
set(TELINHA_WEBRTC_ROOT "" CACHE PATH
    "Root of an existing libwebrtc package, with include/ and lib/ inside; skips the download")
set(TELINHA_WEBRTC_URL_BASE
    "https://github.com/shiguredo-webrtc-build/webrtc-build/releases/download"
    CACHE STRING "Base URL the pinned libwebrtc archive is fetched from")
set(TELINHA_WEBRTC_ARCHIVE "webrtc.windows_x86_64.zip" CACHE STRING
    "Archive name inside the pinned release")
set(TELINHA_WEBRTC_SHA256
    "7ed43bad348830f8d70e8d2be414274aa5bd07c0838cb8086162990c8c8671c9"
    CACHE STRING "SHA256 of TELINHA_WEBRTC_ARCHIVE")

function(telinha_acquire_webrtc)
    if(TARGET Telinha::WebRTC)
        return()
    endif()

    if(NOT WIN32 OR NOT CMAKE_SIZEOF_VOID_P EQUAL 8)
        message(FATAL_ERROR
            "TELINHA_ENABLE_WEBRTC is supported on Windows x64 only. The published libwebrtc "
            "builds for other platforms link Chromium's own libc++ under the __Cr ABI "
            "namespace, which cannot be mixed with the libstdc++ the rest of the tree uses. "
            "Turn the option off, or point TELINHA_WEBRTC_ROOT at a package built against the "
            "same standard library as this build.")
    endif()

    if(TELINHA_WEBRTC_ROOT)
        set(package_root "${TELINHA_WEBRTC_ROOT}")
    else()
        FetchContent_Declare(telinha_libwebrtc
            URL "${TELINHA_WEBRTC_URL_BASE}/${TELINHA_WEBRTC_VERSION}/${TELINHA_WEBRTC_ARCHIVE}"
            URL_HASH "SHA256=${TELINHA_WEBRTC_SHA256}"
            DOWNLOAD_EXTRACT_TIMESTAMP ON)
        FetchContent_MakeAvailable(telinha_libwebrtc)
        set(package_root "${telinha_libwebrtc_SOURCE_DIR}")
    endif()

    set(include_root "${package_root}/include")
    set(library_path "${package_root}/lib/webrtc.lib")

    if(NOT EXISTS "${include_root}")
        message(FATAL_ERROR "libwebrtc package has no include directory at ${include_root}")
    endif()
    if(NOT EXISTS "${library_path}")
        message(FATAL_ERROR "libwebrtc package has no static library at ${library_path}")
    endif()

    add_library(telinha_webrtc STATIC IMPORTED GLOBAL)
    add_library(Telinha::WebRTC ALIAS telinha_webrtc)
    set_target_properties(telinha_webrtc PROPERTIES IMPORTED_LOCATION "${library_path}")

    target_include_directories(telinha_webrtc SYSTEM INTERFACE
        "${include_root}"
        "${include_root}/third_party/abseil-cpp"
        "${include_root}/third_party/boringssl/src/include"
        "${include_root}/third_party/libyuv/include")

    target_compile_definitions(telinha_webrtc INTERFACE
        NDEBUG
        WEBRTC_WIN=1
        NOMINMAX
        WIN32_LEAN_AND_MEAN
        _HAS_ITERATOR_DEBUGGING=0
        _ITERATOR_DEBUG_LEVEL=0)

    target_compile_options(telinha_webrtc INTERFACE
        /external:W0
        /wd4100 /wd4127 /wd4244 /wd4245 /wd4267 /wd4324 /wd4389 /wd4456 /wd4459 /wd4701)

    find_package(Threads REQUIRED)
    target_link_libraries(telinha_webrtc INTERFACE
        Threads::Threads
        winmm ws2_32 secur32 iphlpapi crypt32 bcrypt advapi32 shell32 shlwapi userenv version
        ole32 oleaut32 uuid strmiids msdmo dmoguids wmcodecdspuuid dxgi d3d11 dwrite dxguid)
endfunction()
