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

## Desktop app

`desktop/` holds an Electron app that drives `telinha.exe` through its `--json`
mode, so sharing and watching need no terminal. It looks for the executable in
`TELINHA_EXE`, then next to the packaged app, then in
`build/msvc-release/tools/telinha/Release`.

```
npm --prefix desktop install
npm --prefix desktop start
npm --prefix desktop test
npm --prefix desktop run e2e
npm --prefix desktop run dist
```

`e2e` runs a sender and a receiver on the same machine and exchanges the codes
between them.

The first connection with someone uses the copy-and-paste codes, and after that
the person is saved as a contact. Contacts and servers each get an encrypted
channel over public MQTT brokers that carries presence, chat, call signaling and
stream codes, while chat history stays on each computer. Voice and webcam run on
the WebRTC stack inside Electron as a mesh between everyone in the call. Each
screen share is one `telinha.exe send --multi` process that fans the same
encoded stream out to every viewer, and each viewer runs its own
`telinha.exe recv`.

Screen streams bind their local ports inside 50000-50019 and calls inside
50020-50039. When two networks cannot reach each other directly, forwarding UDP
50000-50039 on the router is usually enough, unless that connection sits behind
carrier grade NAT. `dist` writes a portable `Telinha-<version>-portable.exe` with
`telinha.exe` bundled inside to `desktop/dist`.
