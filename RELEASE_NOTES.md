# Telinha release notes
## Features

None.

## Improvements

1. Updates install themselves: the app downloads the new version, swaps Telinha.exe where it is and reopens as soon as no call or share is running.
2. New versions are looked for every minute, and the download shows its progress.
3. When the share program crashes it now leaves a log line and a dump file to investigate.

## Fixes

1. A share reaches the frame rate it was asked for. Asking for 120 fps used to give about 64, and 60 fps about 41.
2. Fixed a crash that could stop a share when the screen changes mode, as when alt-tabbing out of a fullscreen game.
3. Watching a share that is not 16:9 in fullscreen shows the whole picture instead of zooming into it.

Notes:
Versions 0.1.0 and 0.2.0 cannot replace Telinha.exe by themselves, so download this release by hand once. With Telinha.exe in a protected folder such as C:\, Windows asks for permission on every update; in a regular folder, like the Desktop, nothing is asked.
