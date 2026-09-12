# Telinha

Peer to peer screen sharing for Windows, over the internet.

Telinha transmits a screen or a single application window, together with the
audio produced by that same source. It is a one way transmission tool, not a
communication tool.

## Build

```
cmake --preset linux-release && cmake --build --preset linux-release && ctest --preset linux-release
cmake --preset msvc-release  && cmake --build --preset msvc-release  && ctest --preset msvc-release
```

`cmake --list-presets` shows the rest, including the address and thread
sanitizer presets.
