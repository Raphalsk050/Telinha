# Telinha release notes
## Features

1. A Glass theme (Settings > Appearance > Vidro): translucent panels that blur what is behind them over a soft lit background, with sliders for frost, darkness, blur and the background lights. The message box floats over the conversation. Glass is the theme of new installs.
2. Sounds for muting and unmuting the microphone and for deafening and undeafening, each with its own tune, and a sound when someone sends a message in any conversation.

## Improvements

1. All calls and file links of the app share one WebRTC engine instead of starting a new one each time.
2. The sounds of someone joining or leaving a room are louder.
3. Voice is sent at 96 kbit/s instead of 32, so soundboard sounds no longer come out watery, as if noise suppression had gone over them.
4. Menus and the soundboard are glass on the Glass theme.

## Fixes

1. After someone left a voice room and came back, the people who stayed connected but no longer heard them. When the last call of the app ended, the part that delivers incoming voice stopped for good until the app was restarted.
2. The mouse wheel over a long draft scrolled the conversation behind the message box instead of the draft.
3. The message box stayed tall after its text was deleted.
4. The gear of a voice channel covered the time of the call.
5. Two divider lines between the voice panel and the user panel on the Glass theme.
