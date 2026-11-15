# Telinha release notes
## Features

1. A server can now have a relay for calls and streams. When two people cannot connect directly, which happened between some home networks and took over a minute of retries, the call goes through the relay instead. Set it in the server menu, under Relay das chamadas; it reaches every member of the server.
2. An opened image can be zoomed. The mouse wheel zooms around the cursor, a click goes to the real size and back, and there are buttons at the bottom; drag the picture to move around it. The keys +, - and 0 work too.
3. A GIF larger than 380 KB stays animated in the chat. It used to be turned into a still picture; it now travels as a file and both sides see it moving.

## Improvements

1. Telinha no longer runs on Electron. It is now a native program, without a built-in browser inside it. It keeps the same data folder, so profile, contacts, servers and chats carry over, and it keeps updating itself as before.
2. An opened image blurs what is behind it instead of only darkening it.

## Fixes

1. Some capture cards were shared in 4:3 with a squashed picture. The app chose a 4:3 mode the card lists at 50 fps over its 16:9 modes; it now picks a 16:9 mode first.
2. Capture cards now ask for full range color explicitly, and YUV that the card sends without a range is read as studio range, which keeps the picture from looking washed out when the card does not say which range it uses.
3. Closing the program being shared crashed the stream. Windows ended the process when the capture of a window that no longer existed was shut down; the stream now stops by itself and says the window was closed.
4. Stopping a screen share could crash the stream process on the way out, because it read memory it had already released. The same happened when a fullscreen game being shared was closed.
5. Sharing a screen where nothing was moving showed a black picture, or the previous screen after switching, until something changed on it. It was easiest to see on a second monitor. The last picture is now sent again to whoever starts watching.
6. The fullscreen button on the preview of your own shared screen did nothing.
7. An opened image taller than the window was cut off at the bottom. It now always fits in the window.
8. A crash of the stream process that ended it without an exception left nothing in the log; it now writes where it happened.
