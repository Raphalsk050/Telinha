# nv-codec-headers

`nvEncodeAPI.h` is taken unmodified from the FFmpeg nv-codec-headers project
(https://github.com/FFmpeg/nv-codec-headers), which redistributes the NVIDIA
Video Codec SDK headers under the permissive notice reproduced in `LICENSE`.

The header only declares the NVENC ABI. Nothing links against an NVIDIA import
library: `src/encode/backends/nvenc_encoder.cpp` loads `nvEncodeAPI64.dll` at
runtime and resolves `NvEncodeAPICreateInstance`, so a build without an NVIDIA
driver present still produces a working binary that reports NVENC as
unavailable.

API version vendored: 13.1.
