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
carrier grade NAT. `dist` writes a portable `Telinha.exe` with
`telinha.exe` bundled inside to `desktop/dist`.

On Windows, `.\build.ps1` builds `telinha.exe` on every core and runs `dist` in
one go. `-Run` starts the app from the build instead of packaging it, and
`-ExeOnly` stops after `telinha.exe`.

CI builds the same portable app on every push to `main`, and on demand from
the Actions tab with "Run workflow". It is attached to the run as the
`telinha-app-windows-x64` artifact.

The app version lives in `version.json` and is only ever changed by hand.
`RELEASE_NOTES.md` lists what changed since the last release, in English, under
the fixed Features, Improvements and Fixes headings, with `None.` under an empty
one and an optional `Notes:` block at the end. A push to `main` with a version
above the last release, and every CI job green, tags it and publishes a GitHub
release with `Telinha.exe` and those notes. New notes under an unchanged
version just wait.

The portable app checks the latest release on start and every two hours,
downloads a newer one in the background, swaps it in place of the file it was
opened from, and asks to restart.
